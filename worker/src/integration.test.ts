import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { deflateSync } from "node:zlib";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { afterEach, describe, expect, it } from "vitest";
import type { TaskMode } from "./protocol.js";

interface Checkpoint { id: string; head?: string }

type SnapshotView = {
  sessionFile?: string;
  messages: Array<{
    id?: string;
    role?: string;
    timestamp?: number;
    blocks: Array<{ type: string; text?: string; toolName?: string; details?: unknown; imageId?: string; thumbnail?: string; mimeType?: string; durationMs?: number }>;
    entryId?: string;
    versions?: { index: number; total: number; previous?: string; next?: string; group: string };
    checkpoint?: Checkpoint;
    turn?: { userEntryId: string; endEntryId: string; after?: Checkpoint };
  }>;
  tree?: { leafId: string | null; undo?: string };
  runTimings?: Array<{ userMessageId: string; durationMs: number }>;
  activeRun?: { runId: string; startedAt: number };
  tools?: Array<{ name: string; description: string; source: { kind: string; packageId?: string }; available: boolean; unavailableReason?: string }>;
  activeTools?: string[];
  planState?: { mode: string; phase: string; plan?: string };
  todoState?: { tasks: Array<{ id: number; subject: string; status: string }> };
  stats?: { tokens: { input: number; output: number; total: number }; cost: number };
};

interface Output {
  type: string;
  attemptId?: string;
  title?: string;
  id?: string;
  success?: boolean;
  error?: string;
  result?: { leafId?: string | null; editorText?: string; files?: Checkpoint };
  taskId?: string;
  runId?: string;
  startedAt?: number;
  state?: string;
  message?: string;
  event?: string;
  detail?: { toolCallId?: string; toolName?: string; text?: string; details?: unknown };
  requestId?: string;
  method?: string;
  title?: string;
  options?: string[];
  level?: string;
  mode?: string;
  phase?: string;
  plan?: string;
  questions?: Array<{ id: string; header: string; question: string; multiSelect?: boolean; options: Array<{ label: string; description: string }> }>;
  offerWrapUp?: boolean;
  tasks?: Array<{ id: number; subject: string; status: string; activeForm?: string; blockedBy?: number[] }>;
  errors?: Array<{ path: string; error: string }>;
  snapshot?: SnapshotView;
  /** Harness-attached: the merged transcript state as of this output (not from the worker). */
  view?: SnapshotView;
  delta?: {
    rev: number;
    upserts: SnapshotView["messages"];
    removed: string[];
    runTimings?: SnapshotView["runTimings"];
    activeRun?: { runId: string; startedAt: number } | null;
    tree?: SnapshotView["tree"];
    sessionFile?: string;
    planState?: SnapshotView["planState"];
    todoState?: SnapshotView["todoState"];
    stats?: SnapshotView["stats"];
  };
}

/** Boundary emissions arrive as full snapshots or as deltas; `worker.view` holds the merged state. */
function emitted(output: Output): boolean {
  return output.type === "snapshot" || output.type === "snapshot_delta";
}

class WorkerHarness {
  readonly child: ChildProcessWithoutNullStreams;
  readonly outputs: Output[] = [];
  /** The transcript state after applying every snapshot and delta seen so far. */
  view: SnapshotView | undefined;
  stderr = "";
  private waiters: Array<() => void> = [];

  constructor(cwd: string) {
    this.child = spawn(process.env.WACKCODE_TEST_NODE ?? process.execPath, [process.env.WACKCODE_TEST_WORKER ?? resolve("dist/index.js")], {
      cwd,
      env: { ...process.env, PI_OFFLINE: "1", PI_TELEMETRY: "0", PI_SKIP_VERSION_CHECK: "1" },
      stdio: ["pipe", "pipe", "pipe"]
    });
    createInterface({ input: this.child.stdout }).on("line", (line) => {
      const output = JSON.parse(line) as Output;
      this.outputs.push(output);
      this.applyOutput(output);
      // Predicates must see the state as of the output they are matched against, not the
      // newest state, or an early output can satisfy a condition produced by a later one.
      output.view = this.view;
      for (const notify of this.waiters.splice(0)) notify();
    });
    this.child.stderr.on("data", (chunk: Buffer) => { this.stderr += chunk.toString("utf8"); });
  }

