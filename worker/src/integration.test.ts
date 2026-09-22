import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { deflateSync } from "node:zlib";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { afterEach, describe, expect, it } from "vitest";

interface Output {
  type: string;
  taskId?: string;
  runId?: string;
  startedAt?: number;
  state?: string;
  message?: string;
  event?: string;
  detail?: { toolCallId?: string; text?: string };
  requestId?: string;
  method?: string;
  title?: string;
  options?: string[];
  level?: string;
  mode?: string;
  phase?: string;
  plan?: string;
  questions?: Array<{ id: string; header: string; question: string; multiSelect?: boolean; options: Array<{ label: string; description: string }> }>;
  tasks?: Array<{ id: number; subject: string; status: string; activeForm?: string; blockedBy?: number[] }>;
  errors?: Array<{ path: string; error: string }>;
  snapshot?: {
    sessionFile?: string;
    messages: Array<{ id?: string; role?: string; timestamp?: number; blocks: Array<{ type: string; text?: string; toolName?: string; details?: unknown; imageId?: string; thumbnail?: string; mimeType?: string }> }>;
    runTimings?: Array<{ userMessageId: string; durationMs: number }>;
    activeRun?: { runId: string; startedAt: number };
    tools?: Array<{ name: string; description: string; source: { kind: string; packageId?: string }; available: boolean; unavailableReason?: string }>;
    activeTools?: string[];
    planState?: { mode: string; phase: string; plan?: string };
    todoState?: { tasks: Array<{ id: number; subject: string; status: string }> };
  };
}

class WorkerHarness {
  readonly child: ChildProcessWithoutNullStreams;
  readonly outputs: Output[] = [];
  stderr = "";
  private waiters: Array<() => void> = [];

  constructor(cwd: string) {
    this.child = spawn(process.env.WACKCODE_TEST_NODE ?? process.execPath, [process.env.WACKCODE_TEST_WORKER ?? resolve("dist/index.js")], {
      cwd,
      env: { ...process.env, PI_OFFLINE: "1", PI_TELEMETRY: "0", PI_SKIP_VERSION_CHECK: "1" },
      stdio: ["pipe", "pipe", "pipe"]
    });
    createInterface({ input: this.child.stdout }).on("line", (line) => {
      this.outputs.push(JSON.parse(line) as Output);
      for (const notify of this.waiters.splice(0)) notify();
    });
    this.child.stderr.on("data", (chunk: Buffer) => { this.stderr += chunk.toString("utf8"); });
  }

  send(value: unknown): void {
    this.child.stdin.write(`${JSON.stringify(value)}\n`);
  }

  async waitFor(predicate: (output: Output) => boolean, timeoutMs = 12_000): Promise<Output> {
    const started = Date.now();
    while (true) {
      const match = this.outputs.find(predicate);
      if (match) return match;
      if (Date.now() - started >= timeoutMs) {
        throw new Error(`Timed out waiting for worker output. stderr: ${this.stderr}\noutputs: ${JSON.stringify(this.outputs)}`);
      }
      await new Promise<void>((resolvePromise) => {
        const timer = setTimeout(resolvePromise, 50);
        this.waiters.push(() => { clearTimeout(timer); resolvePromise(); });
      });
    }
  }

  async shutdown(): Promise<void> {
    if (this.child.exitCode !== null) return;
    this.send({ id: crypto.randomUUID(), type: "shutdown" });
    await Promise.race([
      new Promise<void>((resolvePromise) => this.child.once("exit", () => resolvePromise())),
      new Promise<void>((resolvePromise) => setTimeout(resolvePromise, 1_500))
    ]);
    if (this.child.exitCode === null) this.child.kill("SIGKILL");
  }
}

interface MockProvider {
  baseUrl: string;
  requests: Array<{ authorization: string; body: Record<string, unknown> }>;
  close: () => Promise<void>;
  waitForSlowRequest: () => Promise<void>;
}

async function startMockProvider(): Promise<MockProvider> {
  const requests: MockProvider["requests"] = [];
  let slowRequestResolve: (() => void) | undefined;
  const slowRequest = new Promise<void>((resolvePromise) => { slowRequestResolve = resolvePromise; });
  const server = createServer(async (request, response) => {
    const chunks: Buffer[] = [];
    for await (const chunk of request) chunks.push(Buffer.from(chunk));
    const body = JSON.parse(Buffer.concat(chunks).toString("utf8")) as Record<string, unknown>;
    const authorization = String(request.headers.authorization ?? "");
    requests.push({ authorization, body });
    if (authorization === "Bearer cancel-secret") {
      response.writeHead(200, { "content-type": "text/event-stream", connection: "keep-alive" });
      response.write(": waiting\n\n");
      slowRequestResolve?.();
      return;
    }
    streamAgentResponse(response, authorization, body);
  });
  await new Promise<void>((resolvePromise, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject);
      resolvePromise();
    });
  });
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Mock provider did not bind a TCP port");
  return {
    baseUrl: `http://127.0.0.1:${address.port}/v1`,
    requests,
    waitForSlowRequest: () => slowRequest,
    close: () => new Promise<void>((resolvePromise, reject) => {
      server.closeAllConnections();
      server.close((error) => error ? reject(error) : resolvePromise());
    })
  };
}

