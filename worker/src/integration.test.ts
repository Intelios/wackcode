import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { afterEach, describe, expect, it } from "vitest";

interface Output {
  type: string;
  taskId?: string;
  state?: string;
  message?: string;
  requestId?: string;
  method?: string;
  title?: string;
  options?: string[];
  level?: string;
  errors?: Array<{ path: string; error: string }>;
  snapshot?: {
    sessionFile?: string;
    messages: Array<{ blocks: Array<{ type: string; text?: string }> }>;
    tools?: Array<{ name: string; description: string; source: { kind: string; packageId?: string }; available: boolean; unavailableReason?: string }>;
    activeTools?: string[];
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
  if (!hasToolResult) {
    send({
      id: `tool-${suffix}`,
      object: "chat.completion.chunk",
      created: 1,
      model: "shared-model",
      choices: [{
        index: 0,
        delta: {
          role: "assistant",
          tool_calls: [{
            index: 0,
            id: `call-${suffix}`,
            type: "function",
            function: { name: "write", arguments: JSON.stringify({ path: `${suffix}.txt`, content: `changed by ${suffix}\n` }) }
          }]
        },
        finish_reason: null
      }]
    });
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
  resources?: { extensions: string[]; skills: string[]; prompts: string[]; themes: string[] }
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
      baseUrl,
      api: "openai-completions",
      models: [{
        id: "shared-model",
        name: "Shared model",
        contextWindow: 16_384,
        maxTokens: 321,
        reasoning: true,
        thinkingLevels: ["off", "high"],
        thinkingLevelMap: { off: null, high: "wire-high" }
      }]
    },
    modelId: "shared-model",
    apiKey,
    thinkingLevel: "high",
    disabledTools,
    resources
  });
  return { worker, ready: await worker.waitFor((output) => output.type === "ready") };
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

    // Every tool Pi ships is in the registry, including the three the worker never used to enable.
    const names = (ready.snapshot?.tools ?? []).map((tool) => tool.name).sort();
    expect(names).toEqual(["bash", "edit", "find", "grep", "ls", "read", "write"]);
    expect(ready.snapshot?.tools?.every((tool) => tool.source.kind === "builtin")).toBe(true);
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

    // Toggling tools takes effect on the next turn with no worker restart.
    worker.send({ id: crypto.randomUUID(), type: "set_tools", disabledTools: ["bash", "grep", "ls", "find"] });
    await worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.activeTools?.length === 3);
    // `find`/`grep` availability varies by host, so assert the three that never depend on a binary.
    const beforeSecondRun = provider.requests.length;
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-2", message: "Again." });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.snapshot === undefined && provider.requests.length > beforeSecondRun);

    expect(offered(provider.requests[beforeSecondRun])).toEqual(["edit", "read", "write"]);
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
  });
});