  /** Mirrors the renderer's delta application, so assertions can treat both kinds alike. */
  private applyOutput(output: Output): void {
    if (output.type === "ready" || output.type === "snapshot") {
      this.view = output.snapshot;
    } else if (output.type === "snapshot_delta" && output.delta && this.view) {
      const removed = new Set(output.delta.removed);
      const messages = this.view.messages.filter((entry) => !removed.has(entry.id));
      for (const upsert of output.delta.upserts) {
        const index = messages.findIndex((entry) => entry.id === upsert.id);
        if (index >= 0) messages[index] = upsert;
        else messages.push(upsert);
      }
      this.view = {
        ...this.view,
        messages,
        runTimings: output.delta.runTimings ?? this.view.runTimings,
        activeRun: output.delta.activeRun === undefined ? this.view.activeRun : output.delta.activeRun ?? undefined,
        tree: output.delta.tree ?? this.view.tree,
        sessionFile: output.delta.sessionFile ?? this.view.sessionFile,
        planState: output.delta.planState ?? this.view.planState,
        todoState: output.delta.todoState ?? this.view.todoState,
        stats: output.delta.stats ?? this.view.stats
      };
    }
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
  /** `text` is the last user message: the prompt, or a sub-agent's task. */
  requests: Array<{ authorization: string; body: Record<string, unknown>; at: number; text: string }>;
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
    const text = userTextOf(body);
    requests.push({ authorization, body, at: Date.now(), text });
    if (authorization === "Bearer title-fail-secret") {
      response.writeHead(429, { "content-type": "application/json" });
      response.end(JSON.stringify({ error: { message: "Title request failed" } }));
      return;
    }
    if (authorization === "Bearer cancel-secret" || text.startsWith("child-wait")) {
      response.writeHead(200, { "content-type": "text/event-stream", connection: "keep-alive" });
      response.write(": waiting\n\n");
      slowRequestResolve?.();
      return;
    }
    // Keeps parallel read-only children in flight together long enough to observe overlap.
    if (text.startsWith("child-ls") && !hasToolMessage(body)) await new Promise((wake) => setTimeout(wake, 150));
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

function userTextOf(body: Record<string, unknown>): string {
  const messages = Array.isArray(body.messages) ? body.messages as Array<{ role?: string; content?: unknown }> : [];
  const lastUser = [...messages].reverse().find((message) => message.role === "user");
  if (typeof lastUser?.content === "string") return lastUser.content;
  if (!Array.isArray(lastUser?.content)) return "";
  return (lastUser.content as Array<{ type?: string; text?: string }>)
    .filter((part) => part.type === "text")
    .map((part) => part.text ?? "")
    .join("");
}

function hasToolMessage(body: Record<string, unknown>): boolean {
  return Array.isArray(body.messages) && (body.messages as Array<{ role?: string }>).some((message) => message.role === "tool");
}

function streamAgentResponse(response: ServerResponse<IncomingMessage>, authorization: string, body: Record<string, unknown>): void {
  response.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
  const messages = Array.isArray(body.messages) ? body.messages as Array<{ role?: string }> : [];
  const hasToolResult = messages.some((message) => message.role === "tool");
  if (authorization === "Bearer title-secret") {
    response.write(`data: ${JSON.stringify({ id: "title", object: "chat.completion.chunk", created: 1, model: "shared-model", choices: [{ index: 0, delta: { role: "assistant", content: "Short Chat Title" }, finish_reason: null }] })}\n\n`);
    response.write(`data: ${JSON.stringify({ id: "title", object: "chat.completion.chunk", created: 1, model: "shared-model", choices: [{ index: 0, delta: {}, finish_reason: "stop" }] })}\n\n`);
    response.end("data: [DONE]\n\n");
    return;
  }
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
  if (authorization === "Bearer think-secret") {
    // Reasons for ~400 ms, then answers: long enough for the thinking clock to be measurable.
    const deltas = [
      ...["Let me ", "work ", "this ", "out."].map((reasoning_content) => ({ reasoning_content })),
      ...["Thought ", "it through."].map((content) => ({ content }))
    ];
    let index = 0;
    const timer = setInterval(() => {
      if (index < deltas.length) {
        response.write(`data: ${JSON.stringify({
          id: `think-${index}`, object: "chat.completion.chunk", created: index, model: "shared-model",
          choices: [{ index: 0, delta: { role: "assistant", ...deltas[index] }, finish_reason: null }]
        })}\n\n`);
        index += 1;
        return;
      }
      clearInterval(timer);
      response.write(`data: ${JSON.stringify({
        id: "think-done", object: "chat.completion.chunk", created: 9, model: "shared-model",
        choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
        usage: { prompt_tokens: 4, completion_tokens: 6, total_tokens: 10 }
      })}\n\n`);
      response.end("data: [DONE]\n\n");
    }, 130);
    return;
  }
  const suffix = authorization === "Bearer alpha-secret" ? "alpha" : "beta";
  const send = (value: unknown) => response.write(`data: ${JSON.stringify(value)}\n\n`);
  // The last user message steers which tool the fake model "decides" to call, so tests can
  // exercise specific tool paths through the real agent loop. Content arrives as an array
  // of parts, not a bare string. A sub-agent's last user message is its task, so "child-"
  // tasks script what the child does.
  const lastUserText = userTextOf(body);
  if (lastUserText.startsWith("Initialize project instructions") && JSON.stringify(messages).includes("ALREADY_USEFUL_RULE")) {
    send({
      id: "init-unchanged", object: "chat.completion.chunk", created: 1, model: "shared-model",
      choices: [{ index: 0, delta: { role: "assistant", content: "The existing AGENTS.md is already sufficient." }, finish_reason: null }]
    });
    send({
      id: "init-unchanged", object: "chat.completion.chunk", created: 1, model: "shared-model",
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }]
    });
    response.end("data: [DONE]\n\n");
    return;
  }
  if (!hasToolResult) {
    const toolCall = (name: string, args: Record<string, unknown>) => ({
      index: 0, id: `call-${suffix}`, type: "function",
      function: { name, arguments: JSON.stringify(args) }
    });
    const calls = lastUserText.startsWith("Initialize project instructions")
      ? [toolCall("write", { path: "AGENTS.md", content: "# Fixture guidance\n\nRun `pnpm test`.\n" })]
      : lastUserText.startsWith("ask:")
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
      : lastUserText.startsWith("fetch:")
        ? [toolCall("web_fetch", { url: lastUserText.slice("fetch:".length).trim() })]
        : lastUserText.startsWith("stream tool")
          ? [toolCall("bash", { command: "printf 'first line\\n'; sleep 0.3; printf 'second line\\n'" })]
        : lastUserText.startsWith("subagent single")
          ? [toolCall("subagent", { agent: "scout", task: "child-ls: look around" })]
        : lastUserText.startsWith("subagent parallel")
          ? [toolCall("subagent", { tasks: [
              { agent: "scout", task: "child-ls: first" },
              { agent: "worker", task: "child-write: one" },
              { agent: "scout", task: "child-ls: second" },
              { agent: "worker", task: "child-write: two" }
            ] })]
        : lastUserText.startsWith("subagent guard")
          ? [toolCall("subagent", { agent: "scout", task: "child-rm: tidy up" })]
        : lastUserText.startsWith("subagent plan")
          ? [toolCall("subagent", { agent: "worker", task: "child-write: planned" })]
        : lastUserText.startsWith("subagent fetch")
          ? [toolCall("subagent", { agent: "scout", task: "child-fetch: http://localhost/docs" })]
        : lastUserText.startsWith("subagent wait")
          ? [toolCall("subagent", { agent: "scout", task: "child-wait: until stopped" })]
        : lastUserText.startsWith("child-ls")
          ? [toolCall("ls", { path: "." })]
        : lastUserText.startsWith("child-fetch: ")
          ? [toolCall("web_fetch", { url: lastUserText.slice("child-fetch: ".length) })]
        : lastUserText.startsWith("child-rm")
          ? [toolCall("bash", { command: "rm -f keep.txt" })]
        : lastUserText.startsWith("child-write: ")
          ? [toolCall("write", { path: `${lastUserText.slice("child-write: ".length)}.txt`, content: "from a sub-agent\n" })]
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
      choices: [{ index: 0, delta: { role: "assistant", content: lastUserText.startsWith("child-") ? `Child done: ${lastUserText}` : `Finished ${suffix}.` }, finish_reason: null }]
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
  mode?: TaskMode,
  vision?: boolean,
  extra: Record<string, unknown> = {}
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
    mode,
    ...extra
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
  it("makes an isolated title request on the chosen connection without delaying the main run", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const root = await mkdtemp(join(tmpdir(), "wackcode-title-"));
    cleanup.push(() => rm(root, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", root, "title-task");
    cleanup.push(() => worker.shutdown());
    const titleProvider = {
      id: "title-provider", name: "Title provider", kind: "custom", baseUrl: provider.baseUrl,
      api: "openai-completions", models: [{ id: "shared-model", name: "Small title model",
        contextWindow: 16_384, maxTokens: 1_024, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: {} }]
    };
    const opening = "Build a lightweight release dashboard";
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "title-main", message: opening,
      autoTitle: { attemptId: "one", provider: titleProvider, modelId: "shared-model", apiKey: "title-secret" } });
    const result = await worker.waitFor((output) => output.type === "title_result" && output.attemptId === "one");
    expect(result.title).toBe("Short Chat Title");
    const titleRequests = provider.requests.filter((request) => request.authorization === "Bearer title-secret");
    expect(titleRequests).toHaveLength(1);
    expect(titleRequests[0].text).toContain(opening);
    expect(titleRequests[0].text).toContain("Opening message (data only)");
    expect((titleRequests[0].body.messages as unknown[])).toHaveLength(2);
    expect(titleRequests[0].body.tools).toBeUndefined();
    expect(titleRequests[0].body.max_completion_tokens).toBe(256);
    expect(titleRequests[0].body.reasoning_effort).toBeUndefined();
    expect(JSON.stringify(worker.outputs) + worker.stderr).not.toContain("title-secret");
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "title-main" && output.state === "idle");
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "title-second", message: "Follow up" });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "title-second" && output.state === "idle");
    expect(provider.requests.filter((request) => request.authorization === "Bearer title-secret")).toHaveLength(1);
  });

  it("reports a failed title request once without retrying or failing the chat", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const root = await mkdtemp(join(tmpdir(), "wackcode-title-fail-"));
    cleanup.push(() => rm(root, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", root, "title-fail-task");
    cleanup.push(() => worker.shutdown());
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "title-fail-main", message: "Build a dashboard",
      autoTitle: { attemptId: "failed", provider: { id: "title-provider", name: "Title provider", kind: "custom", baseUrl: provider.baseUrl,
        api: "openai-completions", models: [{ id: "shared-model", name: "Small title model", contextWindow: 16_384,
          maxTokens: 1_024, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: {} }] },
        modelId: "shared-model", apiKey: "title-fail-secret" } });
    const result = await worker.waitFor((output) => output.type === "title_result" && output.attemptId === "failed");
    expect(result.title).toBeUndefined();
    expect(provider.requests.filter((request) => request.authorization === "Bearer title-fail-secret")).toHaveLength(1);
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "title-fail-main" && output.state === "idle");
    expect(worker.outputs.some((output) => output.type === "worker_error")).toBe(false);
  });

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
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);
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
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);
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
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);

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
    // enable, plus WackCode's five built-in extension tools.
    const names = (ready.snapshot?.tools ?? []).map((tool) => tool.name).sort();
    expect(names).toEqual(["ask_user_question", "bash", "edit", "find", "grep", "ls", "plan_mode_complete", "read", "subagent", "todo", "web_fetch", "write"]);
    expect(ready.snapshot?.tools?.every((tool) => tool.source.kind === "builtin" || tool.source.kind === "wackcode")).toBe(true);
    // The denylist from `init` is applied before the first turn, and tools whose external
    // binary is missing are never offered even though they stay listed in the catalogue.
    const catalog = ready.snapshot?.tools ?? [];
    // Sub-agents is the one built-in that is off until the user switches it on.
    const expectedActive = catalog
      .filter((tool) => tool.available && tool.name !== "find" && tool.name !== "subagent")
      .map((tool) => tool.name)
      .sort();
    expect(ready.snapshot?.activeTools?.sort()).toEqual(expectedActive);
    for (const tool of catalog) {
      expect(tool.available || typeof tool.unavailableReason === "string").toBe(true);
    }

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "Create the fixture." });
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);

    const offered = (request: (typeof provider.requests)[number]) =>
      (request.body.tools as Array<{ function: { name: string } }> ?? []).map((tool) => tool.function.name).sort();
    expect(offered(provider.requests[0])).toEqual(expectedActive);

    // Toggling tools takes effect on the next turn with no worker restart. The wackcode
    // tools are exempt from the denylist except web_fetch, so six tools stay active.
    worker.send({ id: crypto.randomUUID(), type: "set_tools", disabledTools: ["bash", "grep", "ls", "find", "web_fetch"] });
    await worker.waitFor((output) => emitted(output) && output.view?.activeTools?.length === 6);
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
      alpha.worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true),
      beta.worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished beta.")) === true)
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

    const savedSession = alpha.worker.view?.sessionFile;
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
    const finalSettled = await worker.waitFor((output) =>
      emitted(output) && output.view?.messages.some((message) =>
        message.blocks.some((block) => block.text === "Streaming the answer in pieces.")));
    const finalIndex = worker.outputs.indexOf(finalSettled);
    expect(firstPartial).toBeGreaterThan(-1);
    expect(firstPartial).toBeLessThan(finalIndex);
  });

  it("sends message boundaries as deltas chained by revision, not whole-session snapshots", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-deltas-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker, ready } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "deltas-task");
    cleanup.push(() => worker.shutdown());
    const readyRev = ready.snapshot?.rev ?? 0;
    expect(readyRev).toBeGreaterThan(0);

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "delta-run", message: "Create the fixture." });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "delta-run");

    // The whole run streamed as deltas: no boundary re-sent the transcript whole.
    const runStart = worker.outputs.findIndex((output) => output.type === "run_state" && output.state === "running" && output.runId === "delta-run");
    const runEnd = worker.outputs.findIndex((output) => output.type === "run_state" && output.state === "idle" && output.runId === "delta-run");
    const duringRun = worker.outputs.slice(runStart, runEnd);
    expect(duringRun.some((output) => output.type === "snapshot_delta")).toBe(true);
    expect(duringRun.some((output) => output.type === "snapshot")).toBe(false);

    // Every emission chains onto the previous by exactly one revision.
    let lastRev = readyRev;
    for (const output of worker.outputs) {
      if (!emitted(output)) continue;
      const rev = output.snapshot?.rev ?? output.delta?.rev ?? 0;
      expect(rev).toBe(lastRev + 1);
      lastRev = rev;
    }

    // Applying the chain reproduces exactly what a fresh full snapshot says.
    worker.send({ id: crypto.randomUUID(), type: "snapshot" });
    const full = await worker.waitFor((output) => output.type === "snapshot" && (output.snapshot?.rev ?? 0) > lastRev);
    expect(worker.view?.messages.map((message) => message.id)).toEqual(full.snapshot?.messages.map((message) => message.id));
    expect(worker.view?.runTimings).toEqual(full.snapshot?.runTimings);
    expect(worker.view?.tree).toEqual(full.snapshot?.tree);
    expect(worker.view?.planState).toEqual(full.snapshot?.planState);
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
      emitted(output) && output.view?.messages.some((message) =>
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
    const stopped = worker.view;
    expect(stopped?.runTimings).toHaveLength(1);
    const user = stopped?.messages.find((message) => message.role === "user");
    expect(stopped?.runTimings?.[0]?.userMessageId).toBe(user?.id);
  });

  it("does not report a stop during a tool call as a failure", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-tool-stop-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "tool-stop-secret", workspace, "tool-stop-task");
    cleanup.push(() => worker.shutdown());
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "tool-stop-run", message: "stream tool" });
    await worker.waitFor((output) => output.type === "activity" && output.event === "tool_execution_update");
    // Pi still starts the next model request after the tool, and it fails on the aborted signal.
    worker.send({ id: crypto.randomUUID(), type: "abort" });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    await worker.waitFor((output) => emitted(output) && output.view?.activeRun === undefined);
    expect(worker.outputs.filter((output) => output.type === "worker_error").map((output) => output.message)).toEqual([]);
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
    await first.worker.waitFor((output) => emitted(output) && output.view?.runTimings?.length === 1);
    await first.worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "timing-run-1");
    const firstDone = first.worker.view;
    const firstUser = firstDone?.messages.find((message) => message.role === "user");
    expect(firstDone?.runTimings?.[0]?.userMessageId).toBe(firstUser?.id);
    expect(firstDone?.runTimings?.[0]?.durationMs).toBeGreaterThanOrEqual(5_000);
    expect(firstDone?.activeRun).toBeUndefined();

    first.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "timing-run-2", startedAt: Date.now() - 2_000, message: "Second turn." });
    await first.worker.waitFor((output) => emitted(output) && output.view?.runTimings?.length === 2);
    await first.worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "timing-run-2");
    const bothDone = first.worker.view;
    const userIds = bothDone?.messages.filter((message) => message.role === "user").map((message) => message.id);
    expect(bothDone?.runTimings?.map((timing) => timing.userMessageId)).toEqual(userIds);

    const sessionFile = bothDone?.sessionFile;
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

  it("clocks a thinking block while it streams and keeps its duration across a restart", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-thinking-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const first = await initializeWorker(provider.baseUrl, "think-secret", workspace, "think-task");
    cleanup.push(() => first.worker.shutdown());
    first.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "think-run", message: "Think first." });
    await first.worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "think-run");

    type Message = SnapshotView["messages"][number];
    const thinkingOf = (message: Message | undefined) => message?.blocks.find((block) => block.type === "thinking");
    const partials = first.worker.outputs
      .filter((output) => output.type === "partial")
      .map((output) => (output as unknown as { message: Message }).message);
    // While the model reasons, the block has no duration yet.
    expect(partials.some((message) => thinkingOf(message) && !message.blocks.some((block) => block.type === "text") && thinkingOf(message)?.durationMs === undefined)).toBe(true);
    // Once the answer starts, the streamed block already carries how long the reasoning took.
    const answering = partials.find((message) => message.blocks.some((block) => block.type === "text" && block.text));
    const streamedDuration = thinkingOf(answering)?.durationMs;
    expect(streamedDuration).toBeGreaterThanOrEqual(300);

    // The saved message keeps the same duration, and the timestamp that ties it to its partial.
    const saved = first.worker.view?.messages.find((message) => message.role === "assistant");
    expect(thinkingOf(saved)?.durationMs).toBe(streamedDuration);
    expect(saved?.timestamp).toBe(answering?.timestamp);

    const sessionFile = first.worker.view?.sessionFile;
    expect(sessionFile).toBeTruthy();
    await first.worker.shutdown();
    const restored = await initializeWorker(provider.baseUrl, "think-secret", workspace, "think-task", sessionFile);
    cleanup.push(() => restored.worker.shutdown());
    const reopened = restored.ready.snapshot?.messages.find((message) => message.role === "assistant");
    expect(thinkingOf(reopened)?.text).toBe("Let me work this out.");
    expect(thinkingOf(reopened)?.durationMs).toBe(streamedDuration);
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
    expect(catalog.find((tool) => tool.name === "web_fetch")?.source.kind).toBe("wackcode");
    expect(ready.snapshot?.activeTools).toContain("ask_user_question");
    expect(ready.snapshot?.activeTools).toContain("plan_mode_complete");
    expect(ready.snapshot?.activeTools).toContain("todo");
    expect(ready.snapshot?.planState?.mode).toBe("build");
  });

  it("offers web_fetch by default, lets the denylist switch it off, and refuses private addresses", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-web-fetch-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker, ready } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "web-fetch-task");
    cleanup.push(() => worker.shutdown());
    expect(ready.snapshot?.activeTools).toContain("web_fetch");

    // The mock provider itself is on loopback: the tool must refuse it without connecting.
    const before = provider.requests.length;
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: `fetch: ${provider.baseUrl}/models` });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && provider.requests.length > before + 1);
    const toolResult = provider.requests[before + 1].body.messages.find(
      (message) => (message as { role?: string }).role === "tool"
    ) as { content?: string } | undefined;
    expect(toolResult?.content).toContain("local or private network address");

    worker.send({ id: crypto.randomUUID(), type: "set_tools", disabledTools: ["web_fetch"] });
    await worker.waitFor((output) => emitted(output) && output.view?.activeTools?.includes("web_fetch") === false);
    expect(worker.child.exitCode).toBeNull();
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
    // Only Ultra Plan's interview offers "Write the plan now".
    expect(request.offerWrapUp).toBeUndefined();

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
    await first.worker.waitFor((output) =>
      emitted(output) &&
      output.view?.messages.some((message) => message.blocks.some((block) => block.toolName === "todo" && block.details !== undefined)) === true
    );
    const details = first.worker.view?.messages
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

  it("lets web_fetch run in Plan mode — it only reads", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-web-fetch-plan-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "web-fetch-plan-task", undefined, undefined, undefined, "plan"
    );
    cleanup.push(() => worker.shutdown());
    await worker.waitFor((output) => output.type === "plan_state" && output.mode === "plan");

    // A private address keeps the test offline; what matters is that the tool itself answered
    // rather than the Plan mode policy.
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "fetch: http://localhost/" });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && provider.requests.length > 1);
    const toolResult = provider.requests[1].body.messages.find(
      (message) => (message as { role?: string }).role === "tool"
    ) as { content?: string } | undefined;
    expect(toolResult?.content).toContain("local or private network address");
    expect(toolResult?.content).not.toContain("Plan mode");
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

  it("keeps Ultra Plan read-only, under its own contract", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-ultra-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "ultra-task", undefined, undefined, undefined, "ultraplan"
    );
    cleanup.push(() => worker.shutdown());
    await worker.waitFor((output) => output.type === "plan_state" && output.mode === "ultraplan");

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "Create the fixture." });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    const context = JSON.stringify(provider.requests[0].body.messages);
    expect(context).toContain("[WACKCODE PLAN MODE CONTRACT v1: ULTRAPLAN]");
    expect(context).not.toContain("[WACKCODE PLAN MODE CONTRACT v1: PLAN]");
    // The same read-only policy as Plan mode.
    const toolResult = provider.requests[1].body.messages.find(
      (message) => (message as { role?: string }).role === "tool"
    ) as { content?: string } | undefined;
    expect(toolResult?.content).toContain("Plan mode");
    await expect(readFile(join(workspace, "alpha.txt"), "utf8")).rejects.toThrow();
  });

  it("restores Ultra Plan and its plan across a restart, and switching to Plan keeps the plan", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-ultra-restore-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));

    const first = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "ultra-restore-task", undefined, undefined, undefined, "ultraplan"
    );
    first.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "finish plan" });
    const submitted = await first.worker.waitFor((output) => output.type === "plan_state" && output.phase === "ready");
    expect(submitted.mode).toBe("ultraplan");
    const sessionFile = first.ready.snapshot?.sessionFile;
    expect(sessionFile).toBeTruthy();
    await first.worker.shutdown();

    const second = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "ultra-restore-task", sessionFile);
    cleanup.push(() => second.worker.shutdown());
    const restored = await second.worker.waitFor((output) => output.type === "plan_state" && output.phase === "ready");
    expect(restored.mode).toBe("ultraplan");
    expect(restored.plan).toBe("# The plan\n\n- Ship it");

    // Plan ↔ Ultra Plan only changes the interview: the plan stays reviewable.
    second.worker.send({ id: crypto.randomUUID(), type: "set_mode", mode: "plan" });
    const switched = await second.worker.waitFor((output) => output.type === "plan_state" && output.mode === "plan");
    expect(switched.phase).toBe("ready");
    expect(switched.plan).toBe("# The plan\n\n- Ship it");

    // …and the next request carries Plan's contract as the latest one.
    const before = provider.requests.length;
    second.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-2", message: "One more pass." });
    await second.worker.waitFor((output) => output.type === "run_state" && output.runId === "run-2" && output.state === "idle");
    const latest = JSON.stringify(provider.requests[before].body.messages);
    expect(latest.lastIndexOf("CONTRACT v1: ULTRAPLAN]")).toBeGreaterThan(-1);
    expect(latest.lastIndexOf("CONTRACT v1: PLAN]")).toBeGreaterThan(latest.lastIndexOf("CONTRACT v1: ULTRAPLAN]"));
  });

  it("offers wrap-up on Ultra Plan questions, and pressing it steers the model to submit the plan", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-ultra-ask-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "ultra-ask-task", undefined, undefined, undefined, "ultraplan"
    );
    cleanup.push(() => worker.shutdown());

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "ask: which engine?" });
    const request = await worker.waitFor((output) => output.type === "extension_ui_request" && output.method === "questions");
    expect(request.offerWrapUp).toBe(true);
    worker.send({ id: crypto.randomUUID(), type: "extension_ui_response", requestId: request.requestId, wrapUp: true });

    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    const toolResult = provider.requests[1].body.messages.find(
      (message) => (message as { role?: string }).role === "tool"
    ) as { content?: string } | undefined;
    expect(toolResult?.content).toContain("stop answering questions");
    expect(toolResult?.content).toContain("plan_mode_complete");
  });
});