function streamAgentResponse(response: ServerResponse<IncomingMessage>, authorization: string, body: Record<string, unknown>): void {
  response.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
  const messages = Array.isArray(body.messages) ? body.messages as Array<{ role?: string }> : [];
  const hasToolResult = messages.some((message) => message.role === "tool");
  if (authorization === "Bearer stream-secret") {
    const chunks = ["Streaming ", "the answer ", "in pieces."];
    let index = 0;
    const timer = setInterval(() => {
      if (index < chunks.length) {
        response.write(`data: ${JSON.stringify({
          id: `stream-${index}`, object: "chat.completion.chunk", created: index, model: "shared-model",
          choices: [{ index: 0, delta: { role: "assistant", content: chunks[index] }, finish_reason: null }]
        })}\n\n`);
        index += 1;
        return;
      }
      clearInterval(timer);
      response.write(`data: ${JSON.stringify({
        id: "stream-done", object: "chat.completion.chunk", created: 9, model: "shared-model",
        choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
        usage: { prompt_tokens: 4, completion_tokens: 6, total_tokens: 10 }
      })}\n\n`);
      response.end("data: [DONE]\n\n");
    }, 60);
    return;
  }
  const suffix = authorization === "Bearer alpha-secret" ? "alpha" : "beta";
  const send = (value: unknown) => response.write(`data: ${JSON.stringify(value)}\n\n`);
  // The last user message steers which tool the fake model "decides" to call, so tests can
  // exercise specific tool paths through the real agent loop. Content arrives as an array
  // of parts, not a bare string.
  const lastUser = [...messages].reverse().find((message) => message.role === "user") as
    { content?: unknown } | undefined;
  const lastUserText = typeof lastUser?.content === "string"
    ? lastUser.content
    : Array.isArray(lastUser?.content)
      ? (lastUser.content as Array<{ type?: string; text?: string }>)
          .filter((part) => part.type === "text")
          .map((part) => part.text ?? "")
          .join("")
      : "";
  if (!hasToolResult) {
    const toolCall = (name: string, args: Record<string, unknown>) => ({
      index: 0, id: `call-${suffix}`, type: "function",
      function: { name, arguments: JSON.stringify(args) }
    });
    const calls = lastUserText.startsWith("ask:")
      ? [toolCall("ask_user_question", {
          questions: [{
            id: "approach", header: "Approach", question: "Which storage engine?",
            options: [
              { label: "SQLite", description: "Local file, zero ops." },
              { label: "Postgres", description: "Shared server." }
            ]
          }]
        })]
      : lastUserText.startsWith("finish plan")
        ? [toolCall("plan_mode_complete", { plan: "# The plan\n\n- Ship it" })]
      : lastUserText.startsWith("todo:")
        ? [toolCall("todo", { action: "create", subject: "Ship the thing" })]
        : lastUserText.startsWith("stream tool")
          ? [toolCall("bash", { command: "printf 'first line\\n'; sleep 0.3; printf 'second line\\n'" })]
          : [toolCall("write", { path: `${suffix}.txt`, content: `changed by ${suffix}\n` })];
    for (const [index, call] of calls.entries()) {
      send({
        id: `tool-${suffix}`,
        object: "chat.completion.chunk",
        created: 1,
        model: "shared-model",
        choices: [{ index, delta: { role: "assistant", tool_calls: [call] }, finish_reason: null }]
      });
    }
    send({
      id: `tool-${suffix}`,
      object: "chat.completion.chunk",
      created: 1,
      model: "shared-model",
      choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
      usage: { prompt_tokens: 20, completion_tokens: 8, total_tokens: 28 }
    });
  } else {
    send({
      id: `done-${suffix}`,
      object: "chat.completion.chunk",
      created: 2,
      model: "shared-model",
      choices: [{ index: 0, delta: { role: "assistant", content: `Finished ${suffix}.` }, finish_reason: null }]
    });
    send({
      id: `done-${suffix}`,
      object: "chat.completion.chunk",
      created: 2,
      model: "shared-model",
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      usage: { prompt_tokens: 28, completion_tokens: 4, total_tokens: 32 }
    });
  }
  response.end("data: [DONE]\n\n");
}