describe("custom prompts (Settings → Prompts)", () => {
  const messagesOf = (source: MockProvider, index: number) =>
    (source.requests[index].body.messages ?? []) as { role?: string; content?: unknown }[];
  // The leading prompt message: Pi maps it to "developer" for the completions API.
  const systemPromptOf = (source: MockProvider) => {
    const message = messagesOf(source, 0).find((entry) => entry.role === "system" || entry.role === "developer");
    return typeof message?.content === "string" ? message.content : "";
  };
  // Content parts of every message, joined, so contract text can be matched with real newlines.
  const messageTextOf = (source: MockProvider, index: number) =>
    messagesOf(source, index)
      .map((message) => message.content)
      .flatMap((content) => (Array.isArray(content) ? content : []))
      .map((part) => (part as { text?: string })?.text ?? "")
      .join("\n");

  it("replaces only the system prompt's persona from init, keeping the assembled sections", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-persona-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "persona-task", undefined, undefined, undefined, undefined, undefined,
      { prompts: { systemPrompt: "You are WackTester, a bespoke persona." } }
    );
    cleanup.push(() => worker.shutdown());

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "Create the fixture." });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    const prompt = systemPromptOf(provider);
    expect(prompt).toContain("You are WackTester, a bespoke persona.");
    expect(prompt).not.toContain("expert coding assistant");
    // Only the persona prefix was replaced: the assembled prompt still carries the rest.
    expect(prompt).toContain(workspace);
  });

  it("sends exactly the persona Settings → Prompts shows as the default", async () => {
    // Pin for the display copy: Pi owns this text, so when this fails after a Pi upgrade,
    // update src/promptDefaults.ts to match what the provider actually receives.
    const { DEFAULT_SYSTEM_PROMPT } = await import("../../src/promptDefaults.js");
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-persona-default-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "persona-default-task");
    cleanup.push(() => worker.shutdown());

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "Create the fixture." });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    // The persona is the preamble: rendered first, then the assembled sections follow.
    expect(systemPromptOf(provider).startsWith(`${DEFAULT_SYSTEM_PROMPT}\n\n`)).toBe(true);
  });

  it("uses a custom Plan contract body, and a live edit reaches the next message without a restart", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-custom-plan-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "custom-plan-task", undefined, undefined, undefined, "plan", undefined,
      { prompts: { planPrompt: "Custom plan rules, first take." } }
    );
    cleanup.push(() => worker.shutdown());
    await worker.waitFor((output) => output.type === "plan_state" && output.mode === "plan");

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "Create the fixture." });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    expect(messageTextOf(provider, 0)).toContain("[WACKCODE PLAN MODE CONTRACT v1: PLAN]\nCustom plan rules, first take.");
    expect(messageTextOf(provider, 0)).not.toContain("## Mode rules");

    // Settings change mid-session: queued like set_tools, applied by the next turn. The stale
    // contract stays in the transcript and the updated one is appended after it.
    const editId = crypto.randomUUID();
    worker.send({ id: editId, type: "set_prompts", prompts: { planPrompt: "Custom plan rules, second take." } });
    await worker.waitFor((output) => output.type === "response" && output.id === editId && output.success);

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-2", message: "Another pass." });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "run-2" && output.state === "idle");
    const after = messageTextOf(provider, provider.requests.length - 1);
    expect(after).toContain("Custom plan rules, second take.");
    expect(after.lastIndexOf("second take.")).toBeGreaterThan(after.lastIndexOf("first take."));
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

    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) =>
      message.blocks.some((block) => block.type === "image" && block.thumbnail?.startsWith("data:image/"))) === true);
    const user = worker.view?.messages.find((message) => message.role === "user");
    const image = user?.blocks.find((block) => block.type === "image");
    expect(image?.imageId).toBeTruthy();
    // The preview lands as one targeted upsert, not another whole-session snapshot.
    const thumbnailDelta = worker.outputs.find((output) =>
      output.type === "snapshot_delta" && output.delta?.upserts.some((upsert) =>
        upsert.blocks.some((block) => block.type === "image" && Boolean(block.thumbnail))));
    expect(thumbnailDelta?.delta?.upserts).toHaveLength(1);
    // Snapshots carry the preview only, never the multi-kilobyte original.
    expect(JSON.stringify(worker.view)).not.toContain(original);
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
    const sessionFile = first.worker.view?.sessionFile;
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
    await second.worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) =>
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