async function initializeWorker(
  baseUrl: string,
  apiKey: string,
  workspace: string,
  taskId: string,
  sessionFile?: string,
  disabledTools?: string[],
  resources?: { extensions: string[]; skills: string[]; prompts: string[]; themes: string[] },
  mode?: "build" | "plan",
  vision?: boolean
): Promise<{ worker: WorkerHarness; ready: Output }> {
  const worker = new WorkerHarness(workspace);
  worker.send({
    id: crypto.randomUUID(),
    type: "init",
    taskId,
    cwd: workspace,
    agentDir: join(workspace, ".agent", taskId),
    sessionDir: join(workspace, ".sessions", taskId),
    sessionFile,
    provider: {
      id: `provider-${taskId}`,
      name: `Provider ${taskId}`,
      kind: "custom",
      baseUrl,
      api: "openai-completions",
      models: [{
        id: "shared-model",
        name: "Shared model",
        contextWindow: 16_384,
        maxTokens: 321,
        reasoning: true,
        thinkingLevels: ["off", "high"],
        thinkingLevelMap: { off: null, high: "wire-high" },
        vision
      }]
    },
    modelId: "shared-model",
    apiKey,
    thinkingLevel: "high",
    disabledTools,
    resources,
    mode
  });
  return { worker, ready: await worker.waitFor((output) => output.type === "ready") };
}

/** A solid-colour RGB PNG, built by hand so the tests need no image fixtures. */
function solidPng(width: number, height: number): string {
  const crcTable = Array.from({ length: 256 }, (_, n) => {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  const crc32 = (bytes: Buffer) => {
    let c = 0xffffffff;
    for (const byte of bytes) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };
  const chunk = (type: string, data: Buffer) => {
    const length = Buffer.alloc(4);
    length.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(body));
    return Buffer.concat([length, body, crc]);
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 2, 0, 0, 0], 8);
  const row = Buffer.concat([Buffer.from([0]), Buffer.alloc(width * 3, 0x5a)]);
  const pixels = deflateSync(Buffer.concat(Array.from({ length: height }, () => row)));
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", pixels),
    chunk("IEND", Buffer.alloc(0))
  ]).toString("base64");
}

type RequestPart = { type?: string; text?: string; image_url?: { url?: string } };

function userParts(request: { body: Record<string, unknown> }): RequestPart[] {
  const messages = (request.body.messages ?? []) as Array<{ role?: string; content?: unknown }>;
  return messages
    .filter((message) => message.role === "user" && Array.isArray(message.content))
    .flatMap((message) => message.content as RequestPart[]);
}

const cleanup: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const action of cleanup.splice(0).reverse()) await action();
});

describe("Pi worker integration", () => {
  it("answers an extension dialog raised mid-prompt, and keeps a broken extension non-fatal", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const root = await mkdtemp(join(tmpdir(), "wackcode-dialog-"));
    cleanup.push(() => rm(root, { recursive: true, force: true }));

    // Asks the user a question from inside a tool call — i.e. while session.prompt() is running
    // and holding the worker's serial command queue.
    const asking = join(root, "asking.ts");
    await writeFile(asking,
      `export default function (pi: any) {
         pi.on("tool_call", async (_event: any, ctx: any) => {
           const answer = await ctx.ui.select("Pick a branch", ["main", "develop"]);
           ctx.ui.notify("picked:" + String(answer));
         });
       }\n`);
    // Throws at module scope: must be reported and skipped, not fatal.
    const broken = join(root, "broken.ts");
    await writeFile(broken, `throw new Error("deliberately broken extension");\n`);

    const { worker, ready } = await initializeWorker(
      provider.baseUrl, "alpha-secret", root, "dialog-task", undefined, undefined,
      { extensions: [asking, broken], skills: [], prompts: [], themes: [] }
    );
    cleanup.push(() => worker.shutdown());

    // The session came up despite the broken extension, and said why.
    expect(ready.type).toBe("ready");
    const loaded = await worker.waitFor((output) => output.type === "extensions_loaded");
    expect(loaded.errors?.map((entry) => entry.path)).toContain(broken);

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "Go." });
    const request = await worker.waitFor((output) => output.type === "extension_ui_request");
    expect(request.method).toBe("select");
    expect(request.title).toBe("Pick a branch");
    expect(request.options).toEqual(["main", "develop"]);

    // The run is still streaming, so this answer has to jump the queue or nothing completes.
    worker.send({ id: crypto.randomUUID(), type: "extension_ui_response", requestId: request.requestId, value: "develop" });
    const answered = await worker.waitFor((output) => output.type === "extension_notice" && output.message === "picked:develop");
    expect(answered.level).toBe("info");

    // And the prompt still finishes normally.
    await worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);
    expect(worker.child.exitCode).toBeNull();
  });

  it("keeps a disabled tool off even when an extension registers it during session_start", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const root = await mkdtemp(join(tmpdir(), "wackcode-dynamic-"));
    cleanup.push(() => rm(root, { recursive: true, force: true }));

    // Registers its tool after load, during session_start — the point at which Pi auto-activates
    // anything new in the registry and would otherwise undo the user's choice.
    const late = join(root, "late.ts");
    await writeFile(late,
      `export default function (pi: any) {
         pi.on("session_start", () => {
           pi.registerTool({ name: "late_tool", label: "l", description: "Registered at session_start", parameters: { type: "object", properties: {} }, async execute() { return { content: [] }; } });
         });
       }\n`);

    const { worker, ready } = await initializeWorker(
      provider.baseUrl, "alpha-secret", root, "dynamic-task", undefined, ["late_tool"],
      { extensions: [late], skills: [], prompts: [], themes: [] }
    );
    cleanup.push(() => worker.shutdown());

    // It exists, so Settings can still show it as an off toggle...
    expect((ready.snapshot?.tools ?? []).some((tool) => tool.name === "late_tool")).toBe(true);
    // ...but it is not active, and the model is never offered it.
    expect(ready.snapshot?.activeTools).not.toContain("late_tool");

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "Go." });
    await worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);
    const offered = (provider.requests[0].body.tools as Array<{ function: { name: string } }> ?? []).map((tool) => tool.function.name);
    expect(offered).not.toContain("late_tool");
  });

  it("loads an extension's tool from an explicit path and offers it to the model, while a project .pi/ stays inert", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const root = await mkdtemp(join(tmpdir(), "wackcode-extension-"));
    cleanup.push(() => rm(root, { recursive: true, force: true }));

    // The extension the user installed and trusted, handed over by absolute path.
    const trusted = join(root, "trusted");
    await mkdir(trusted, { recursive: true });
    await writeFile(join(trusted, "tool.ts"),
      `export default function (pi: any) { pi.registerTool({ name: "trusted_tool", label: "t", description: "A tool from a trusted package", parameters: { type: "object", properties: {} }, async execute() { return { content: [{ type: "text", text: "ok" }] }; } }); }\n`);

    // A project-local extension that must never run: the worker disables all auto-discovery.
    const workspace = join(root, "workspace");
    await mkdir(join(workspace, ".pi", "extensions"), { recursive: true });
    await writeFile(join(workspace, ".pi", "extensions", "untrusted.ts"),
      `export default function (pi: any) { pi.registerTool({ name: "untrusted_tool", label: "u", description: "Must never load", parameters: { type: "object", properties: {} }, async execute() { return { content: [] }; } }); }\n`);

    const { worker, ready } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "extension-task", undefined, undefined,
      { extensions: [join(trusted, "tool.ts")], skills: [], prompts: [], themes: [] }
    );
    cleanup.push(() => worker.shutdown());

    const catalog = ready.snapshot?.tools ?? [];
    const trustedTool = catalog.find((tool) => tool.name === "trusted_tool");
    expect(trustedTool).toBeDefined();
    // Extensions loaded by explicit path are attributed by path, not by a package name.
    expect(trustedTool?.source.kind).toBe("package");
    expect(catalog.some((tool) => tool.name === "untrusted_tool")).toBe(false);

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "Go." });
    await worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);

    const offered = (provider.requests[0].body.tools as Array<{ function: { name: string } }> ?? []).map((tool) => tool.function.name);
    // The regression that matters: a hard `tools` allowlist used to erase this from the registry.
    expect(offered).toContain("trusted_tool");
    expect(offered).not.toContain("untrusted_tool");
  });

  it("advertises the full built-in tool catalogue and applies the denylist live, without a restart", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const root = await mkdtemp(join(tmpdir(), "wackcode-tools-"));
    cleanup.push(() => rm(root, { recursive: true, force: true }));
    const { worker, ready } = await initializeWorker(provider.baseUrl, "alpha-secret", root, "tools-task", undefined, ["find"]);
    cleanup.push(() => worker.shutdown());

    // Every tool Pi ships is in the registry, including the three the worker never used to
    // enable, plus WackCode's three built-in extension tools.
    const names = (ready.snapshot?.tools ?? []).map((tool) => tool.name).sort();
    expect(names).toEqual(["ask_user_question", "bash", "edit", "find", "grep", "ls", "plan_mode_complete", "read", "todo", "write"]);
    expect(ready.snapshot?.tools?.every((tool) => tool.source.kind === "builtin" || tool.source.kind === "wackcode")).toBe(true);
    // The denylist from `init` is applied before the first turn, and tools whose external
    // binary is missing are never offered even though they stay listed in the catalogue.
    const catalog = ready.snapshot?.tools ?? [];
    const expectedActive = catalog
      .filter((tool) => tool.available && tool.name !== "find")
      .map((tool) => tool.name)
      .sort();
    expect(ready.snapshot?.activeTools?.sort()).toEqual(expectedActive);
    for (const tool of catalog) {
      expect(tool.available || typeof tool.unavailableReason === "string").toBe(true);
    }

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "Create the fixture." });
    await worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);

    const offered = (request: (typeof provider.requests)[number]) =>
      (request.body.tools as Array<{ function: { name: string } }> ?? []).map((tool) => tool.function.name).sort();
    expect(offered(provider.requests[0])).toEqual(expectedActive);

    // Toggling tools takes effect on the next turn with no worker restart. The wackcode
    // tools are exempt from the denylist, so six tools stay active.
    worker.send({ id: crypto.randomUUID(), type: "set_tools", disabledTools: ["bash", "grep", "ls", "find"] });
    await worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.activeTools?.length === 6);
    // `find`/`grep` availability varies by host, so assert the five that never depend on a binary.
    const beforeSecondRun = provider.requests.length;
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-2", message: "Again." });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.snapshot === undefined && provider.requests.length > beforeSecondRun);

    expect(offered(provider.requests[beforeSecondRun])).toEqual(["ask_user_question", "edit", "plan_mode_complete", "read", "todo", "write"]);
    expect(worker.child.exitCode).toBeNull();
  });

  it("keeps overlapping model IDs, credentials, sessions, and edits isolated across concurrent tasks", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const root = await mkdtemp(join(tmpdir(), "wackcode-worker-"));
    cleanup.push(() => rm(root, { recursive: true, force: true }));
    const alphaRoot = join(root, "alpha-workspace");
    const betaRoot = join(root, "beta-workspace");
    await Promise.all([mkdir(alphaRoot), mkdir(betaRoot)]);
    const alpha = await initializeWorker(provider.baseUrl, "alpha-secret", alphaRoot, "alpha-task");
    const beta = await initializeWorker(provider.baseUrl, "beta-secret", betaRoot, "beta-task");
    cleanup.push(() => alpha.worker.shutdown(), () => beta.worker.shutdown());

    alpha.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "alpha-run", message: "Create the fixture." });
    beta.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "beta-run", message: "Create the fixture." });
    await Promise.all([
      alpha.worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true),
      beta.worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.messages.some((message) => message.blocks.some((block) => block.text === "Finished beta.")) === true)
    ]);

    expect(await readFile(join(alphaRoot, "alpha.txt"), "utf8")).toBe("changed by alpha\n");
    expect(await readFile(join(betaRoot, "beta.txt"), "utf8")).toBe("changed by beta\n");
    expect(provider.requests.filter((request) => request.authorization === "Bearer alpha-secret")).toHaveLength(2);
    expect(provider.requests.filter((request) => request.authorization === "Bearer beta-secret")).toHaveLength(2);
    for (const request of provider.requests) {
      expect(request.body.model).toBe("shared-model");
      expect(request.body.max_completion_tokens).toBe(321);
      expect(request.body.reasoning_effort).toBe("wire-high");
    }
    expect(JSON.stringify(alpha.worker.outputs) + alpha.worker.stderr).not.toContain("alpha-secret");
    expect(JSON.stringify(beta.worker.outputs) + beta.worker.stderr).not.toContain("beta-secret");

    const savedSession = alpha.worker.outputs.findLast((output) => output.snapshot?.sessionFile)?.snapshot?.sessionFile;
    expect(savedSession).toBeTruthy();
    await alpha.worker.shutdown();
    const requestCount = provider.requests.length;
    const reopened = await initializeWorker(provider.baseUrl, "alpha-secret", alphaRoot, "alpha-task", savedSession);
    cleanup.push(() => reopened.worker.shutdown());
    expect(reopened.ready.snapshot?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha."))).toBe(true);
    expect(provider.requests).toHaveLength(requestCount);
  });

  it("streams partial assistant messages before the authoritative snapshot", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-stream-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "stream-secret", workspace, "stream-task");
    cleanup.push(() => worker.shutdown());
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "stream-run", message: "Talk slowly." });

    await worker.waitFor((output) => output.type === "partial", 8_000);
    const firstPartial = worker.outputs.findIndex((output) => output.type === "partial");
    const finalSnapshot = await worker.waitFor((output) =>
      output.type === "snapshot" && output.snapshot?.messages.some((message) =>
        message.blocks.some((block) => block.text === "Streaming the answer in pieces.")));
    const finalIndex = worker.outputs.indexOf(finalSnapshot);
    expect(firstPartial).toBeGreaterThan(-1);
    expect(firstPartial).toBeLessThan(finalIndex);
  });

  it("forwards accumulated tool output before the final result", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-tool-stream-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "tool-stream-secret", workspace, "tool-stream-task");
    cleanup.push(() => worker.shutdown());
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "tool-stream-run", message: "stream tool" });

    const first = await worker.waitFor((output) =>
      output.type === "activity" && output.event === "tool_execution_update" && output.detail?.text?.includes("first line") === true);
    const second = await worker.waitFor((output) =>
      output.type === "activity" && output.event === "tool_execution_update" && output.detail?.text?.includes("second line") === true);
    const finished = await worker.waitFor((output) =>
      output.type === "snapshot" && output.snapshot?.messages.some((message) =>
        message.blocks.some((block) => block.type === "tool-result" && block.text?.includes("second line"))) === true);

    expect(first.detail?.toolCallId).toBe("call-beta");
    expect(second.detail?.toolCallId).toBe(first.detail?.toolCallId);
    expect(second.detail?.text).toContain("first line");
    expect(worker.outputs.indexOf(first)).toBeLessThan(worker.outputs.indexOf(finished));
    expect(worker.outputs.indexOf(second)).toBeLessThan(worker.outputs.indexOf(finished));
  });

  it("cancels an in-flight provider stream without reporting cancellation as a worker failure", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-cancel-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "cancel-secret", workspace, "cancel-task");
    cleanup.push(() => worker.shutdown());
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "cancel-run", message: "Wait until stopped." });
    await provider.waitForSlowRequest();
    worker.send({ id: crypto.randomUUID(), type: "abort" });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    expect(worker.outputs.some((output) => output.type === "run_state" && output.state === "stopping")).toBe(true);
    expect(worker.outputs.some((output) => output.type === "worker_error")).toBe(false);
    const stopped = worker.outputs.findLast((output) => output.type === "snapshot" && output.snapshot?.runTimings?.length === 1);
    expect(stopped?.snapshot?.runTimings).toHaveLength(1);
    const user = stopped?.snapshot?.messages.find((message) => message.role === "user");
    expect(stopped?.snapshot?.runTimings?.[0]?.userMessageId).toBe(user?.id);
  });
});