describe("/init", () => {
  it("writes workspace instructions and reloads them for the next prompt", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-init-context-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    await writeFile(join(workspace, "package.json"), '{"scripts":{"test":"vitest run"}}\n');
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "init-context-task");
    cleanup.push(() => worker.shutdown());

    const id = crypto.randomUUID();
    worker.send({ id, type: "init_agents", runId: "init-run", checkpoint: null });
    expect((await worker.waitFor((output) => output.type === "response" && output.id === id)).success).toBe(true);
    await worker.waitFor((output) => output.type === "extension_notice" && output.message?.startsWith("Created AGENTS.md") === true);
    expect(await readFile(join(workspace, "AGENTS.md"), "utf8")).toContain("Run `pnpm test`");

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "after-init", message: "Next task" });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "after-init");
    const nextRequest = provider.requests.find((request) => request.text === "Next task");
    expect(JSON.stringify(nextRequest?.body.messages)).toContain("# Fixture guidance");
  });

  it("leaves sufficient existing instructions untouched", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-init-existing-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const content = "# ALREADY_USEFUL_RULE\n\nRun `pnpm test`.\n";
    await writeFile(join(workspace, "AGENTS.md"), content);
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "init-existing-task");
    cleanup.push(() => worker.shutdown());

    const id = crypto.randomUUID();
    worker.send({ id, type: "init_agents", runId: "init-existing" });
    expect((await worker.waitFor((output) => output.type === "response" && output.id === id)).success).toBe(true);
    await worker.waitFor((output) => output.type === "extension_notice" && output.message?.startsWith("Left unchanged AGENTS.md") === true);
    expect(await readFile(join(workspace, "AGENTS.md"), "utf8")).toBe(content);
  });

  it("refuses a root override before contacting the model", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-init-override-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    await writeFile(join(workspace, "AGENTS.override.md"), "# Keep the override\n");
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "init-override-task");
    cleanup.push(() => worker.shutdown());

    const id = crypto.randomUUID();
    worker.send({ id, type: "init_agents", runId: "init-refused" });
    const response = await worker.waitFor((output) => output.type === "response" && output.id === id);
    expect(response.success).toBe(false);
    expect(response.error).toContain("Pi would ignore AGENTS.md");
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "init-refused" && output.state === "idle");
    expect(provider.requests).toHaveLength(0);
  });

  it.each(["plan", "ultraplan"] as const)("refuses to write while %s mode is active", async (mode) => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-init-plan-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, `init-${mode}-task`, undefined, undefined, undefined, mode);
    cleanup.push(() => worker.shutdown());

    const id = crypto.randomUUID();
    worker.send({ id, type: "init_agents", runId: "init-plan" });
    const response = await worker.waitFor((output) => output.type === "response" && output.id === id);
    expect(response.success).toBe(false);
    expect(response.error).toContain("Build mode");
    expect(provider.requests).toHaveLength(0);
  });
});