describe("built-in extensions", () => {
  it("persists one duration per user prompt and restores the timings from Pi's session", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-timing-restore-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const first = await initializeWorker(provider.baseUrl, "timing-secret", workspace, "timing-task");
    cleanup.push(() => first.worker.shutdown());

    const firstStartedAt = Date.now() - 5_000;
    first.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "timing-run-1", startedAt: firstStartedAt, message: "First turn." });
    const firstDone = await first.worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.runTimings?.length === 1);
    await first.worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "timing-run-1");
    const firstUser = firstDone.snapshot?.messages.find((message) => message.role === "user");
    expect(firstDone.snapshot?.runTimings?.[0]?.userMessageId).toBe(firstUser?.id);
    expect(firstDone.snapshot?.runTimings?.[0]?.durationMs).toBeGreaterThanOrEqual(5_000);
    expect(firstDone.snapshot?.activeRun).toBeUndefined();

    first.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "timing-run-2", startedAt: Date.now() - 2_000, message: "Second turn." });
    const bothDone = await first.worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.runTimings?.length === 2);
    await first.worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "timing-run-2");
    const userIds = bothDone.snapshot?.messages.filter((message) => message.role === "user").map((message) => message.id);
    expect(bothDone.snapshot?.runTimings?.map((timing) => timing.userMessageId)).toEqual(userIds);

    const sessionFile = bothDone.snapshot?.sessionFile;
    expect(sessionFile).toBeTruthy();
    const savedSession = await readFile(sessionFile as string, "utf8");
    expect(savedSession.match(/wackcode-run-timing/g)).toHaveLength(2);
    await first.worker.shutdown();

    const restored = await initializeWorker(provider.baseUrl, "timing-secret", workspace, "timing-task", sessionFile);
    cleanup.push(() => restored.worker.shutdown());
    expect(restored.ready.snapshot?.runTimings).toHaveLength(2);
    const restoredUserIds = restored.ready.snapshot?.messages.filter((message) => message.role === "user").map((message) => message.id);
    expect(restored.ready.snapshot?.runTimings.map((timing) => timing.userMessageId)).toEqual(restoredUserIds);
  });

  it("registers the built-in tools as wackcode sources that ignore the denylist", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-builtin-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    // Even a denylist that names them cannot switch built-in tools off.
    const { worker, ready } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "builtin-task", undefined,
      ["ask_user_question", "plan_mode_complete", "todo"]
    );
    cleanup.push(() => worker.shutdown());

    const catalog = ready.snapshot?.tools ?? [];
    const ask = catalog.find((tool) => tool.name === "ask_user_question");
    const complete = catalog.find((tool) => tool.name === "plan_mode_complete");
    const todo = catalog.find((tool) => tool.name === "todo");
    expect(ask?.source.kind).toBe("wackcode");
    expect(complete?.source.kind).toBe("wackcode");
    expect(todo?.source.kind).toBe("wackcode");
    expect(ready.snapshot?.activeTools).toContain("ask_user_question");
    expect(ready.snapshot?.activeTools).toContain("plan_mode_complete");
    expect(ready.snapshot?.activeTools).toContain("todo");
    expect(ready.snapshot?.planState?.mode).toBe("build");
  });

  it("round-trips an ask_user_question dialog while the prompt holds the queue", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-ask-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "ask-task");
    cleanup.push(() => worker.shutdown());

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "ask: which engine?" });
    const request = await worker.waitFor((output) => output.type === "extension_ui_request" && output.method === "questions");
    expect(request.questions?.[0]?.id).toBe("approach");
    expect(request.questions?.[0]?.options.map((option) => option.label)).toEqual(["SQLite", "Postgres"]);

    // The tool is awaiting this inside the in-flight prompt, so the response must bypass
    // the queue — if it didn't, this test would simply time out.
    worker.send({
      id: crypto.randomUUID(),
      type: "extension_ui_response",
      requestId: request.requestId,
      answers: [{ questionId: "approach", selected: ["Postgres"] }]
    });

    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    // The model saw the formatted answer, not an error or a cancellation.
    const toolResult = provider.requests[1].body.messages.find(
      (message) => (message as { role?: string }).role === "tool"
    ) as { content?: string } | undefined;
    expect(toolResult?.content).toContain("Postgres");
    expect(worker.child.exitCode).toBeNull();
  });

  it("cancelling the questions dialog resolves the tool instead of hanging the run", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-askcancel-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "ask-cancel-task");
    cleanup.push(() => worker.shutdown());

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "ask: which engine?" });
    const request = await worker.waitFor((output) => output.type === "extension_ui_request" && output.method === "questions");
    worker.send({ id: crypto.randomUUID(), type: "extension_ui_response", requestId: request.requestId, cancelled: true });

    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    const toolResult = provider.requests[1].body.messages.find(
      (message) => (message as { role?: string }).role === "tool"
    ) as { content?: string } | undefined;
    expect(toolResult?.content).toContain("dismissed");
  });

  it("publishes todo_state with a replay snapshot in every result, and restores across a restart", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-todo-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const first = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "todo-task");
    cleanup.push(() => first.worker.shutdown());

    first.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "todo: start the work" });
    const published = await first.worker.waitFor((output) => output.type === "todo_state" && output.tasks?.length === 1);
    expect(published.tasks?.[0]).toMatchObject({ id: 1, subject: "Ship the thing", status: "pending" });
    await first.worker.waitFor((output) => output.type === "run_state" && output.state === "idle");

    // The provider wire only sees the formatted text...
    const wireResult = provider.requests[1].body.messages.find(
      (message) => (message as { role?: string }).role === "tool"
    ) as { content?: string } | undefined;
    expect(wireResult?.content).toContain("Created #1: Ship the thing (pending)");

    // ...while the session's tool result embeds the full list — that snapshot is the
    // whole persistence layer.
    const withTodo = await first.worker.waitFor((output) =>
      output.type === "snapshot" &&
      output.snapshot?.messages.some((message) => message.blocks.some((block) => block.toolName === "todo" && block.details !== undefined)) === true
    );
    const details = withTodo.snapshot?.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.toolName === "todo" && block.details !== undefined)
      ?.details as { version?: number; tasks?: unknown[]; nextId?: number } | undefined;
    expect(details?.version).toBe(1);
    expect(details?.nextId).toBe(2);
    expect(details?.tasks).toHaveLength(1);
    const sessionFile = first.ready.snapshot?.sessionFile;
    expect(sessionFile).toBeTruthy();
    await first.worker.shutdown();

    // No disk writes: the list is replayed from the branch after the restart.
    const second = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "todo-task", sessionFile);
    cleanup.push(() => second.worker.shutdown());
    expect(second.ready.snapshot?.todoState?.tasks).toMatchObject([
      { id: 1, subject: "Ship the thing", status: "pending" }
    ]);
  });

  it("stays usable in Plan mode — it mutates only its own list, never the workspace", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-todo-plan-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "todo-plan-task", undefined, undefined, undefined, "plan"
    );
    cleanup.push(() => worker.shutdown());
    await worker.waitFor((output) => output.type === "plan_state" && output.mode === "plan");

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "todo: plan the work" });
    const published = await worker.waitFor((output) => output.type === "todo_state" && output.tasks?.length === 1);
    expect(published.tasks?.[0]?.subject).toBe("Ship the thing");
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    expect(worker.outputs.some((output) => output.type === "worker_error")).toBe(false);
  });
});

describe("Plan mode", () => {
  it("publishes plan_state on mode changes and blocks mutating tools while planning", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-plan-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker, ready } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "plan-task");
    cleanup.push(() => worker.shutdown());
    expect(ready.snapshot?.planState?.mode).toBe("build");

    worker.send({ id: crypto.randomUUID(), type: "set_mode", mode: "plan" });
    const entered = await worker.waitFor((output) => output.type === "plan_state" && output.mode === "plan");
    expect(entered.phase).toBe("planning");

    // The fake model tries to `write` — Plan mode must block it, and the model must see why.
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "Create the fixture." });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    const toolResult = provider.requests[1].body.messages.find(
      (message) => (message as { role?: string }).role === "tool"
    ) as { content?: string } | undefined;
    expect(toolResult?.content).toContain("Plan mode");
    // Nothing was written: the policy blocked the call before it ran.
    await expect(readFile(join(workspace, "alpha.txt"), "utf8")).rejects.toThrow();

    worker.send({ id: crypto.randomUUID(), type: "set_mode", mode: "build" });
    await worker.waitFor((output) => output.type === "plan_state" && output.mode === "build");
  });

  it("marks the state ready when the agent submits a plan, and clears it on revision", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-ready-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "ready-task", undefined, undefined, undefined, "plan"
    );
    cleanup.push(() => worker.shutdown());
    await worker.waitFor((output) => output.type === "plan_state" && output.mode === "plan");

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "finish plan" });
    const readyState = await worker.waitFor((output) => output.type === "plan_state" && output.phase === "ready");
    expect(readyState.plan).toBe("# The plan\n\n- Ship it");

    // A fresh prompt is revision feedback — the old plan can no longer be approved.
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-2", message: "Actually, change it." });
    await worker.waitFor((output) => output.type === "plan_state" && output.phase === "planning" && output.mode === "plan");
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
  });

  it("restores mode and plan across a worker restart, with the record's mode winning", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-restore-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));

    const first = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "restore-task", undefined, undefined, undefined, "plan"
    );
    first.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "finish plan" });
    await first.worker.waitFor((output) => output.type === "plan_state" && output.phase === "ready");
    const sessionFile = first.ready.snapshot?.sessionFile;
    expect(sessionFile).toBeTruthy();
    await first.worker.shutdown();

    // The session restores Plan mode + the proposed plan from its own entries.
    const second = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "restore-task", sessionFile);
    cleanup.push(() => second.worker.shutdown());
    const restored = await second.worker.waitFor((output) => output.type === "plan_state" && output.phase === "ready");
    expect(restored.mode).toBe("plan");
    expect(restored.plan).toBe("# The plan\n\n- Ship it");
    await second.worker.shutdown();

    // But the task record's mode wins over the restored session when they disagree —
    // the record is the user's latest explicit choice.
    const third = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "restore-task", sessionFile, undefined, undefined, "build"
    );
    cleanup.push(() => third.worker.shutdown());
    await third.worker.waitFor((output) => output.type === "plan_state" && output.mode === "build");
  });
});