describe("session tree", () => {
  const TREE_A = "a".repeat(40);
  const TREE_B = "b".repeat(40);
  const TREE_C = "c".repeat(40);

  async function settle(worker: WorkerHarness, runId: string): Promise<SnapshotView> {
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === runId);
    if (!worker.view) throw new Error("No snapshot after the run");
    return worker.view;
  }

  async function request(worker: WorkerHarness, command: Record<string, unknown>): Promise<Output> {
    const id = crypto.randomUUID();
    worker.send({ id, ...command });
    return worker.waitFor((output) => output.type === "response" && output.id === id);
  }

  function users(snapshot: SnapshotView | undefined) {
    return (snapshot?.messages ?? []).filter((message) => message.role === "user");
  }

  function lastUserText(provider: MockProvider): string {
    return userParts(provider.requests[provider.requests.length - 1]).filter((part) => part.type === "text").map((part) => part.text).join("");
  }

  it("retries a prompt as a new version, records checkpoints, and switches back to the first", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-retry-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "retry-task");
    cleanup.push(() => worker.shutdown());

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "retry-1", message: "Make alpha.", checkpoint: { id: TREE_A, head: "f".repeat(40) } });
    const first = await settle(worker, "retry-1");
    const [original] = users(first);
    expect(original.entryId).toBeTruthy();
    expect(original.id).toBe(original.entryId);
    expect(original.checkpoint).toEqual({ id: TREE_A, head: "f".repeat(40) });
    expect(original.versions).toBeUndefined();
    const answer = first.messages.filter((message) => message.role === "assistant").at(-1);
    expect(answer?.turn?.userEntryId).toBe(original.entryId);
    expect(provider.requests).toHaveLength(2);

    const resent = await request(worker, {
      type: "resend", runId: "retry-2", entryId: original.entryId,
      leave: { id: TREE_B }, checkpoint: { id: TREE_C }
    });
    expect(resent.success).toBe(true);
    const second = await settle(worker, "retry-2");
    const [retried] = users(second);
    expect(retried.entryId).not.toBe(original.entryId);
    expect(retried.versions).toMatchObject({ index: 1, total: 2, previous: original.entryId });
    expect(retried.checkpoint).toEqual({ id: TREE_C });
    // The retry asked the model again, with the same prompt and without the first answer.
    expect(provider.requests).toHaveLength(4);
    expect(lastUserText(provider)).toBe("Make alpha.");
    expect(second.messages.filter((message) => message.role === "user")).toHaveLength(1);

    const switched = await request(worker, { type: "navigate", entryId: original.entryId, target: "latest", kind: "switch", leave: null });
    expect(switched.success).toBe(true);
    // The first version was left with TREE_B when it was retried.
    expect(switched.result?.files).toEqual({ id: TREE_B });
    const back = worker.view;
    const [shown] = users(back);
    expect(shown.entryId).toBe(original.entryId);
    expect(shown.versions).toMatchObject({ index: 0, total: 2, next: retried.entryId });
    expect(back?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha."))).toBe(true);
    expect(provider.requests).toHaveLength(4);
  });

  it("edits a message, rewinds it into the composer, and keeps the rewind across a restart", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-rewind-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const first = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "rewind-task");
    cleanup.push(() => first.worker.shutdown());

    first.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "rewind-1", message: "Original request." });
    const [original] = users(await settle(first.worker, "rewind-1"));
    const edited = await request(first.worker, { type: "resend", runId: "rewind-2", entryId: original.entryId, message: "Edited request." });
    expect(edited.success).toBe(true);
    const afterEdit = await settle(first.worker, "rewind-2");
    expect(lastUserText(provider)).toBe("Edited request.");
    const [editedMessage] = users(afterEdit);
    expect(editedMessage.blocks.find((block) => block.type === "text")?.text).toBe("Edited request.");
    expect(editedMessage.versions).toMatchObject({ index: 1, total: 2 });

    const rewound = await request(first.worker, { type: "navigate", entryId: editedMessage.entryId, target: "before", kind: "rewind" });
    expect(rewound.success).toBe(true);
    expect(rewound.result?.editorText).toBe("Edited request.");
    const empty = first.worker.view;
    expect(empty).toBeDefined();
    expect(users(empty)).toHaveLength(0);
    expect(empty?.tree?.undo).toBeTruthy();
    const sessionFile = empty?.sessionFile as string;
    const requestsBefore = provider.requests.length;
    await first.worker.shutdown();

    // Pi reopens a session at its last line; the navigation marker makes that the rewound point.
    const restored = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "rewind-task", sessionFile);
    cleanup.push(() => restored.worker.shutdown());
    expect(users(restored.ready.snapshot!)).toHaveLength(0);
    expect(restored.ready.snapshot?.tree?.undo).toBe(empty?.tree?.undo);
    expect(provider.requests).toHaveLength(requestsBefore);

    const undone = await request(restored.worker, { type: "navigate", entryId: empty?.tree?.undo, target: "latest", kind: "undo" });
    expect(undone.success).toBe(true);
    const back = restored.worker.view;
    expect(users(back).map((message) => message.entryId)).toEqual([editedMessage.entryId]);
    expect(back?.tree?.undo).toBeUndefined();
  });

  it("re-derives Plan mode and keeps the tool denylist after moving in the tree", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-tree-plan-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "tree-plan-task", undefined, ["bash"]);
    cleanup.push(() => worker.shutdown());

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "plan-1", message: "Build first.", mode: "build" });
    const [first] = users(await settle(worker, "plan-1"));
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "plan-2", message: "Now plan.", mode: "plan" });
    const planned = await settle(worker, "plan-2");
    expect(planned.planState?.mode).toBe("plan");

    const rewound = await request(worker, { type: "navigate", entryId: first.entryId, target: "before", kind: "rewind" });
    expect(rewound.success).toBe(true);
    const snapshot = worker.view;
    expect(snapshot?.planState?.mode).toBe("build");
    expect([...worker.outputs].reverse().find((output) => output.type === "plan_state")?.mode).toBe("build");
    expect(snapshot?.activeTools).not.toContain("bash");
    expect(snapshot?.activeTools).toContain("read");
  });

  it("refuses to navigate or resend unknown messages without raising a worker error", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-tree-refuse-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "tree-refuse-task");
    cleanup.push(() => worker.shutdown());

    const navigated = await request(worker, { type: "navigate", entryId: "missing", target: "latest", kind: "switch" });
    expect(navigated.success).toBe(false);
    const resent = await request(worker, { type: "resend", runId: "never", entryId: "missing" });
    expect(resent.success).toBe(false);
    expect(resent.error).toMatch(/no longer in this chat/);
    expect(worker.outputs.some((output) => output.type === "worker_error")).toBe(false);
  });

  it("forks a chat at the end of a turn into a new session", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-fork-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const source = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "fork-source");
    cleanup.push(() => source.worker.shutdown());
    source.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "fork-1", message: "First turn." });
    await settle(source.worker, "fork-1");
    source.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "fork-2", message: "Second turn." });
    const both = await settle(source.worker, "fork-2");
    const firstTurn = both.messages.find((message) => message.turn?.userEntryId === users(both)[0].entryId)?.turn;
    expect(firstTurn?.endEntryId).toBeTruthy();
    const requestsBefore = provider.requests.length;

    const fork = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "fork-copy", undefined, undefined, undefined, undefined, undefined, {
      forkFrom: { sessionFile: both.sessionFile, entryId: firstTurn?.endEntryId }
    });
    cleanup.push(() => fork.worker.shutdown());
    const forked = fork.ready.snapshot!;
    expect(users(forked).map((message) => message.blocks[0]?.text)).toEqual(["First turn."]);
    expect(forked.sessionFile).toContain(join(".sessions", "fork-copy"));
    expect(forked.runTimings).toHaveLength(1);
    const header = JSON.parse((await readFile(forked.sessionFile as string, "utf8")).split("\n")[0]) as { parentSession?: string };
    expect(header.parentSession).toBe(both.sessionFile);
    expect(provider.requests).toHaveLength(requestsBefore);
  });

  it("refuses to resend images to a model without vision, and resends without them", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-resend-images-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const vision = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "images-task", undefined, undefined, undefined, undefined, true);
    cleanup.push(() => vision.worker.shutdown());
    vision.worker.send({
      id: crypto.randomUUID(), type: "prompt", runId: "images-1", message: "Look at this.",
      images: [{ type: "image", data: solidPng(4, 4), mimeType: "image/png" }]
    });
    const snapshot = await settle(vision.worker, "images-1");
    const [original] = users(snapshot);
    await vision.worker.shutdown();

    const textOnly = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "images-task", snapshot.sessionFile, undefined, undefined, undefined, false);
    cleanup.push(() => textOnly.worker.shutdown());
    const refused = await request(textOnly.worker, { type: "resend", runId: "images-2", entryId: original.entryId });
    expect(refused.success).toBe(false);
    expect(refused.error).toMatch(/doesn't accept images/);
    expect(textOnly.worker.outputs.some((output) => output.type === "worker_error")).toBe(false);

    const withoutImages = await request(textOnly.worker, { type: "resend", runId: "images-3", entryId: original.entryId, removeImages: [0] });
    expect(withoutImages.success).toBe(true);
    await settle(textOnly.worker, "images-3");
    expect(userParts(provider.requests[provider.requests.length - 1]).some((part) => part.type === "image_url")).toBe(false);
  });

  it("reports a prompt an extension command handled as idle", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-command-idle-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const extension = join(workspace, "hello.ts");
    await writeFile(extension, `export default function (pi: any) {
      pi.registerCommand("hello", { description: "Say hello", handler: async () => undefined });
      pi.registerCommand("new", { description: "Package new", handler: async () => undefined });
      pi.registerCommand("init", { description: "Package init", handler: async () => undefined });
      pi.registerCommand("review", { description: "Extension review", handler: async () => undefined });
      pi.registerCommand("try-new-session", { description: "Unsupported session action", handler: async (_args: string, ctx: any) => { await ctx.newSession(); } });
      pi.registerCommand("ask-command", { description: "Ask through desktop UI", handler: async (_args: string, ctx: any) => { const answer = await ctx.ui.select("Choose", ["a", "b"]); ctx.ui.notify("picked:" + answer); } });
    }\n`);
    const prompt = join(workspace, "review.md");
    await writeFile(prompt, "---\ndescription: Review this work\n---\nReview $ARGUMENTS\n");
    const skill = join(workspace, "skill");
    await mkdir(skill);
    await writeFile(join(skill, "SKILL.md"), "---\nname: fixture\ndescription: Fixture skill\n---\nRead the fixture.\n");
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "command-task", undefined, undefined,
      { extensions: [extension], skills: [skill], prompts: [prompt], themes: [] });
    cleanup.push(() => worker.shutdown());
    const tooSmall = await request(worker, { type: "compact", runId: "compact-small", instructions: "" });
    expect(tooSmall.success).toBe(false);
    expect(tooSmall.error).toContain("not enough conversation");
    const listed = await request(worker, { type: "list_commands" });
    expect(listed.success).toBe(true);
    const commands = listed.result as unknown as Array<{ id: string; name: string; source: string }>;
    expect(commands).toEqual(expect.arrayContaining([
      expect.objectContaining({ name: "hello", source: "extension" }),
      expect.objectContaining({ name: "extension:new", source: "extension" }),
      expect.objectContaining({ name: "extension:init", source: "extension" }),
      expect.objectContaining({ name: "prompt:review", source: "prompt" }),
      expect.objectContaining({ name: "skill:fixture", source: "skill" })
    ]));
    const stale = await request(worker, { type: "execute_command", commandId: "missing", args: "", runId: "stale" });
    expect(stale.success).toBe(false);
    const hello = commands.find((entry) => entry.name === "hello")!;
    const accepted = await request(worker, { type: "execute_command", commandId: hello.id, args: "", runId: "command-1" });
    expect(accepted.success).toBe(true);
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "command-1");
    expect(provider.requests).toHaveLength(0);
    const unsupported = commands.find((entry) => entry.name === "try-new-session")!;
    expect((await request(worker, { type: "execute_command", commandId: unsupported.id, args: "", runId: "session-action" })).success).toBe(true);
    await worker.waitFor((output) => output.type === "extension_notice" && output.message?.includes("cannot create") === true);
    await settle(worker, "session-action");
    expect(provider.requests).toHaveLength(0);
    const asking = commands.find((entry) => entry.name === "ask-command")!;
    expect((await request(worker, { type: "execute_command", commandId: asking.id, args: "", runId: "dialog-command" })).success).toBe(true);
    const dialog = await worker.waitFor((output) => output.type === "extension_ui_request" && output.method === "select" && output.title === "Choose");
    worker.send({ id: crypto.randomUUID(), type: "extension_ui_response", requestId: dialog.requestId, value: "b" });
    await worker.waitFor((output) => output.type === "extension_notice" && output.message === "picked:b");
    await settle(worker, "dialog-command");
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "literal-1", message: "/not-a-command", literal: true });
    await settle(worker, "literal-1");
    expect(provider.requests.length).toBeGreaterThan(0);
    expect(lastUserText(provider)).toContain("/not-a-command");
    const template = commands.find((entry) => entry.name === "prompt:review")!;
    expect((await request(worker, { type: "execute_command", commandId: template.id, args: '"changed files"', runId: "template-1" })).success).toBe(true);
    await settle(worker, "template-1");
    expect(lastUserText(provider)).toContain("Review changed files");
    const selectedSkill = commands.find((entry) => entry.name === "skill:fixture")!;
    expect((await request(worker, { type: "execute_command", commandId: selectedSkill.id, args: "details", runId: "skill-1" })).success).toBe(true);
    await settle(worker, "skill-1");
    expect(lastUserText(provider)).toContain("<skill name=\"fixture\"");
  });
});