describe("image attachments", () => {
  it("resizes an attached image with Pi's pipeline, sends it to a vision model, and previews it", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-vision-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "vision-task", undefined, undefined, undefined, undefined, true
    );
    cleanup.push(() => worker.shutdown());
    // Wider than Pi's 2000px inline limit, so the worker has to resize it.
    const original = solidPng(2400, 16);
    worker.send({
      id: crypto.randomUUID(), type: "prompt", runId: "vision-run", message: "What is in this image?",
      images: [{ type: "image", data: original, mimeType: "image/png" }]
    });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");

    const images = userParts(provider.requests[0]).filter((part) => part.type === "image_url");
    expect(images).toHaveLength(1);
    const sent = images[0].image_url?.url ?? "";
    expect(sent).toMatch(/^data:image\/(png|jpeg);base64,/);
    expect(sent.endsWith(original)).toBe(false);
    expect(userParts(provider.requests[0]).some((part) => part.type === "text" && part.text === "What is in this image?")).toBe(true);

    const previewed = await worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.messages.some((message) =>
      message.blocks.some((block) => block.type === "image" && block.thumbnail?.startsWith("data:image/"))) === true);
    const user = previewed.snapshot?.messages.find((message) => message.role === "user");
    const image = user?.blocks.find((block) => block.type === "image");
    expect(image?.imageId).toBeTruthy();
    // Snapshots carry the preview only, never the multi-kilobyte original.
    expect(JSON.stringify(previewed)).not.toContain(original);
    expect(user?.blocks.find((block) => block.type === "text")?.text).toBe("What is in this image?");
  });

  it("lets Pi swap the image for its placeholder when the model has no vision", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-blind-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "blind-task");
    cleanup.push(() => worker.shutdown());
    worker.send({
      id: crypto.randomUUID(), type: "prompt", runId: "blind-run", message: "Look.",
      images: [{ type: "image", data: solidPng(4, 4), mimeType: "image/png" }]
    });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    const parts = userParts(provider.requests[0]);
    expect(parts.some((part) => part.type === "image_url")).toBe(false);
    expect(parts.some((part) => part.text === "(image omitted: model does not support images)")).toBe(true);
  });

  it("keeps attached images in the Pi session file across a restart", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-image-restore-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const first = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "image-restore", undefined, undefined, undefined, undefined, true
    );
    first.worker.send({
      id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "Remember this.",
      images: [{ type: "image", data: solidPng(8, 8), mimeType: "image/png" }]
    });
    await first.worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    const sessionFile = first.worker.outputs.findLast((output) => output.snapshot?.sessionFile)?.snapshot?.sessionFile;
    expect(sessionFile).toBeTruthy();
    // Stored in Pi's own format, so the session stays readable by Pi itself.
    const stored = await readFile(sessionFile as string, "utf8");
    expect(stored).toContain('"type":"image"');
    expect(stored).toContain('"mimeType":"image/png"');
    await first.worker.shutdown();

    const second = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "image-restore", sessionFile, undefined, undefined, undefined, true
    );
    cleanup.push(() => second.worker.shutdown());
    const restoredUser = second.ready.snapshot?.messages.find((message) => message.role === "user");
    expect(restoredUser?.blocks.some((block) => block.type === "image" && block.imageId)).toBe(true);
    second.worker.send({ id: crypto.randomUUID(), type: "snapshot" });
    await second.worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.messages.some((message) =>
      message.blocks.some((block) => block.type === "image" && Boolean(block.thumbnail))) === true);
  });

  it("fails the run cleanly when an attachment cannot be decoded", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-bad-image-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "bad-image", undefined, undefined, undefined, undefined, true
    );
    cleanup.push(() => worker.shutdown());
    worker.send({
      id: crypto.randomUUID(), type: "prompt", runId: "bad-run", message: "Look.",
      images: [{ type: "image", data: Buffer.from("not an image").toString("base64"), mimeType: "image/png" }]
    });
    const failure = await worker.waitFor((output) => output.type === "worker_error");
    expect(failure.message).toContain("image");
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    expect(provider.requests).toHaveLength(0);
  });
});