describe("sub-agents", () => {
  const scout = {
    name: "scout",
    description: "Looks around without changing anything",
    prompt: "SCOUT-PROMPT-MARKER. Report what you find.",
    tools: ["read", "grep", "find", "ls", "bash"],
    readOnly: true
  };
  const editor = {
    name: "worker",
    description: "Implements a scoped task",
    prompt: "WORKER-PROMPT-MARKER. Make the change.",
    tools: ["read", "ls", "bash", "edit", "write"],
    readOnly: false
  };
  const config = (overrides: Record<string, unknown> = {}) => ({
    trigger: "on_request",
    maxConcurrency: 4,
    agents: [scout, editor],
    providers: [],
    ...overrides
  });

  type ToolResultBlock = { type: string; toolName?: string; text?: string; details?: unknown; isError?: boolean };
  type Details = {
    v: number;
    mode: string;
    results: Array<{ agent: string; task: string; status: string; model?: string; output?: string; error?: string; activity: Array<{ tool: string; subject: string }>; usage: { input: number; output: number; turns: number } }>;
  };

  function subagentResult(worker: WorkerHarness): (ToolResultBlock & { details: Details }) | undefined {
    const block = worker.view?.messages
      .flatMap((message) => message.blocks as ToolResultBlock[])
      .find((candidate) => candidate.type === "tool-result" && candidate.toolName === "subagent");
    return block as (ToolResultBlock & { details: Details }) | undefined;
  }

  const childRequests = (provider: MockProvider, marker: string) =>
    provider.requests.filter((request) => JSON.stringify(request.body.messages).includes(marker));
  const offered = (request: { body: Record<string, unknown> }) =>
    (request.body.tools as Array<{ function: { name: string } }> ?? []).map((tool) => tool.function.name).sort();

  async function start(taskId: string, extra: Record<string, unknown>, mode?: TaskMode, apiKey = "alpha-secret") {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), `wackcode-${taskId}-`));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker, ready } = await initializeWorker(provider.baseUrl, apiKey, workspace, taskId, undefined, undefined, undefined, mode, undefined, extra);
    cleanup.push(() => worker.shutdown());
    return { provider, workspace, worker, ready };
  }

  it("keeps the tool off until switched on, and applies settings live without a restart", async () => {
    const { worker, ready } = await start("subagents-toggle", {});
    expect(ready.snapshot?.tools?.find((tool) => tool.name === "subagent")?.source.kind).toBe("wackcode");
    expect(ready.snapshot?.activeTools).not.toContain("subagent");

    worker.send({ id: crypto.randomUUID(), type: "set_subagents", subagents: config() });
    const on = await worker.waitFor((output) => output.type === "snapshot" && output.snapshot?.activeTools?.includes("subagent") === true);
    const description = on.snapshot?.tools?.find((tool) => tool.name === "subagent")?.description ?? "";
    expect(description).toContain("scout (read-only)");
    expect(description).toContain("worker (can edit files)");

    worker.send({ id: crypto.randomUUID(), type: "set_subagents", subagents: null });
    await worker.waitFor((output) => output.type === "snapshot" && output !== on && output.snapshot?.activeTools?.includes("subagent") === false);
    expect(worker.child.exitCode).toBeNull();
  });

  it("gives a read-only child web_fetch while Web Fetch is on, under the same address policy", async () => {
    const webScout = { ...scout, tools: [...scout.tools, "web_fetch"] };
    const { provider, worker } = await start("subagents-fetch", { subagents: config({ agents: [webScout, editor] }) });

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent fetch" });
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);
    const child = subagentResult(worker)?.details.results[0];
    expect(child?.status).toBe("done");
    expect(child?.activity).toEqual([{ tool: "web_fetch", subject: "http://localhost/docs" }]);

    // The child was offered the tool, the read-only guard let it run, and the fetch itself
    // refused the private address.
    const requests = childRequests(provider, "SCOUT-PROMPT-MARKER");
    expect(offered(requests[0])).toContain("web_fetch");
    const toolResult = requests[1].body.messages.find(
      (message) => (message as { role?: string }).role === "tool"
    ) as { content?: string } | undefined;
    expect(toolResult?.content).toContain("local or private network address");
  });

  it("keeps web_fetch from children while Web Fetch is switched off", async () => {
    const webScout = { ...scout, tools: [...scout.tools, "web_fetch"] };
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-subagents-no-fetch-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "subagents-no-fetch", undefined, ["web_fetch"], undefined, undefined, undefined,
      { subagents: config({ agents: [webScout, editor] }) }
    );
    cleanup.push(() => worker.shutdown());

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent fetch" });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && childRequests(provider, "SCOUT-PROMPT-MARKER").length > 0);
    const requests = childRequests(provider, "SCOUT-PROMPT-MARKER");
    expect(offered(requests[0])).toContain("read");
    expect(offered(requests[0])).not.toContain("web_fetch");
  });

  it("runs a sub-agent in its own in-process session and records its card and usage", async () => {
    const { provider, worker, ready } = await start("subagents-single", { subagents: config() });
    expect(ready.snapshot?.activeTools).toContain("subagent");

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent single" });
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);

    // The live card arrived as structured progress before the final result.
    const live = worker.outputs.find((output) => output.type === "activity" && output.event === "tool_execution_update"
      && (output.detail?.details as Details | undefined)?.v === 1);
    expect(live).toBeDefined();

    const result = subagentResult(worker);
    expect(result?.isError).toBeFalsy();
    const child = result?.details.results[0];
    expect(child?.status).toBe("done");
    expect(child?.model).toBe("Provider subagents-single · Shared model");
    expect(child?.output).toBe("Child done: child-ls: look around");
    expect(child?.activity).toEqual([{ tool: "ls", subject: "." }]);
    expect(child?.usage.turns).toBe(2);
    expect(result?.text).toBe("Child done: child-ls: look around");

    // The child ran with its own prompt and only its role's tools, never the parent's built-ins.
    const requests = childRequests(provider, "SCOUT-PROMPT-MARKER");
    expect(requests).toHaveLength(2);
    for (const request of requests) {
      expect(request.authorization).toBe("Bearer alpha-secret");
      const tools = offered(request);
      expect(tools).toContain("ls");
      for (const hidden of ["edit", "write", "subagent", "todo", "ask_user_question", "plan_mode_complete"]) {
        expect(tools).not.toContain(hidden);
      }
    }
    expect(provider.requests.filter((request) => request.text === "subagent single")).toHaveLength(2);

    // The children's usage is part of the chat's totals: two parent requests and two child ones.
    expect(worker.view?.stats?.tokens.input).toBeGreaterThanOrEqual(96);
  });

  it("runs read-only sub-agents in parallel while those that edit take turns", async () => {
    const { provider, workspace, worker } = await start("subagents-parallel", { subagents: config() });
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent parallel" });
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true, 20_000);

    const result = subagentResult(worker);
    expect(result?.details.mode).toBe("parallel");
    expect(result?.details.results.map((child) => [child.agent, child.status])).toEqual([
      ["scout", "done"], ["worker", "done"], ["scout", "done"], ["worker", "done"]
    ]);
    expect(result?.text).toContain("4/4 sub-agents completed.");
    expect(result?.text).toContain("### [worker] completed");
    expect(await readFile(join(workspace, "one.txt"), "utf8")).toBe("from a sub-agent\n");
    expect(await readFile(join(workspace, "two.txt"), "utf8")).toBe("from a sub-agent\n");

    const window = (text: string) => {
      const times = provider.requests.filter((request) => request.text === text).map((request) => request.at);
      return { first: Math.min(...times), last: Math.max(...times) };
    };
    // Both scouts were in flight together.
    const first = window("child-ls: first");
    const second = window("child-ls: second");
    expect(second.first).toBeLessThan(first.last);
    expect(first.first).toBeLessThan(second.last);
    // The two editors never overlapped.
    const one = window("child-write: one");
    const two = window("child-write: two");
    expect(one.last <= two.first || two.last <= one.first).toBe(true);
  });

  it("blocks a read-only sub-agent's mutating command, and refuses editing agents in Plan mode", async () => {
    const { provider, workspace, worker } = await start("subagents-guard", { subagents: config() });
    await writeFile(join(workspace, "keep.txt"), "still here\n");
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent guard" });
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);
    expect(await readFile(join(workspace, "keep.txt"), "utf8")).toBe("still here\n");
    const answered = childRequests(provider, "SCOUT-PROMPT-MARKER").at(-1);
    expect(JSON.stringify(answered?.body.messages)).toContain("This sub-agent is read-only");
    await worker.shutdown();

    const planned = await start("subagents-plan", { subagents: config() }, "plan");
    planned.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent plan" });
    await planned.worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);
    const refused = subagentResult(planned.worker);
    expect(refused?.isError).toBe(true);
    // Refused by the tool itself, not by Plan mode's package-tool rule: the helper exemption held.
    expect(refused?.text).toContain("Plan mode only runs read-only sub-agents, and worker can edit files");
    expect(childRequests(planned.provider, "WORKER-PROMPT-MARKER")).toHaveLength(0);
    await planned.worker.shutdown();

    // Ultra Plan is Plan mode too.
    const ultra = await start("subagents-ultra", { subagents: config() }, "ultraplan");
    ultra.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent plan" });
    await ultra.worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);
    expect(subagentResult(ultra.worker)?.text).toContain("Plan mode only runs read-only sub-agents, and worker can edit files");
    expect(childRequests(ultra.provider, "WORKER-PROMPT-MARKER")).toHaveLength(0);
  });

  it("uses a sub-agent's own connection and key when its model lives elsewhere", async () => {
    const second = await startMockProvider();
    cleanup.push(second.close);
    const other = {
      provider: {
        id: "second-connection",
        name: "Second connection",
        kind: "custom",
        baseUrl: second.baseUrl,
        api: "openai-completions",
        models: [{ id: "small-model", name: "Small model", contextWindow: 8_192, maxTokens: 256, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null } }]
      },
      apiKey: "second-secret"
    };
    const agents = [{ ...scout, model: { providerId: "second-connection", modelId: "small-model", thinkingLevel: "off" } }];
    const { provider, worker } = await start("subagents-connection", { subagents: config({ agents, providers: [other] }) });
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent single" });
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);

    expect(childRequests(provider, "SCOUT-PROMPT-MARKER")).toHaveLength(0);
    const requests = childRequests(second, "SCOUT-PROMPT-MARKER");
    expect(requests).toHaveLength(2);
    expect(requests.every((request) => request.authorization === "Bearer second-secret" && request.body.model === "small-model")).toBe(true);
    expect(subagentResult(worker)?.details.results[0].model).toBe("Second connection · Small model");
  });

  it("stops running sub-agents with the chat, and still records what they did", async () => {
    const { provider, worker } = await start("subagents-abort", { subagents: config() });
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent wait" });
    await provider.waitForSlowRequest();
    const stoppedAt = Date.now();
    worker.send({ id: crypto.randomUUID(), type: "abort" });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    expect(Date.now() - stoppedAt).toBeLessThan(5_000);
    expect(worker.outputs.filter((output) => output.type === "worker_error").map((output) => output.message)).toEqual([]);
    await worker.waitFor((output) => emitted(output) && subagentResult(worker) !== undefined);
    expect(subagentResult(worker)?.details.results[0].status).toBe("aborted");
    expect(worker.child.exitCode).toBeNull();
  });
});
