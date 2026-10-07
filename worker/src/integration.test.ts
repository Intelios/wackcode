import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { deflateSync } from "node:zlib";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { afterEach, describe, expect, it } from "vitest";
import { readSavedSession } from "./saved-session.js";
import type { ExecutionPolicyConfig, SessionSnapshot, TaskMode } from "./protocol.js";

interface Checkpoint { id: string; head?: string }

type SnapshotView = {
  sessionFile?: string;
  messages: Array<{
    id?: string;
    role?: string;
    timestamp?: number;
    blocks: Array<{
      type: string; text?: string; toolName?: string; toolCallId?: string; details?: unknown; imageId?: string; thumbnail?: string; mimeType?: string;
      durationMs?: number; startedAt?: number; images?: Array<{ imageId: string; thumbnail?: string }>;
    }>;
    entryId?: string;
    versions?: { index: number; total: number; previous?: string; next?: string; group: string };
    checkpoint?: Checkpoint;
    commandPresentation?: {
      id: string;
      name: string;
      arguments: string;
      kind: "command" | "goal-continuation" | "goal-resume";
      round?: number;
      nextAction?: string;
    };
    turn?: { userEntryId: string; endEntryId: string; after?: Checkpoint };
    compaction?: { summary: string; tokensBefore: number; estimatedTokensAfter?: number };
  }>;
  modelSwitches?: Array<{
    id: string;
    at: number;
    from: { providerId: string; modelId: string };
    to: { providerId: string; modelId: string };
  }>;
  modelMissing?: boolean;
  modelIssue?: string;
  tree?: { leafId: string | null; undo?: string };
  runTimings?: Array<{ userMessageId: string; durationMs: number }>;
  activeRun?: { runId: string; startedAt: number };
  workActivity?: SessionSnapshot["workActivity"];
  compaction?: SessionSnapshot["compaction"];
  tools?: Array<{ name: string; description: string; source: { kind: string; packageId?: string }; available: boolean; unavailableReason?: string }>;
  activeTools?: string[];
  executionPolicy?: ExecutionPolicyConfig;
  planState?: { mode: string; phase: string; plan?: string };
  todoState?: { tasks: Array<{ id: number; subject: string; status: string }> };
  goalState?: {
    objective: string; phase: string; iteration: number; maxIterations: number; noProgress: number;
    lastReason?: string; lastNextAction?: string; note?: string;
  };
  skillCreator?: { draftId: string; name: string; revision?: string };
  stats?: { tokens: { input: number; output: number; total: number }; cost: number; contextUsage?: SessionSnapshot["stats"]["contextUsage"]; contextBreakdown?: SessionSnapshot["stats"]["contextBreakdown"] };
};

async function expectSavedHistory(view: SnapshotView, mode: TaskMode = "build") {
  const history = await readSavedSession({ sessionFile: view.sessionFile, taskId: "offline-history", mode, thinkingLevel: "off", contextWindow: view.stats?.contextUsage?.contextWindow });
  expect(history.messages).toEqual(view.messages);
  expect(history.tree).toEqual(view.tree);
  expect(history.runTimings).toEqual(view.runTimings ?? []);
  expect(history.modelSwitches).toEqual(view.modelSwitches ?? []);
  expect(history.stats.tokens).toEqual(view.stats?.tokens);
  expect(history.stats.cost).toEqual(view.stats?.cost);
  expect(history.stats.contextUsage).toEqual(view.stats?.contextUsage);
  expect(history.stats.contextBreakdown).toEqual(view.stats?.contextBreakdown);
  expect(history.planState).toEqual(view.planState);
  expect(history.todoState).toEqual(view.todoState);
  expect(history.goalState).toEqual(view.goalState);
}

interface Output {
  record?: import("./usage.js").UsageRecord;
  type: string;
  attemptId?: string;
  title?: string;
  id?: string;
  success?: boolean;
  error?: string;
  result?: { leafId?: string | null; editorText?: string; files?: Checkpoint; steering?: string[]; followUp?: string[] };
  taskId?: string;
  runId?: string;
  startedAt?: number;
  operation?: "compaction";
  workActivity?: SessionSnapshot["workActivity"];
  state?: string;
  messages?: Array<{ id: string; text: string }>;
  message?: string;
  event?: string;
  detail?: { toolCallId?: string; toolName?: string; text?: string; details?: unknown };
  requestId?: string;
  /** `computer_request` / `browser_request` payloads. */
  request?: Record<string, unknown>;
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
  goal?: SnapshotView["goalState"] | null;
  errors?: Array<{ path: string; error: string }>;
  snapshot?: SnapshotView;
  /** Harness-attached: the merged transcript state as of this output (not from the worker). */
  view?: SnapshotView;
  /** `subagent_stream` frames. */
  toolCallId?: string;
  index?: number;
  rev?: number;
  reset?: boolean;
  upserts?: SnapshotView["messages"];
  removed?: string[];
  partial?: SnapshotView["messages"][number] | null;
  live?: boolean;
  missing?: boolean;
  truncated?: boolean;
  delta?: {
    rev: number;
    upserts: SnapshotView["messages"];
    removed: string[];
    runTimings?: SnapshotView["runTimings"];
    modelSwitches?: SnapshotView["modelSwitches"];
    activeRun?: { runId: string; startedAt: number } | null;
    workActivity?: SessionSnapshot["workActivity"];
    compaction?: SessionSnapshot["compaction"] | null;
    tree?: SnapshotView["tree"];
    sessionFile?: string;
    executionPolicy?: ExecutionPolicyConfig;
    planState?: SnapshotView["planState"];
    todoState?: SnapshotView["todoState"];
    goalState?: SnapshotView["goalState"] | null;
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
  /** Observe lifecycle frames synchronously, before a waiter can check cleanup too late. */
  onOutput?: (output: Output) => void;
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
      this.onOutput?.(output);
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
        modelSwitches: output.delta.modelSwitches ?? this.view.modelSwitches,
        runTimings: output.delta.runTimings ?? this.view.runTimings,
        compaction: output.delta.compaction === undefined ? this.view.compaction : output.delta.compaction ?? undefined,
        activeRun: output.delta.activeRun === undefined ? this.view.activeRun : output.delta.activeRun ?? undefined,
        workActivity: output.delta.workActivity ?? this.view.workActivity,
        tree: output.delta.tree ?? this.view.tree,
        sessionFile: output.delta.sessionFile ?? this.view.sessionFile,
        executionPolicy: output.delta.executionPolicy ?? this.view.executionPolicy,
        planState: output.delta.planState ?? this.view.planState,
        todoState: output.delta.todoState ?? this.view.todoState,
        goalState: output.delta.goalState === undefined ? this.view.goalState : output.delta.goalState ?? undefined,
        skillCreator: output.delta.skillCreator === undefined ? this.view.skillCreator : output.delta.skillCreator ?? undefined,
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
  /** Goal-verifier replies, one per verifier request (`<goal>` turn); empty = default pass. */
  verdicts: string[];
  close: () => Promise<void>;
  waitForSlowRequest: () => Promise<void>;
  holdChildren: (count: number, afterTools?: boolean) => { arrived: Promise<void>; release: () => void };
  holdRequest: (matches: (request: MockProvider["requests"][number]) => boolean) => { arrived: Promise<void>; release: () => void };
}

async function startMockProvider(): Promise<MockProvider> {
  const requests: MockProvider["requests"] = [];
  const verdicts: string[] = [];
  let childGate: { afterTools: boolean; remaining: number; arrive: () => void; released: Promise<void>; release: () => void } | undefined;
  let requestGate: { matches: (request: MockProvider["requests"][number]) => boolean; consumed: boolean; arrive: () => void; released: Promise<void>; release: () => void } | undefined;
  let slowRequestResolve: (() => void) | undefined;
  const slowRequest = new Promise<void>((resolvePromise) => { slowRequestResolve = resolvePromise; });
  const server = createServer(async (request, response) => {
    const chunks: Buffer[] = [];
    for await (const chunk of request) chunks.push(Buffer.from(chunk));
    const body = JSON.parse(Buffer.concat(chunks).toString("utf8")) as Record<string, unknown>;
    const authorization = String(request.headers.authorization ?? "");
    const text = userTextOf(body);
    const entry = { authorization, body, at: Date.now(), text };
    requests.push(entry);
    // Hold the MODEL request after a shell yield, not the original bash tool call.
    if (requestGate && !requestGate.consumed && requestGate.matches(entry)) {
      const gate = requestGate;
      gate.consumed = true;
      gate.arrive();
      await gate.released;
    }
    if (authorization === "Bearer overflow-secret" && text === "Overflow fixture"
      && requests.filter((entry) => entry.text === text).length === 1) {
      response.writeHead(400, { "content-type": "application/json" });
      response.end(JSON.stringify({ error: { message: "Context length exceeded" } }));
      return;
    }
    // Test-controlled barrier: requests stay open until the test observes all expected children.
    if (childGate && text.startsWith("child-") && hasToolMessage(body) === childGate.afterTools) {
      if (--childGate.remaining === 0) childGate.arrive();
      await childGate.released;
    }
    if (text.startsWith("child-fail:")) {
      response.writeHead(400, { "content-type": "application/json" });
      response.end(JSON.stringify({ error: { message: "Fixture child failure." } }));
      return;
    }
    if (authorization === "Bearer title-fail-secret") {
      response.writeHead(429, { "content-type": "application/json" });
      response.end(JSON.stringify({ error: { message: "Title request failed" } }));
      return;
    }
    if (authorization === "Bearer cancel-secret" || text === "wait until stopped" || text.startsWith("child-wait")) {
      response.writeHead(200, { "content-type": "text/event-stream", connection: "keep-alive" });
      response.write(": waiting\n\n");
      slowRequestResolve?.();
      return;
    }
    // Keeps parallel read-only children in flight together long enough to observe overlap.
    if (text.startsWith("child-ls") && !hasToolMessage(body)) await new Promise((wake) => setTimeout(wake, 150));
    streamAgentResponse(response, authorization, body, verdicts);
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
    verdicts,
    waitForSlowRequest: () => slowRequest,
    holdChildren: (count, afterTools = false) => {
      let arrive!: () => void;
      let release!: () => void;
      const arrived = new Promise<void>((resolve) => { arrive = resolve; });
      const released = new Promise<void>((resolve) => { release = resolve; });
      childGate = { afterTools, remaining: count, arrive, released, release };
      return { arrived, release };
    },
    holdRequest: (matches) => {
      let arrive!: () => void;
      let release!: () => void;
      const arrived = new Promise<void>((resolve) => { arrive = resolve; });
      const released = new Promise<void>((resolve) => { release = resolve; });
      requestGate = { matches, consumed: false, arrive, released, release };
      return { arrived, release };
    },
    close: () => new Promise<void>((resolvePromise, reject) => {
      childGate?.release();
      requestGate?.release();
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

type ProviderMessage = {
  role?: string;
  content?: unknown;
  tool_calls?: Array<{ id: string; function: { name: string; arguments: string } }>;
};

function messagesAfterUser(body: Record<string, unknown>): ProviderMessage[] {
  const messages = (body.messages ?? []) as ProviderMessage[];
  return messages.slice(messages.map((message) => message.role).lastIndexOf("user") + 1);
}

function toolResultContents(body: Record<string, unknown>): string[] {
  return messagesAfterUser(body).filter((message) => message.role === "tool").map((message) =>
    typeof message.content === "string" ? message.content
      : Array.isArray(message.content) ? (message.content as Array<{ type?: string; text?: string }>)
        .filter((part) => part.type === "text").map((part) => part.text ?? "").join("\n") : "");
}

function toolCallsOf(body: Record<string, unknown>) {
  return messagesAfterUser(body).flatMap((message) => message.tool_calls ?? []).map((call) => ({
    id: call.id, name: call.function.name, args: JSON.parse(call.function.arguments) as Record<string, unknown>,
  }));
}

interface ShellJobScript {
  command: string;
  yieldTimeout?: number;
  timeout?: number;
  actions: Array<{ action: "status" | "wait" | "stop"; waitSeconds?: number }>;
}

function shellJobPrompt(script: ShellJobScript, child = false): string {
  return `${child ? "child-shell-job" : "shell job"}: ${JSON.stringify(script)}`;
}

function shellJobScriptOf(text: string): ShellJobScript | undefined {
  // Only the goal's kickoff runs a shell; the continuation should find the old job gone.
  if (text.startsWith("The user has set a goal.")) text = /<objective>\n([\s\S]*?)\n<\/objective>/.exec(text)?.[1] ?? "";
  const prefix = ["shell job: ", "child-shell-job: "].find((candidate) => text.startsWith(candidate));
  return prefix ? JSON.parse(text.slice(prefix.length)) as ShellJobScript : undefined;
}

const RUNNING_SHELL_JOB = /Job ([0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}) is still running/i;

function nextShellJobCall(script: ShellJobScript, results: string[]): { name: string; args: Record<string, unknown> } | undefined {
  if (!results.length) return {
    name: "bash", args: { command: script.command, yieldTimeout: script.yieldTimeout ?? 0.03, ...(script.timeout === undefined ? {} : { timeout: script.timeout }) },
  };
  // The provider sees CONTENT, not details.shellJob. A blocked/fast bash has no running id.
  const jobId = RUNNING_SHELL_JOB.exec(results[0])?.[1];
  if (!jobId) return undefined;
  const last = script.actions.at(-1);
  const action = script.actions[results.length - 1]
    // Repeat only a final wait while still running, with a finite budget of short check-ins.
    ?? (results.length < 30 && last?.action === "wait" && RUNNING_SHELL_JOB.test(results.at(-1) ?? "") ? last : undefined);
  return action ? { name: "bash_job", args: { jobId, ...action } } : undefined;
}

function streamAgentResponse(response: ServerResponse<IncomingMessage>, authorization: string, body: Record<string, unknown>, verdicts: string[]): void {
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
  if (userTextOf(body).startsWith("Staged diff")) {
    // Git mode's body mode asks for a description after the summary line.
    const content = JSON.stringify(body.messages ?? []).includes("short description")
      ? "Summarize staged changes\n\nExplain the change."
      : "Summarize staged changes";
    response.write(`data: ${JSON.stringify({ id: "commit-message", object: "chat.completion.chunk", created: 1, model: "shared-model", choices: [{ index: 0, delta: { role: "assistant", content }, finish_reason: null }] })}\n\n`);
    response.write(`data: ${JSON.stringify({ id: "commit-message", object: "chat.completion.chunk", created: 1, model: "shared-model", choices: [{ index: 0, delta: {}, finish_reason: "stop" }] })}\n\n`);
    response.end("data: [DONE]\n\n");
    return;
  }
  const suffix = authorization === "Bearer alpha-secret" ? "alpha" : "beta";
  const send = (value: unknown) => response.write(`data: ${JSON.stringify(value)}\n\n`);
  const userText = userTextOf(body);
  // The goal loop's verifier: a no-tools `completeSimple` whose user message is a <goal>
  // envelope. Tests script its verdicts; unanswered ones pass so a goal always terminates.
  if (userText.startsWith("<goal>")) {
    const verdict = verdicts.shift() ?? '{"passed": true, "reason": "verified"}';
    send({
      id: `verdict-${suffix}`, object: "chat.completion.chunk", created: 1, model: "shared-model",
      choices: [{ index: 0, delta: { role: "assistant", content: verdict }, finish_reason: null }]
    });
    send({
      id: `verdict-${suffix}`, object: "chat.completion.chunk", created: 1, model: "shared-model",
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      usage: { prompt_tokens: 10, completion_tokens: 4, total_tokens: 14 }
    });
    response.end("data: [DONE]\n\n");
    return;
  }
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
  // The /skill-creator guide: prepare first, then preview once the draft exists (its id rides
  // the prepare tool result). "skill preview again" scripts a feedback turn's preview.
  if (lastUserText.startsWith("You are helping the user create or improve an Agent Skill") || lastUserText === "skill preview again") {
    const results = lastUserText === "skill preview again"
      ? (Array.isArray(body.messages) ? body.messages as Array<{ role?: string }> : []).filter((message) => message.role === "tool")
          .map((message) => typeof (message as { content?: unknown }).content === "string" ? (message as { content: string }).content : "")
          .join("\n")
      : toolResultContents(body).join("\n");
    const draftId = /\n([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\n?$/.exec(results)?.[1]
      ?? /Draft workspace ready: [^\n]*skill-creator\/([0-9a-f-]{36})/.exec(results)?.[1];
    const call = draftId
      ? { index: 0, id: `skill-preview-${suffix}`, type: "function", function: { name: "skill_creator", arguments: JSON.stringify({ action: "preview", draftId }) } }
      : { index: 0, id: `skill-prepare-${suffix}`, type: "function", function: { name: "skill_creator", arguments: JSON.stringify({ action: "prepare", name: "greeting-skill" }) } };
    send({
      id: `skill-creator-${suffix}`, object: "chat.completion.chunk", created: 1, model: "shared-model",
      choices: [{ index: 0, delta: { role: "assistant", tool_calls: [call] }, finish_reason: null }]
    });
    send({
      id: `skill-creator-${suffix}`, object: "chat.completion.chunk", created: 1, model: "shared-model",
      choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
      usage: { prompt_tokens: 20, completion_tokens: 8, total_tokens: 28 }
    });
    response.end("data: [DONE]\n\n");
    return;
  }
  // These scripts call their tool once per prompt, so one chat can make several calls.
  const answeredSinceUser = messages.slice(messages.map((message) => message.role).lastIndexOf("user") + 1).some((message) => message.role === "tool");
  const repeatable = lastUserText.startsWith("mcp: ") || lastUserText.startsWith("subagent ") || lastUserText.startsWith("finish plan");
  const backgroundText = lastUserText.startsWith("The user has set a goal.")
    ? /<objective>\n([^\n]+)\n<\/objective>/.exec(lastUserText)?.[1] ?? lastUserText : lastUserText;
  const backgroundScript = backgroundText.startsWith("subagent background: ")
    ? JSON.parse(backgroundText.slice("subagent background: ".length)) as { tag: string; launch: Record<string, unknown>; next?: { name: string; args: Record<string, unknown> } } : undefined;
  const backgroundResults = backgroundScript ? toolResultContents(body) : [];
  const backgroundCall = backgroundScript ? backgroundResults.length === 0 ? { name: "subagent", args: { ...backgroundScript.launch, background: true } }
    : backgroundResults.length === 1 ? backgroundScript.next : undefined : undefined;
  const shellScript = shellJobScriptOf(lastUserText);
  const shellResults = shellScript ? toolResultContents(body) : [];
  const shellCall = shellScript ? nextShellJobCall(shellScript, shellResults) : undefined;
  if (backgroundScript ? backgroundCall !== undefined : shellScript ? shellCall !== undefined : repeatable ? !answeredSinceUser : !hasToolResult) {
    const toolCall = (name: string, args: Record<string, unknown>) => ({
      index: 0, id: backgroundScript ? `call-background-${backgroundScript.tag}-${backgroundResults.length}` : shellScript ? `call-shell-${suffix}-${shellResults.length}` : `call-${suffix}`, type: "function",
      function: { name, arguments: JSON.stringify(args) }
    });
    const calls = backgroundCall ? [toolCall(backgroundCall.name, backgroundCall.args)]
      : shellCall ? [toolCall(shellCall.name, shellCall.args)]
      : lastUserText.startsWith("Initialize project instructions")
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
      // "mcp: <tool name> <JSON arguments>"
      : lastUserText.startsWith("mcp: ")
        ? [toolCall(lastUserText.slice(5).split(" ")[0], JSON.parse(lastUserText.slice(5).split(" ").slice(1).join(" ") || "{}"))]
        : lastUserText.startsWith("stream tool")
          ? [toolCall("bash", { command: lastUserText === "stream tool until stopped"
            ? "printf 'first line\\n'; sleep 30; printf 'should not finish\\n'; touch completed.txt"
            : "printf 'first line\\n'; sleep 0.3; printf 'second line\\n'" })]
        : lastUserText.startsWith("subagent access: ")
          ? [toolCall("subagent", JSON.parse(lastUserText.slice("subagent access: ".length)))]
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
        : lastUserText.startsWith("subagent test: ")
          ? [toolCall("subagent", { agent: "reviewer", task: `child-test: ${lastUserText.slice("subagent test: ".length)}` })]
        : lastUserText.startsWith("subagent plan")
          ? [toolCall("subagent", { agent: "worker", task: "child-write: planned" })]
        : lastUserText.startsWith("subagent fetch")
          ? [toolCall("subagent", { agent: "scout", task: "child-fetch: http://localhost/docs" })]
        : lastUserText.startsWith("subagent batch wait")
          ? [toolCall("subagent", { tasks: [0, 1, 2, 3].map((index) => ({ agent: "worker", task: `child-wait: ${index}` })) })]
        : lastUserText.startsWith("subagent wait")
          ? [toolCall("subagent", { agent: "scout", task: "child-wait: until stopped" })]
        // The child's answer quotes its task, so it repeats the chat's own key.
        : lastUserText.startsWith("subagent leak")
          ? [toolCall("subagent", { agent: "scout", task: "child-ls: leak alpha-secret" })]
        : lastUserText.startsWith("child-ls")
          ? [toolCall("ls", { path: "." })]
        : lastUserText.startsWith("child-fetch: ")
          ? [toolCall("web_fetch", { url: lastUserText.slice("child-fetch: ".length) })]
        : lastUserText.startsWith("child-shell: ")
          ? [toolCall("bash", { command: lastUserText.slice("child-shell: ".length) })]
        : lastUserText.startsWith("child-rm")
          ? [toolCall("bash", { command: "rm -f keep.txt" })]
        : lastUserText.startsWith("child-test: ")
          ? [toolCall("bash", { command: lastUserText.slice("child-test: ".length) })]
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

function initCommand(
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
) {
  return {
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
  };
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
  worker.send(initCommand(baseUrl, apiKey, workspace, taskId, sessionFile, disabledTools, resources, mode, vision, extra));
  return { worker, ready: await worker.waitFor((output) => output.type === "ready") };
}

/**
 * Sends only `init`, without waiting for `ready`: tests for the start-up window then send
 * their commands the way the host does, back to back with the init that is still running.
 */
function launchWorker(baseUrl: string, apiKey: string, workspace: string, taskId: string): WorkerHarness {
  const worker = new WorkerHarness(workspace);
  worker.send(initCommand(baseUrl, apiKey, workspace, taskId));
  return worker;
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
  it("generates a commit message from staged data without tools or a chat turn", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-commit-message-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "commit-message-task");
    cleanup.push(() => worker.shutdown());
    const id = crypto.randomUUID();
    worker.send({ id, type: "generate_commit_message", diff: "diff --git a/file b/file\n+new text", truncated: false });
    const result = await worker.waitFor((output) => output.type === "response" && output.id === id);
    expect(result.success).toBe(true);
    expect(result.result).toBe("Summarize staged changes");
    expect(worker.outputs.filter((o) => o.type === "usage_record").map((o) => o.record?.purpose)).toEqual(["commit_message"]);
    expect(provider.requests).toHaveLength(1);
    expect(provider.requests[0].body.tools).toBeUndefined();
    expect(provider.requests[0].text).toContain("+new text");
    expect(worker.view?.messages ?? []).toHaveLength(0);
    expect(JSON.stringify(worker.outputs) + worker.stderr).not.toContain("alpha-secret");

    // Body mode (Git mode's form): a summary, a blank line, then a description, newlines kept.
    const bodyId = crypto.randomUUID();
    worker.send({ id: bodyId, type: "generate_commit_message", diff: "diff --git a/file b/file\n+new text", truncated: false, body: true });
    const described = await worker.waitFor((output) => output.type === "response" && output.id === bodyId);
    expect(described.success).toBe(true);
    expect(described.result).toBe("Summarize staged changes\n\nExplain the change.");
    expect(provider.requests).toHaveLength(2);
    expect(provider.requests[1].body.max_tokens ?? provider.requests[1].body.max_completion_tokens).toBe(512);
  });
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
    const usage = worker.outputs.filter((o) => o.type === "usage_record").map((o) => o.record!);
    expect(usage.filter((r) => r.purpose === "title")).toHaveLength(1);
    expect(usage.filter((r) => r.purpose === "chat")).toHaveLength(3);
    expect(usage).toHaveLength(provider.requests.length);
    expect(new Set(usage.map((r) => r.id)).size).toBe(usage.length);
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

  it("titles a chat whose opening run is a slash command, /init or /goal", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-title-command-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const prompt = join(workspace, "review.md");
    await writeFile(prompt, "---\ndescription: Review this work\n---\nReview $ARGUMENTS\n");
    const resources = { extensions: [], skills: [], prompts: [prompt], themes: [] };
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "title-command-task", undefined, undefined, resources);
    cleanup.push(() => worker.shutdown());
    const titleProvider = {
      id: "title-provider", name: "Title provider", kind: "custom", baseUrl: provider.baseUrl,
      api: "openai-completions", models: [{ id: "shared-model", name: "Small title model",
        contextWindow: 16_384, maxTokens: 1_024, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: {} }]
    };

    // A template command's expansion is the opening message the title labels.
    const listedId = crypto.randomUUID();
    worker.send({ id: listedId, type: "list_commands" });
    const listed = await worker.waitFor((output) => output.type === "response" && output.id === listedId);
    const commands = listed.result as unknown as Array<{ id: string; source: string }>;
    const review = commands.find((entry) => entry.source === "prompt")!;
    const commandId = crypto.randomUUID();
    worker.send({ id: commandId, type: "execute_command", commandId: review.id, args: '"the staged diff"', runId: "title-command",
      autoTitle: { attemptId: "cmd", provider: titleProvider, modelId: "shared-model", apiKey: "title-secret" } });
    const accepted = await worker.waitFor((output) => output.type === "response" && output.id === commandId);
    expect(accepted.success).toBe(true);
    const commandTitle = await worker.waitFor((output) => output.type === "title_result" && output.attemptId === "cmd");
    expect(commandTitle.title).toBe("Short Chat Title");
    const commandSource = provider.requests.find((request) => request.authorization === "Bearer title-secret")!;
    expect(commandSource.text).toContain("Review the staged diff");
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "title-command" && output.state === "idle");

    // /init titles from the generated prompt it opens with.
    const initId = crypto.randomUUID();
    worker.send({ id: initId, type: "init_agents", runId: "title-init",
      autoTitle: { attemptId: "init", provider: titleProvider, modelId: "shared-model", apiKey: "title-secret" } });
    const initAccepted = await worker.waitFor((output) => output.type === "response" && output.id === initId);
    expect(initAccepted.success).toBe(true);
    const initTitle = await worker.waitFor((output) => output.type === "title_result" && output.attemptId === "init");
    expect(initTitle.title).toBe("Short Chat Title");
    const initSource = provider.requests.filter((request) => request.authorization === "Bearer title-secret")[1];
    expect(initSource.text).toContain("Initialize project instructions");
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "title-init" && output.state === "idle");

    // /goal titles from the objective itself, not the kickoff prompt wrapped around it.
    worker.send({ id: crypto.randomUUID(), type: "goal_control", action: "set", objective: "goal: polish the README", runId: "title-goal", startedAt: Date.now(),
      autoTitle: { attemptId: "goal", provider: titleProvider, modelId: "shared-model", apiKey: "title-secret" } });
    const goalTitle = await worker.waitFor((output) => output.type === "title_result" && output.attemptId === "goal");
    expect(goalTitle.title).toBe("Short Chat Title");
    const goalSource = provider.requests.filter((request) => request.authorization === "Bearer title-secret")[2];
    expect(goalSource.text).toContain("goal: polish the README");
    expect(goalSource.text).not.toContain("Work on it now");
    await worker.waitFor((output) => output.type === "goal_state" && output.goal?.phase === "complete");
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
    const resolved = await worker.waitFor((output) => output.type === "extension_ui_resolved" && output.requestId === request.requestId);
    expect(resolved.cancelled).toBe(false);
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
    // enable, plus WackCode's built-in extension tools.
    const names = (ready.snapshot?.tools ?? []).map((tool) => tool.name).sort();
    expect(names).toEqual([
      "ask_user_question",
      "bash",
      "bash_job",
      "browser_act",
      "browser_console",
      "browser_open",
      "browser_screenshot",
      "browser_snapshot",
      "computer_act",
      "computer_apps",
      "computer_open",
      "computer_screenshot",
      "computer_snapshot",
      "edit",
      "find",
      "grep",
      "ls",
      "memory_forget",
      "memory_recall",
      "memory_save",
      "plan_mode_complete",
      "read",
      "skill_creator",
      "subagent",
      "subagent_job",
      "todo",
      "web_fetch",
      "write"
    ]);
    expect(ready.snapshot?.tools?.every((tool) => tool.source.kind === "builtin" || tool.source.kind === "wackcode")).toBe(true);
    // The denylist from `init` is applied before the first turn, and tools whose external
    // binary is missing are never offered even though they stay listed in the catalogue.
    const catalog = ready.snapshot?.tools ?? [];
    // Sub-agents, computer use and memory are the built-ins that stay off until the user (or
    // the host's memory payload) switches them on; this test sends no memory payload.
    const expectedActive = catalog
      .filter((tool) => tool.available && tool.name !== "find" && !tool.name.startsWith("subagent") && !tool.name.startsWith("computer_") && !tool.name.startsWith("memory_"))
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

    // Toggling tools takes effect on the next turn with no worker restart. Browser tools
    // and web_fetch are user-switchable; all other WackCode tools stay active.
    worker.send({
      id: crypto.randomUUID(),
      type: "set_tools",
      disabledTools: [
        "bash",
        "bash_job", // Always-on management survives the bash denylist.
        "grep",
        "ls",
        "find",
        "web_fetch",
        "browser_open",
        "browser_snapshot",
        "browser_act",
        "browser_screenshot",
        "browser_console"
      ]
    });
    await worker.waitFor((output) => emitted(output) && output.view?.activeTools?.length === 8);
    // `find`/`grep` availability varies by host; the companion stays on even without bash.
    const beforeSecondRun = provider.requests.length;
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-2", message: "Again." });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.snapshot === undefined && provider.requests.length > beforeSecondRun);

    expect(offered(provider.requests[beforeSecondRun])).toEqual(["ask_user_question", "bash_job", "edit", "plan_mode_complete", "read", "skill_creator", "todo", "write"]);
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

  it("records a restored session's configured model switch once and replays it", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-model-switch-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const first = await initializeWorker(provider.baseUrl, "switch-secret", workspace, "switch-task");
    cleanup.push(() => first.worker.shutdown());
    first.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "switch-run", message: "First model." });
    await first.worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.role === "assistant") === true);
    const sessionFile = first.worker.view?.sessionFile;
    expect(sessionFile).toBeTruthy();
    await first.worker.shutdown();

    const alternateProvider = {
      id: "provider-switch-task",
      name: "Provider switch-task",
      kind: "custom",
      baseUrl: provider.baseUrl,
      api: "openai-completions",
      models: [{
        id: "alternate-model", name: "Alternate model", contextWindow: 16_384, maxTokens: 321,
        reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null }, vision: false
      }]
    };
    const second = await initializeWorker(provider.baseUrl, "switch-secret", workspace, "switch-task", sessionFile, undefined, undefined, undefined, false, {
      provider: alternateProvider, modelId: "alternate-model", thinkingLevel: "off"
    });
    cleanup.push(() => second.worker.shutdown());
    expect(second.ready.snapshot?.modelSwitches).toMatchObject([{
      at: second.ready.snapshot?.messages.length,
      from: { providerId: "provider-switch-task", modelId: "shared-model" },
      to: { providerId: "provider-switch-task", modelId: "alternate-model" }
    }]);
    await second.worker.shutdown();

    const third = await initializeWorker(provider.baseUrl, "switch-secret", workspace, "switch-task", sessionFile, undefined, undefined, undefined, false, {
      provider: alternateProvider, modelId: "alternate-model", thinkingLevel: "off"
    });
    cleanup.push(() => third.worker.shutdown());
    expect(third.ready.snapshot?.modelSwitches).toHaveLength(1);
    await expectSavedHistory(third.ready.snapshot!);
    const saved = await readFile(sessionFile as string, "utf8");
    expect(saved.match(/"type":"model_change"/g)).toHaveLength(2);
  });

  it("opens a chat read-only when its model left the connection, and runs again once another is picked", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-missing-model-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const first = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "missing-model-task");
    cleanup.push(() => first.worker.shutdown());
    first.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "first-run", message: "Run once." });
    await first.worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.role === "assistant") === true);
    const sessionFile = first.worker.view?.sessionFile;
    expect(sessionFile).toBeTruthy();
    await first.worker.shutdown();
    const servedRequests = provider.requests.length;
    const modelChanges = ((await readFile(sessionFile as string, "utf8")).match(/"type":"model_change"/g) ?? []).length;

    // The connection no longer lists the model the chat was configured with: the session must
    // still load for reading, on a stand-in the snapshot reports, without touching the provider.
    const withoutModel = {
      id: "provider-missing-model-task",
      name: "Provider missing-model-task",
      kind: "custom",
      baseUrl: provider.baseUrl,
      api: "openai-completions",
      models: [{
        id: "alternate-model", name: "Alternate model", contextWindow: 16_384, maxTokens: 321,
        reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null }, vision: false
      }]
    };
    const degraded = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "missing-model-task", sessionFile, undefined, undefined, undefined, false, {
      provider: withoutModel, modelId: "shared-model", thinkingLevel: "off"
    });
    cleanup.push(() => degraded.worker.shutdown());
    expect(degraded.ready.snapshot?.modelMissing).toBe(true);
    expect(degraded.ready.snapshot?.modelIssue).toBe("This chat's model is no longer configured. Pick another to continue.");
    expect(degraded.ready.snapshot?.messages.some((message) => message.role === "assistant")).toBe(true);
    expect(degraded.worker.outputs.filter((output) => output.type === "worker_error")).toEqual([]);
    expect(provider.requests).toHaveLength(servedRequests);
    expect(((await readFile(sessionFile as string, "utf8")).match(/"type":"model_change"/g) ?? []).length).toBe(modelChanges);

    // Everything that needs the model is refused with one actionable sentence, not a crash.
    const promptId = crypto.randomUUID();
    degraded.worker.send({ id: promptId, type: "prompt", runId: "refused-run", message: "Try to run." });
    const refusal = await degraded.worker.waitFor((output) => output.type === "response" && output.id === promptId && output.success === false);
    expect(refusal.error).toBe("This chat's model is no longer configured. Pick another to continue.");
    await degraded.worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "refused-run");
    const commitId = crypto.randomUUID();
    degraded.worker.send({ id: commitId, type: "generate_commit_message", diff: "diff --git a/file b/file\n+new text", truncated: false });
    const commitRefusal = await degraded.worker.waitFor((output) => output.type === "response" && output.id === commitId && output.success === false);
    expect(commitRefusal.error).toBe("Choose an available model before generating a commit message");
    expect(provider.requests).toHaveLength(servedRequests);
    await degraded.worker.shutdown();

    // Picking a model that exists respawns into a normal chat, records the durable switch, runs.
    const repaired = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "missing-model-task", sessionFile, undefined, undefined, undefined, false, {
      provider: withoutModel, modelId: "alternate-model", thinkingLevel: "off"
    });
    cleanup.push(() => repaired.worker.shutdown());
    expect(repaired.ready.snapshot?.modelMissing).toBeUndefined();
    expect(repaired.ready.snapshot?.modelIssue).toBeUndefined();
    expect(repaired.ready.snapshot?.modelSwitches).toMatchObject([{
      from: { providerId: "provider-missing-model-task", modelId: "shared-model" },
      to: { providerId: "provider-missing-model-task", modelId: "alternate-model" }
    }]);
    repaired.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "repaired-run", message: "Run again." });
    await repaired.worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "repaired-run");
    // The restored transcript already ends in "Finished alpha." from the first run; the repaired
    // run adds its own.
    const finished = repaired.worker.view?.messages.filter((message) => message.blocks.some((block) => block.text === "Finished alpha."));
    expect(finished).toHaveLength(2);
    expect(provider.requests.at(-1)?.body.model).toBe("alternate-model");
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

  it("honours a stop sent while the worker is still starting", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-early-abort-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    // The host writes init and the first prompt back to back; a stop in that first second
    // used to bounce off "Worker is not initialized" and the run carried on.
    const worker = launchWorker(provider.baseUrl, "cancel-secret", workspace, "early-abort-task");
    cleanup.push(() => worker.shutdown());
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "early-run", message: "Wait until stopped." });
    worker.send({ id: crypto.randomUUID(), type: "abort" });

    await worker.waitFor((output) => output.type === "ready");
    await worker.waitFor((output) => output.type === "run_state" && output.state === "stopping");
    const finished = await worker.waitFor((output) => output.type === "run_finished" && output.runId === "early-run");
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    expect(finished.outcome).toBe("stopped");
    expect(worker.outputs.some((output) => output.type === "worker_error")).toBe(false);
    // Whether the stop landed before the run's first model request or cancelled it mid-stream,
    // at most one request went out.
    expect(provider.requests.length).toBeLessThanOrEqual(1);
  });

  it("delivers a message queued while the worker is still starting", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-early-queue-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    // The second message used to skip the queue straight into a session that didn't exist yet
    // and vanish: fire-and-forget on the host side, error wiped by the next snapshot.
    const worker = launchWorker(provider.baseUrl, "alpha-secret", workspace, "early-queue-task");
    cleanup.push(() => worker.shutdown());
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "early-one", message: "Create the fixture." });
    worker.send({ id: crypto.randomUUID(), type: "queue_message", message: "Second message." });

    await worker.waitFor((output) => emitted(output)
      && output.view?.messages.filter((message) => message.role === "user").length === 2
      && output.view?.messages.some((message) => message.role === "user" && message.blocks.some((block) => block.text === "Second message.")) === true);
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    expect(worker.outputs.some((output) => output.type === "worker_error")).toBe(false);
    expect(worker.view?.messages.some((message) => message.role === "assistant")).toBe(true);
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
    // The aborted-signal failure Pi records on the trailing assistant message must not reach the
    // transcript as an error either: saved chats showed it as a red "This operation was aborted"
    // block instead of the quiet "Stopped" label.
    const trailing = [...worker.view?.messages ?? []].reverse().find((message) => message.role === "assistant");
    expect(trailing?.stopReason).toBe("aborted");
    expect(trailing?.errorMessage).toBeUndefined();
    expect(worker.view?.messages.some((message) => message.stopReason === "error")).toBe(false);
  });
});

describe("shell job check-ins", () => {
  type ShellJob = { id: string; status: string; elapsedSeconds: number; quietSeconds: number; outputBytes: number; newOutputBytes: number };
  const shellResults = (worker: WorkerHarness) => (worker.view?.messages ?? []).flatMap((message) => {
    const blocks = message.blocks.filter((block) => block.type === "tool-result" && ["bash", "bash_job"].includes(block.toolName ?? ""));
    // One result can normalize to several blocks (output + job summary), each repeating details.
    return blocks.length ? [{ ...blocks[0], text: blocks.map((block) => block.text ?? "").join("\n") }] : [];
  });
  const jobOf = (block: ReturnType<typeof shellResults>[number]) => (block.details as { shellJob?: ShellJob } | undefined)?.shellJob;
  const sleepingShell = "printf '%s\\n' $$ > shell.pid; sleep 60 & printf '%s\\n' $! > child.pid; printf 'started\\n'; wait";

  function alive(pid: number): boolean {
    try { process.kill(pid, 0); return true; }
    catch (error) { if ((error as NodeJS.ErrnoException).code === "ESRCH") return false; throw error; }
  }

  async function pidAt(workspace: string, file: string): Promise<number> {
    const read = async () => Number(await readFile(join(workspace, file), "utf8"));
    await expect.poll(read, { timeout: 2_000, interval: 10 }).toBeGreaterThan(1);
    return read();
  }

  async function start(taskId: string) {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), `wackcode-${taskId}-`));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker, ready } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, taskId);
    // A failed assertion at a held model request must not leave a detached shell behind.
    cleanup.push(async () => { worker.send({ id: crypto.randomUUID(), type: "abort" }); await worker.shutdown(); });
    return { provider, workspace, worker, ready };
  }

  function holdAfter(provider: MockProvider, message: string, results: number) {
    const barrier = provider.holdRequest((request) => !request.text.startsWith("<goal>")
      && request.text.includes(message) && toolResultContents(request.body).length === results);
    cleanup.push(async () => { barrier.release(); });
    return barrier;
  }

  it("returns control to the model with a live job, then completes that same execution through status/wait", async () => {
    const { provider, workspace, worker, ready } = await start("shell-check-in");
    expect(ready.snapshot?.tools?.find((tool) => tool.name === "bash")?.source.kind).toBe("builtin");
    const message = shellJobPrompt({
      command: "printf 'once\\n' >> executions.txt; printf '%s\\n' $$ > shell.pid; printf 'started\\n'; while [ ! -e finish ]; do sleep 0.01; done; printf 'finished\\n'",
      timeout: 600, yieldTimeout: 0.03, actions: [{ action: "status" }, { action: "wait", waitSeconds: 0.1 }],
    });
    const yielded = holdAfter(provider, message, 1);
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "shell-run", message });
    await yielded.arrived;
    const request = provider.requests.at(-1)!;
    const content = toolResultContents(request.body)[0];
    const jobId = RUNNING_SHELL_JOB.exec(content)?.[1];
    expect(jobId).toBeTruthy();
    expect(content).toContain("it has NOT completed");
    expect(messagesAfterUser(request.body).find((entry) => entry.role === "tool")).not.toHaveProperty("details");
    const pid = await pidAt(workspace, "shell.pid");
    expect(alive(pid)).toBe(true);
    expect(worker.outputs.some((output) => output.type === "run_state" && output.state === "idle")).toBe(false);

    const inspected = holdAfter(provider, message, 2);
    yielded.release();
    await inspected.arrived;
    expect(toolResultContents(provider.requests.at(-1)!.body)[1]).toContain(`Job ${jobId} is still running`);
    expect(alive(pid)).toBe(true);
    await writeFile(join(workspace, "finish"), "go");
    inspected.release();
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "shell-run" && output.state === "idle", 5_000);

    const results = shellResults(worker);
    const jobs = results.map(jobOf);
    expect(jobs[0]?.status).toBe("running");
    expect(jobs[1]?.status).toBe("running");
    expect(jobs.every((job) => job?.id === jobId)).toBe(true);
    expect(jobs.at(-1)).toMatchObject({
      status: "completed", elapsedSeconds: expect.any(Number), quietSeconds: expect.any(Number),
      outputBytes: Buffer.byteLength("started\nfinished\n"), newOutputBytes: expect.any(Number),
    });
    expect(jobs.reduce((total, job) => total + (job?.newOutputBytes ?? 0), 0)).toBe(jobs.at(-1)?.outputBytes);
    expect(results.at(-1)?.text).toContain("started\nfinished");
    const calls = toolCallsOf(provider.requests.at(-1)!.body);
    expect(calls.filter((call) => call.name === "bash")).toHaveLength(1);
    expect(calls.filter((call) => call.name === "bash_job").map((call) => call.args.jobId)).toEqual(jobs.slice(1).map(() => jobId));
    expect(new Set(calls.map((call) => call.id)).size).toBe(calls.length);
    expect(await readFile(join(workspace, "executions.txt"), "utf8")).toBe("once\n");
    expect(alive(pid)).toBe(false);
    expect(worker.outputs.filter((output) => output.type === "worker_error")).toEqual([]);
  });

  it("lets the model stop a yielded process group and inspect its stopped result", async () => {
    const { provider, workspace, worker } = await start("shell-model-stop");
    const message = shellJobPrompt({ command: sleepingShell, actions: [
      { action: "status" }, { action: "stop", waitSeconds: 0.1 }, { action: "status" },
    ] });
    const yielded = holdAfter(provider, message, 1);
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "model-stop", message });
    await yielded.arrived;
    const pids = await Promise.all([pidAt(workspace, "shell.pid"), pidAt(workspace, "child.pid")]);
    expect(pids.map(alive)).toEqual([true, true]);
    yielded.release();
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "model-stop" && output.state === "idle", 5_000);
    const results = shellResults(worker);
    expect(results.map((block) => jobOf(block)?.status)).toEqual(["running", "running", "stopped", "stopped"]);
    expect(new Set(results.map((block) => jobOf(block)?.id)).size).toBe(1);
    expect(results[2]).toMatchObject({ isError: false, text: expect.stringContaining("stopped") });
    expect(pids.map(alive)).toEqual([false, false]);
    expect(toolCallsOf(provider.requests.at(-1)!.body).map((call) => call.name)).toEqual(["bash", "bash_job", "bash_job", "bash_job"]);
    expect(worker.outputs.filter((output) => output.type === "worker_error")).toEqual([]);
  });

  it("keeps the original execution timeout hard after the model receives a check-in", async () => {
    const { provider, workspace, worker } = await start("shell-hard-timeout");
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "deadline", message: shellJobPrompt({
      command: "printf 'before deadline\\n'; sleep 60; touch deadline-escaped.txt",
      timeout: 0.2, yieldTimeout: 0.03, actions: [{ action: "wait", waitSeconds: 0.1 }],
    }) });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "deadline" && output.state === "idle", 5_000);
    const results = shellResults(worker);
    expect(jobOf(results[0])?.status).toBe("running");
    expect(jobOf(results.at(-1)!)?.status).toBe("failed");
    expect(results.at(-1)).toMatchObject({ isError: true, text: expect.stringContaining("Command timed out after 0.2 seconds") });
    expect(results.at(-1)?.text).toContain("before deadline");
    expect(new Set(results.map((block) => jobOf(block)?.id)).size).toBe(1);
    expect(toolCallsOf(provider.requests.at(-1)!.body).filter((call) => call.name === "bash")).toHaveLength(1);
    await expect(readFile(join(workspace, "deadline-escaped.txt"))).rejects.toMatchObject({ code: "ENOENT" });
  });

  it.each(["Stop", "normal settle", "goal continuation"] as const)("cleans the yielded shell AND its descendants before lifecycle frames on %s", async (ending) => {
    const { provider, workspace, worker } = await start(`shell-cleanup-${ending.replaceAll(" ", "-")}`);
    const message = shellJobPrompt({ command: sleepingShell, actions: [] });
    const yielded = holdAfter(provider, message, 1);
    if (ending === "goal continuation") {
      provider.verdicts.push('{"passed":false,"reason":"one more round","nextAction":"Confirm cleanup"}');
      worker.send({ id: crypto.randomUUID(), type: "goal_control", action: "set", runId: "cleanup", objective: message });
    } else worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "cleanup", message });
    await yielded.arrived;
    const pids = await Promise.all([pidAt(workspace, "shell.pid"), pidAt(workspace, "child.pid")]);
    expect(pids.map(alive)).toEqual([true, true]);
    expect(RUNNING_SHELL_JOB.test(toolResultContents(provider.requests.at(-1)!.body)[0])).toBe(true);
    const observed: Array<{ type: string; alive: boolean[] }> = [];
    worker.onOutput = (output) => {
      if (output.type === "run_finished" || (output.type === "run_state" && output.state === "idle")
        || (output.type === "goal_state" && output.goal?.phase === "verifying")) {
        observed.push({ type: output.type, alive: pids.map(alive) });
      }
    };
    if (ending === "Stop") worker.send({ id: crypto.randomUUID(), type: "abort" });
    else yielded.release();
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle", 5_000);
    // Sampled as each frame arrived, not after an extra sleep that could mask late cleanup.
    expect(observed.filter((entry) => entry.type === "run_state")).toEqual([{ type: "run_state", alive: [false, false] }]);
    expect(observed.every((entry) => entry.alive.every((running) => !running))).toBe(true);
    const finished = worker.outputs.find((output) => output.type === "run_finished" && output.runId === "cleanup");
    expect(finished).toMatchObject({ outcome: ending === "Stop" ? "stopped" : "completed" });
    if (ending === "Stop") {
      expect(worker.outputs.some((output) => output.type === "run_state" && output.state === "stopping")).toBe(true);
      expect(provider.requests).toHaveLength(2); // The second model request is STILL held.
      yielded.release();
    }
    if (ending === "goal continuation") {
      expect(observed.filter((entry) => entry.type === "goal_state")).toHaveLength(2);
      const completed = worker.outputs.find((output) => output.type === "goal_state" && output.goal?.phase === "complete");
      expect(completed?.goal).toMatchObject({ phase: "complete", iteration: 2 });
      expect(provider.requests.some((request) => request.text.startsWith("Goal continuation"))).toBe(true);
    }
    expect(worker.outputs.filter((output) => output.type === "worker_error")).toEqual([]);
    expect(worker.child.exitCode).toBeNull();
  });
});

describe("compaction continuity", () => {
  async function start(reason: "threshold" | "overflow" = "threshold") {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-compaction-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const sessionFile = join(workspace, "history.jsonl");
    const timestamp = Date.now() - 60_000;
    const input = reason === "overflow" ? 28 : 55_000;
    const usage = { input, output: 4, cacheRead: 0, cacheWrite: 0, totalTokens: input + 4, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };
    const assistant = (id: string, parentId: string) => ({ type: "message", id, parentId, timestamp: new Date(timestamp).toISOString(), message: { role: "assistant", content: [{ type: "text", text: id }], api: "openai-completions", provider: "provider-compaction", model: "shared-model", usage, stopReason: "stop", timestamp } });
    const image = solidPng(24, 24);
    const oldText = "Compacted source prompt. " + "Earlier context. ".repeat(8_000);
    const entries = [
      { type: "session", version: 3, id: "compaction-session", cwd: workspace, timestamp: new Date(timestamp).toISOString() },
      { type: "message", id: "initial-user", parentId: null, message: { role: "user", timestamp, content: "Initial task." } },
      assistant("initial-answer", "initial-user"),
      { type: "message", id: "old-user", parentId: "initial-answer", message: { role: "user", timestamp, content: [{ type: "text", text: oldText }, { type: "image", data: solidPng(12, 12), mimeType: "image/png" }] } },
      assistant("old-answer", "old-user"),
      { type: "context_edit", id: "image-edit", parentId: "old-answer", targetId: "old-user", replacement: { content: [{ type: "text", text: oldText }, { type: "image", data: image, mimeType: "image/png" }] } },
      { type: "custom", id: "old-timing", parentId: "image-edit", customType: "wackcode-run-timing", data: { version: 1, runId: "old", userMessageEntryId: "old-user", startedAt: timestamp - 3_000, endedAt: timestamp, durationMs: 3_000 } },
      { type: "message", id: "latest-user", parentId: "old-timing", message: { role: "user", timestamp, content: "Latest source prompt." } },
      assistant("latest-answer", "latest-user"),
      { type: "custom", id: "latest-timing", parentId: "latest-answer", customType: "wackcode-run-timing", data: { version: 1, runId: "latest", userMessageEntryId: "latest-user", startedAt: timestamp - 2_000, endedAt: timestamp, durationMs: 2_000 } }
    ];
    await writeFile(sessionFile, entries.map((entry) => JSON.stringify(entry)).join("\n") + "\n");
    const extension = join(workspace, "compact.ts");
    await writeFile(extension, `export default function (pi: any) {
      pi.on("session_before_compact", async (event: any) => {
        if (event.customInstructions === "wait") await new Promise((resolve) => event.signal.addEventListener("abort", resolve, { once: true }));
        if (event.reason !== "manual") await new Promise((resolve) => setTimeout(resolve, 120));
        return { compaction: { summary: "## Goal\\nContinue the fixture.\\n\\n## Next steps\\nComplete the task.", firstKeptEntryId: "latest-answer", tokensBefore: event.preparation.tokensBefore } };
      });
    }\n`);
    const resources = { extensions: [extension], skills: [], prompts: [], themes: [] };
    const command = initCommand(provider.baseUrl, reason === "overflow" ? "overflow-secret" : "alpha-secret", workspace, "compaction", sessionFile, undefined, resources, "build", true);
    command.provider.models[0].contextWindow = 65_536;
    const worker = new WorkerHarness(workspace);
    cleanup.push(() => worker.shutdown());
    worker.send(command);
    await worker.waitFor((output) => output.type === "ready");
    return { worker, provider, workspace, sessionFile, image, resources, command };
  }

  it("keeps manual compaction history, timings and images in live, restored and offline transcripts", async () => {
    const { worker, provider, workspace, sessionFile, image, resources, command } = await start();
    await worker.waitFor((output) => emitted(output) && output.view?.messages.find((message) => message.id === "old-user")?.blocks.some((block) => block.type === "image" && block.thumbnail !== undefined) === true);
    const before = worker.view!;
    const id = crypto.randomUUID();
    worker.send({ id, type: "compact", runId: "manual", instructions: "" });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "manual" && output.state === "idle");
    const compacted = worker.view!;
    expect(worker.outputs.find((output) => output.type === "run_state" && output.runId === "manual" && output.state === "running")?.operation).toBe("compaction");
    expect(compacted.activeRun).toBeUndefined();
    expect(compacted.compaction).toBeUndefined();
    expect(worker.outputs.filter((output) => output.type === "worker_error").map((output) => output.message)).toEqual([]);
    expect(compacted.messages.filter((message) => !message.compaction)).toEqual(before.messages);
    expect(compacted.runTimings).toEqual(before.runTimings);
    const marker = compacted.messages.find((message) => message.compaction);
    expect(marker?.compaction).toMatchObject({ summary: expect.stringContaining("Continue the fixture"), tokensBefore: expect.any(Number), estimatedTokensAfter: expect.any(Number) });
    expect(provider.requests).toHaveLength(0);
    const imageId = crypto.randomUUID();
    worker.send({ id: imageId, type: "message_image", entryId: "old-user", index: 0 });
    expect((await worker.waitFor((output) => output.type === "response" && output.id === imageId)).result).toEqual({ type: "image", mimeType: "image/png", data: image });
    await expectSavedHistory(compacted);
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "continue", message: "Continue after compaction" });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "continue" && output.state === "idle");
    expect(JSON.stringify(provider.requests[0].body)).not.toContain("Compacted source prompt");
    expect(JSON.stringify(provider.requests[0].body)).not.toContain("Latest source prompt");
    expect(JSON.stringify(provider.requests[0].body)).toContain("Continue the fixture");
    expect(worker.view?.messages.find((message) => message.id === marker?.id)?.compaction).toEqual(marker?.compaction);
    expect(worker.view?.runTimings).toHaveLength(3);
    await expectSavedHistory(worker.view!);
    const final = worker.view!;
    await worker.shutdown();
    const restored = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "compaction", sessionFile, undefined, resources, "build", true, { provider: command.provider });
    cleanup.push(() => restored.worker.shutdown());
    await restored.worker.waitFor((output) => emitted(output) && output.view?.messages.find((message) => message.id === "old-user")?.blocks.some((block) => block.type === "image" && block.thumbnail !== undefined) === true);
    expect(restored.worker.view?.messages).toEqual(final.messages);
    expect(restored.worker.view?.runTimings).toEqual(final.runTimings);
  });

  it("retains an automatic compaction boundary while the prompt resumes and settles", async () => {
    const { worker, provider } = await start();
    const startedAt = Date.now();
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "automatic", startedAt, message: "Resume the fixture" });
    const compacting = await worker.waitFor((output) => emitted(output) && output.view?.compaction?.reason === "threshold");
    expect(compacting.view?.activeRun).toEqual({ runId: "automatic", startedAt });
    const finished = await worker.waitFor((output) => output.type === "run_state" && output.runId === "automatic" && output.state === "idle");
    expect(finished.view?.compaction).toBeUndefined();
    expect(finished.view?.activeRun).toBeUndefined();
    expect(finished.view?.messages.filter((message) => message.role === "user").map((message) => message.id)).toContain("old-user");
    expect(finished.view?.messages.filter((message) => message.compaction)).toHaveLength(1);
    expect(finished.view?.runTimings).toHaveLength(3);
    expect(JSON.stringify(provider.requests[0].body)).not.toContain("Compacted source prompt");
    await expectSavedHistory(finished.view!);
  });

  it("continues the same run after overflow compaction without resurrecting the omitted error attempt", async () => {
    const { worker, provider } = await start("overflow");
    const startedAt = Date.now();
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "overflow", startedAt, message: "Overflow fixture" });
    const compacting = await worker.waitFor((output) => emitted(output) && output.view?.compaction?.reason === "overflow");
    expect(compacting.view?.activeRun).toEqual({ runId: "overflow", startedAt });
    const finished = await worker.waitFor((output) => output.type === "run_state" && output.runId === "overflow" && output.state === "idle");
    expect(finished.view?.messages.filter((message) => message.role === "user").map((message) => message.id)).toContain("old-user");
    expect(finished.view?.messages.find((message) => message.blocks.some((block) => block.text === "Overflow fixture"))?.timestamp).toBeGreaterThanOrEqual(startedAt);
    expect(finished.view?.messages.some((message) => message.stopReason === "error")).toBe(false);
    expect(finished.view?.messages.filter((message) => message.compaction)).toHaveLength(1);
    expect(finished.view?.runTimings).toHaveLength(3);
    expect(finished.view?.compaction).toBeUndefined();
    expect(finished.view?.activeRun).toBeUndefined();
    expect(provider.requests.length).toBeGreaterThan(1);
    expect(JSON.stringify(provider.requests[1].body)).not.toContain("Compacted source prompt");
    await expectSavedHistory(finished.view!);
  });

  it("ends compaction status on cancellation and failure without adding a successful marker", async () => {
    const { worker } = await start();
    worker.send({ id: crypto.randomUUID(), type: "compact", runId: "stopped-compact", instructions: "wait" });
    await worker.waitFor((output) => emitted(output) && output.view?.compaction?.reason === "manual");
    worker.send({ id: crypto.randomUUID(), type: "abort" });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "stopped-compact" && output.state === "idle");
    expect(worker.view?.compaction).toBeUndefined();
    expect(worker.view?.messages.some((message) => message.compaction)).toBe(false);
    expect(worker.outputs.some((output) => output.type === "extension_notice" && output.message === "Context compaction stopped.")).toBe(true);
    expect(worker.outputs.some((output) => output.type === "worker_error")).toBe(false);
    worker.send({ id: crypto.randomUUID(), type: "compact", runId: "completed-compact", instructions: "" });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "completed-compact" && output.state === "idle");
    worker.send({ id: crypto.randomUUID(), type: "compact", runId: "failed-compact", instructions: "" });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "failed-compact" && output.state === "idle");
    expect(worker.view?.compaction).toBeUndefined();
    expect(worker.outputs.find((output) => output.type === "run_finished" && output.runId === "failed-compact")).toMatchObject({ outcome: "failed" });
    expect(worker.view?.messages.filter((message) => message.compaction)).toHaveLength(1);
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
    await expectSavedHistory(restored.ready.snapshot!);
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
    type Message = SnapshotView["messages"][number];
    const thinkingOf = (message: Message | undefined) => message?.blocks.find((block) => block.type === "thinking");
    const promptStartedAt = Date.now();
    first.worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "think-run", message: "Think first." });
    const live = await first.worker.waitFor((output) => output.type === "partial"
      && typeof thinkingOf((output as unknown as { message?: Message }).message)?.startedAt === "number");
    const liveBlock = thinkingOf((live as unknown as { message: Message }).message);
    expect(liveBlock?.startedAt).toBeGreaterThanOrEqual(promptStartedAt);
    expect(liveBlock?.startedAt).toBeLessThanOrEqual(Date.now());
    expect(liveBlock).not.toHaveProperty("durationMs");
    await first.worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "think-run");
    const partials = first.worker.outputs
      .filter((output) => output.type === "partial")
      .map((output) => (output as unknown as { message: Message }).message);
    // While the model reasons, each delta retains the same live start and has no duration yet.
    const reasoning = partials.filter((message) => thinkingOf(message) && !message.blocks.some((block) => block.type === "text"));
    expect(reasoning.length).toBeGreaterThanOrEqual(2);
    expect(new Set(reasoning.map((message) => thinkingOf(message)?.startedAt))).toEqual(new Set([liveBlock?.startedAt]));
    expect(reasoning.every((message) => thinkingOf(message)?.durationMs === undefined)).toBe(true);
    // Once the answer starts, the streamed block already carries how long the reasoning took.
    const answering = partials.find((message) => message.blocks.some((block) => block.type === "text" && block.text));
    const streamedDuration = thinkingOf(answering)?.durationMs;
    expect(streamedDuration).toBeGreaterThanOrEqual(300);
    expect(thinkingOf(answering)).not.toHaveProperty("startedAt");

    // The saved message keeps the same duration, and the timestamp that ties it to its partial.
    const saved = first.worker.view?.messages.find((message) => message.role === "assistant");
    expect(thinkingOf(saved)?.durationMs).toBe(streamedDuration);
    expect(saved?.timestamp).toBe(answering?.timestamp);
    expect(thinkingOf(saved)).not.toHaveProperty("startedAt");

    const sessionFile = first.worker.view?.sessionFile;
    expect(sessionFile).toBeTruthy();
    const entries = (await readFile(sessionFile as string, "utf8")).trim().split("\n").map((line) => JSON.parse(line));
    const persistedThinking = entries.find((entry) => entry.customType === "wackcode-run-timing")?.data.thinking;
    expect(Object.values(persistedThinking ?? {})).toEqual([[streamedDuration]]);
    for (const entry of entries) if (entry.message) expect(JSON.stringify(entry.message)).not.toContain('"startedAt"');
    await expectSavedHistory(first.worker.view!);
    await first.worker.shutdown();
    const restored = await initializeWorker(provider.baseUrl, "think-secret", workspace, "think-task", sessionFile);
    cleanup.push(() => restored.worker.shutdown());
    const reopened = restored.ready.snapshot?.messages.find((message) => message.role === "assistant");
    expect(thinkingOf(reopened)?.text).toBe("Let me work this out.");
    expect(thinkingOf(reopened)?.durationMs).toBe(streamedDuration);
    expect(thinkingOf(reopened)).not.toHaveProperty("startedAt");
  });

  it("registers the built-in tools as wackcode sources that ignore the denylist", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-builtin-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    // Even a denylist that names them cannot switch built-in tools off.
    const { worker, ready } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "builtin-task", undefined,
      ["ask_user_question", "plan_mode_complete", "todo", "bash_job"]
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
    expect(catalog.find((tool) => tool.name === "bash_job")?.source.kind).toBe("wackcode");
    expect(ready.snapshot?.activeTools).toContain("bash_job");
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

  it("keeps computer use off until it is switched on, then round-trips a screenshot while the prompt holds the queue", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-computer-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker, ready } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "computer-task", undefined, undefined, undefined, undefined, true
    );
    cleanup.push(() => worker.shutdown());
    expect(ready.snapshot?.tools?.some((tool) => tool.name === "computer_screenshot")).toBe(true);
    expect(ready.snapshot?.activeTools?.some((name) => name.startsWith("computer_"))).toBe(false);

    worker.send({ id: crypto.randomUUID(), type: "set_computer_use", enabled: true });
    await worker.waitFor((output) => emitted(output) && output.view?.activeTools?.includes("computer_screenshot") === true);

    // Wider than the 480px transcript preview, so the preview is a resized copy.
    const shot = solidPng(1200, 30);
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "computer-run", message: 'mcp: computer_screenshot {"app":"TextEdit"}' });
    const request = await worker.waitFor((output) => output.type === "computer_request");
    expect(request.request).toEqual({ op: "screenshot", app: "TextEdit" });
    // The response reaches the tool even though the running prompt holds the command queue.
    worker.send({
      id: crypto.randomUUID(), type: "computer_response", requestId: request.requestId, success: true,
      result: {
        stateId: "s1", app: { name: "TextEdit", bundleId: "com.apple.TextEdit", pid: 42 },
        window: { id: 7, title: "Untitled", width: 32, height: 20 },
        image: { mimeType: "image/png", data: shot, width: 1200, height: 30 }, coordinateScale: 2
      }
    });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    expect(JSON.stringify(provider.requests[1].body.messages)).toContain(shot);

    // The transcript gets a preview on the result block; the original stays in the session.
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) =>
      message.blocks.some((block) => block.type === "tool-result" && block.images?.[0]?.thumbnail?.startsWith("data:image/"))) === true);
    const result = worker.view?.messages.flatMap((message) => message.blocks).find((block) => block.type === "tool-result" && block.images);
    expect(result?.images?.[0]?.imageId).toBeTruthy();
    expect(result?.text).toContain("stateId: s1");
    expect(JSON.stringify(worker.view)).not.toContain(shot);

    const id = crypto.randomUUID();
    worker.send({ id, type: "tool_image", toolCallId: result?.toolCallId });
    const original = await worker.waitFor((output) => output.type === "response" && output.id === id);
    expect(original.result).toEqual({ type: "image", data: shot, mimeType: "image/png" });

    worker.send({ id: crypto.randomUUID(), type: "set_computer_use", enabled: false });
    await worker.waitFor((output) => emitted(output) && output.view?.activeTools?.includes("computer_screenshot") === false);
    expect(worker.child.exitCode).toBeNull();
  });

  it("lets Plan mode look at an allowed app but never operate one", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-computer-plan-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "computer-plan", undefined, undefined, undefined, "plan", false,
      { computerUse: { enabled: true } }
    );
    cleanup.push(() => worker.shutdown());

    const before = provider.requests.length;
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "plan-act", message: 'mcp: computer_act {"app":"TextEdit","stateId":"s1","actions":[{"kind":"press","ref":"e1-0"}]}' });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && provider.requests.length > before + 1);
    expect(JSON.stringify(provider.requests[before + 1].body.messages)).toContain("Plan mode may look at an app you've allowed but cannot open or operate one.");
    expect(worker.outputs.some((output) => output.type === "computer_request")).toBe(false);

    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "plan-look", message: 'mcp: computer_snapshot {"app":"TextEdit"}' });
    const request = await worker.waitFor((output) => output.type === "computer_request");
    expect(request.request).toEqual({ op: "snapshot", app: "TextEdit" });
    worker.send({ id: crypto.randomUUID(), type: "computer_response", requestId: request.requestId, success: false, error: "TextEdit isn't running." });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "plan-look");
  });

  it("cancels a pending computer-use request when the run is stopped", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-computer-stop-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "computer-stop", undefined, undefined, undefined, undefined, false,
      { computerUse: { enabled: true } }
    );
    cleanup.push(() => worker.shutdown());
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "stop-run", message: "mcp: computer_apps {}" });
    const request = await worker.waitFor((output) => output.type === "computer_request");
    expect(request.request).toEqual({ op: "apps" });
    worker.send({ id: crypto.randomUUID(), type: "abort" });
    const cancel = await worker.waitFor((output) => output.type === "computer_cancel");
    expect(cancel.requestId).toBe(request.requestId);
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
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

    const resolved = await worker.waitFor((output) => output.type === "extension_ui_resolved" && output.requestId === request.requestId);
    expect(resolved.cancelled).toBe(true);

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
  it.each(["plan", "ultraplan"] as const)("applies and withdraws unrestricted %s access without persisting guidance", async (mode) => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const fixture = await mkdtemp(join(tmpdir(), "wackcode-plan-access-"));
    cleanup.push(() => rm(fixture, { recursive: true, force: true }));
    const workspace = join(fixture, "project");
    await mkdir(workspace);
    const enabled = { unrestrictedSubagents: false, unrestrictedPlanning: true };
    const { worker, ready } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "plan-access", undefined, undefined, undefined, mode, undefined, { executionPolicy: enabled });
    cleanup.push(() => worker.shutdown());
    expect(ready.snapshot?.executionPolicy).toEqual(enabled);
    const run = async (runId: string, message: string) => {
      worker.send({ id: crypto.randomUUID(), type: "prompt", runId, message });
      await worker.waitFor((o) => o.type === "run_state" && o.runId === runId && o.state === "idle");
    };
    await run("write", 'mcp: write {"path":"unlocked.txt","content":"changed"}');
    expect(await readFile(join(workspace, "unlocked.txt"), "utf8")).toBe("changed");
    const outside = join(fixture, "outside.txt");
    await run("outside", `mcp: write ${JSON.stringify({ path: outside, content: "outside the project" })}`);
    expect(await readFile(outside, "utf8")).toBe("outside the project");
    await run("shell", 'mcp: bash {"command":"printf shell > shell.txt"}');
    expect(await readFile(join(workspace, "shell.txt"), "utf8")).toBe("shell");
    await run("complete", "finish plan");
    expect(worker.view?.planState).toMatchObject({ mode, phase: "ready" });
    for (const request of provider.requests) {
      expect(JSON.stringify(request.body.messages).match(/WACKCODE CURRENT PLANNING ACCESS POLICY/g)).toHaveLength(1);
    }
    const saved = await readFile(worker.view!.sessionFile!, "utf8");
    expect(saved).not.toContain("wackcode-execution-policy");
    expect(saved).not.toContain("WACKCODE CURRENT PLANNING ACCESS POLICY");
    worker.send({ id: crypto.randomUUID(), type: "set_execution_policy", executionPolicy: { unrestrictedPlanning: false, unrestrictedSubagents: false } });
    await worker.waitFor((o) => o.type === "snapshot" && o.snapshot?.executionPolicy?.unrestrictedPlanning === false);
    const before = provider.requests.length;
    await run("restricted", 'mcp: write {"path":"blocked.txt","content":"blocked"}');
    await expect(readFile(join(workspace, "blocked.txt"), "utf8")).rejects.toThrow();
    expect(JSON.stringify(provider.requests[before].body.messages)).not.toContain("WACKCODE CURRENT PLANNING ACCESS POLICY");
    // Rewinding into an unrestricted turn restores conversation state, never its permissions.
    const original = worker.view!.messages.find((message) => message.role === "user")!;
    const rewindId = crypto.randomUUID();
    worker.send({ id: rewindId, type: "navigate", entryId: original.entryId, target: "before", kind: "rewind" });
    expect((await worker.waitFor((o) => o.type === "response" && o.id === rewindId)).success).toBe(true);
    expect(worker.view?.executionPolicy).toEqual({ unrestrictedPlanning: false, unrestrictedSubagents: false });
    await run("after-rewind", 'mcp: write {"path":"rewound.txt","content":"blocked"}');
    await expect(readFile(join(workspace, "rewound.txt"), "utf8")).rejects.toThrow();
    expect(JSON.stringify(provider.requests.at(-1)!.body.messages)).not.toContain("WACKCODE CURRENT PLANNING ACCESS POLICY");
    const sessionFile = worker.view!.sessionFile;
    await worker.shutdown();
    const restarted = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "plan-access", sessionFile, undefined, undefined, mode);
    cleanup.push(() => restarted.worker.shutdown());
    expect(restarted.ready.snapshot?.executionPolicy).toEqual({ unrestrictedPlanning: false, unrestrictedSubagents: false });
    expect(restarted.ready.snapshot?.planState?.mode).toBe(mode);
  });

  it.each(["plan", "ultraplan"] as const)("allows enabled package, browser and computer actions in unrestricted %s, retaining their own controls", async (mode) => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-plan-tools-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const trusted = join(workspace, "trusted.ts");
    await writeFile(trusted, `export default function(pi: any) { pi.registerTool({ name: "trusted_tool", label: "Trusted", description: "Explicitly loaded fixture", parameters: { type: "object", properties: {} }, async execute() { return { content: [{ type: "text", text: "trusted tool ran" }] }; } }); }`);
    await mkdir(join(workspace, ".pi", "extensions"), { recursive: true });
    await writeFile(join(workspace, ".pi", "extensions", "untrusted.ts"), `throw new Error("Untrusted project extension loaded");`);
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "plan-tools", undefined, undefined,
      { extensions: [trusted], skills: [], prompts: [], themes: [] }, mode, undefined,
      { executionPolicy: { unrestrictedPlanning: true, unrestrictedSubagents: false }, computerUse: { enabled: true } });
    cleanup.push(() => worker.shutdown());
    const run = async (runId: string, message: string, native?: "browser" | "computer") => {
      const mark = worker.outputs.length;
      const before = provider.requests.length;
      worker.send({ id: crypto.randomUUID(), type: "prompt", runId, message });
      if (native) {
        const request = await worker.waitFor((o) => worker.outputs.indexOf(o) >= mark && o.type === `${native}_request`);
        // The native access boundary can still refuse an action; no real app/browser is operated.
        worker.send({ id: crypto.randomUUID(), type: `${native}_response`, requestId: request.requestId, success: false, error: "Fixture access denied by native controls." });
      }
      await worker.waitFor((o) => o.type === "run_state" && o.runId === runId && o.state === "idle");
      return provider.requests.slice(before);
    };
    const installed = await run("package", "mcp: trusted_tool {}");
    expect(JSON.stringify(installed.at(-1)!.body.messages)).toContain("trusted tool ran");
    for (const [id, message, native] of [
      ["browser-open", 'mcp: browser_open {"url":"http://localhost/fixture"}', "browser"],
      ["browser-act", 'mcp: browser_act {"kind":"reload"}', "browser"],
      ["computer-open", 'mcp: computer_open {"app":"TextEdit"}', "computer"],
      ["computer-act", 'mcp: computer_act {"app":"TextEdit","stateId":"s1","actions":[{"kind":"press","ref":"e1-0"}]}', "computer"],
    ] as const) {
      const requests = await run(id, message, native);
      expect(JSON.stringify(requests.at(-1)!.body.messages)).toContain("Fixture access denied by native controls.");
    }
    const mark = worker.outputs.length;
    worker.send({ id: crypto.randomUUID(), type: "set_tools", disabledTools: ["trusted_tool", "browser_act"] });
    worker.send({ id: crypto.randomUUID(), type: "set_computer_use", enabled: false });
    await worker.waitFor((o) => worker.outputs.indexOf(o) >= mark && o.type === "snapshot" && !o.snapshot?.activeTools?.includes("computer_act"));
    const disabled = await run("disabled", "mcp: trusted_tool {}");
    const offered = (disabled[0].body.tools as Array<{ function: { name: string } }>).map((tool) => tool.function.name);
    for (const name of ["trusted_tool", "browser_act", "computer_act", "computer_open"]) expect(offered).not.toContain(name);
    expect(worker.outputs.some((o) => o.type === "worker_error")).toBe(false);
    const initId = crypto.randomUUID();
    worker.send({ id: initId, type: "init_agents", runId: "init-refused" });
    expect(await worker.waitFor((o) => o.type === "response" && o.id === initId)).toMatchObject({ success: false, error: expect.stringContaining("Build mode") });
    const goalId = crypto.randomUUID();
    worker.send({ id: goalId, type: "goal_control", action: "set", objective: "Implement the fixture", runId: "goal-refused" });
    expect(await worker.waitFor((o) => o.type === "response" && o.id === goalId)).toMatchObject({ success: false, error: expect.stringContaining("Switch to Build first") });
  });

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

  it("serves the lightbox the original of a sent image by entry and position", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-message-image-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "message-image-task", undefined, undefined, undefined, undefined, true
    );
    cleanup.push(() => worker.shutdown());
    // Wider than the 512px transcript preview (so the preview is a resized copy), yet within
    // Pi's 2000px inline limit (so the session keeps the exact bytes the model received).
    const first = solidPng(600, 30);
    const second = solidPng(640, 24);
    worker.send({
      id: crypto.randomUUID(), type: "prompt", runId: "message-image-run", message: "Two for you.",
      images: [
        { type: "image", data: first, mimeType: "image/png" },
        { type: "image", data: second, mimeType: "image/png" }
      ]
    });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) =>
      message.role === "user" && message.blocks.filter((block) => block.type === "image").length === 2
      && message.blocks.filter((block) => block.type === "image").every((block) => Boolean(block.thumbnail))) === true);
    const user = worker.view?.messages.find((message) => message.role === "user");
    expect(user?.entryId).toBeTruthy();

    async function request(command: Record<string, unknown>): Promise<Output> {
      const id = crypto.randomUUID();
      worker.send({ id, ...command });
      return worker.waitFor((output) => output.type === "response" && output.id === id);
    }

    // Each image comes back at its own position, while snapshots keep carrying previews alone.
    const zero = await request({ type: "message_image", entryId: user?.entryId, index: 0 });
    expect(zero.result).toEqual({ type: "image", data: first, mimeType: "image/png" });
    const one = await request({ type: "message_image", entryId: user?.entryId, index: 1 });
    expect(one.result).toEqual({ type: "image", data: second, mimeType: "image/png" });
    expect(JSON.stringify(worker.view)).not.toContain(second);

    // A wrong position or entry finds nothing instead of another image.
    expect((await request({ type: "message_image", entryId: user?.entryId, index: 2 })).result).toBeNull();
    expect((await request({ type: "message_image", entryId: "missing", index: 0 })).result).toBeNull();
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
    const history = await readSavedSession({ sessionFile, taskId: "image-restore", mode: "build", thinkingLevel: "off" });
    expect(history.messages.find((message) => message.role === "user")?.blocks).toContainEqual(expect.objectContaining({ type: "image", thumbnail: expect.stringMatching(/^data:image\//) }));
    expect(await readFile(sessionFile as string, "utf8")).toBe(stored);

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
    expect(worker.view?.messages.find((message) => message.role === "user")?.commandPresentation).toEqual({
      id: "app:init", name: "init", arguments: "", kind: "command"
    });

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
    await expectSavedHistory(back!);
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
    await expectSavedHistory(empty!);
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
    const resources = { extensions: [extension], skills: [skill], prompts: [prompt], themes: [] };
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "command-task", undefined, undefined, resources);
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
    const templateMessage = [...(worker.view?.messages ?? [])].reverse().find((message) => message.role === "user");
    expect(templateMessage?.commandPresentation).toEqual({
      id: template.id, name: "prompt:review", arguments: '"changed files"', kind: "command"
    });
    expect((await request(worker, { type: "resend", runId: "template-retry", entryId: templateMessage!.entryId })).success).toBe(true);
    await settle(worker, "template-retry");
    const retriedTemplate = [...(worker.view?.messages ?? [])].reverse().find((message) => message.role === "user");
    expect(retriedTemplate?.commandPresentation).toEqual(templateMessage?.commandPresentation);
    expect((await request(worker, { type: "resend", runId: "template-edit", entryId: retriedTemplate!.entryId, message: "Review edited files" })).success).toBe(true);
    await settle(worker, "template-edit");
    expect([...(worker.view?.messages ?? [])].reverse().find((message) => message.role === "user")?.commandPresentation).toBeUndefined();
    const selectedSkill = commands.find((entry) => entry.name === "skill:fixture")!;
    expect((await request(worker, { type: "execute_command", commandId: selectedSkill.id, args: "details", runId: "skill-1" })).success).toBe(true);
    await settle(worker, "skill-1");
    expect(lastUserText(provider)).toContain("<skill name=\"fixture\"");
    expect([...(worker.view?.messages ?? [])].reverse().find((message) => message.role === "user")?.commandPresentation).toEqual({
      id: selectedSkill.id, name: "skill:fixture", arguments: "details", kind: "command"
    });

    const sessionFile = worker.view?.sessionFile;
    expect(sessionFile).toBeTruthy();
    await expectSavedHistory(worker.view!);
    await worker.shutdown();
    const restored = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "command-task", sessionFile, undefined, resources);
    cleanup.push(() => restored.worker.shutdown());
    expect([...(restored.ready.snapshot?.messages ?? [])].reverse().find((message) => message.role === "user")?.commandPresentation).toEqual({
      id: selectedSkill.id, name: "skill:fixture", arguments: "details", kind: "command"
    });
  });
});

describe("skills (Settings › Skills)", () => {
  async function request(worker: WorkerHarness, command: Record<string, unknown>): Promise<Output> {
    const id = crypto.randomUUID();
    worker.send({ id, ...command });
    return worker.waitFor((output) => output.type === "response" && output.id === id);
  }

  async function skills(worker: WorkerHarness): Promise<Array<{ name: string; sourceLabel: string }>> {
    const listed = await request(worker, { type: "list_commands" });
    return (listed.result as unknown as Array<{ name: string; source: string; sourceLabel: string }>)
      .filter((entry) => entry.source === "skill")
      .map(({ name, sourceLabel }) => ({ name, sourceLabel }));
  }

  async function run(worker: WorkerHarness, runId: string, message = "Go."): Promise<void> {
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId, message });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === runId);
  }

  // Every system and developer message in a request, joined: a mid-chat change arrives as a
  // later system message patching the sections, after the original prompt.
  const systemTextOf = (source: MockProvider, index: number) =>
    ((source.requests[index].body.messages ?? []) as { role?: string; content?: unknown }[])
      .filter((message) => message.role === "system" || message.role === "developer")
      .map((message) => typeof message.content === "string"
        ? message.content
        : Array.isArray(message.content) ? message.content.map((part) => (part as { text?: string }).text ?? "").join("") : "")
      .join("\n---\n");

  async function writeSkill(root: string, name: string, description: string): Promise<string> {
    await mkdir(join(root, name), { recursive: true });
    const file = join(root, name, "SKILL.md");
    await writeFile(file, `---\nname: ${name}\ndescription: ${description}\n---\nFollow ${name}.\n`);
    return file;
  }

  it("loads the user's folders in order, labels them, and applies set_skills live", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-skills-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const home = await mkdtemp(join(tmpdir(), "wackcode-skills-home-"));
    cleanup.push(() => rm(home, { recursive: true, force: true }));
    const library = join(home, ".agents", "skills");
    const claude = join(home, ".claude", "skills");
    const mine = await writeSkill(library, "alpha", "MINE-ALPHA-DESCRIPTION");
    await writeSkill(claude, "alpha", "CLAUDE-ALPHA-DESCRIPTION");
    await writeSkill(claude, "beta", "CLAUDE-BETA-DESCRIPTION");
    const roots = [{ path: library, label: "Your skills" }, { path: claude, label: "Claude Code" }];
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "skills-task", undefined, undefined, undefined, undefined, undefined,
      { skills: { roots, disabled: [] } }
    );
    cleanup.push(() => worker.shutdown());

    expect(await skills(worker)).toEqual([{ name: "skill:alpha", sourceLabel: "Your skills" }, { name: "skill:beta", sourceLabel: "Claude Code" }]);
    await run(worker, "run-1");
    const first = systemTextOf(provider, 0);
    expect(first).toContain("MINE-ALPHA-DESCRIPTION");
    expect(first).toContain("CLAUDE-BETA-DESCRIPTION");
    expect(first).not.toContain("CLAUDE-ALPHA-DESCRIPTION");

    // Switching the user's alpha off lets Claude Code's alpha load instead, on the next turn.
    expect((await request(worker, { type: "set_skills", skills: { roots, disabled: [mine] } })).success).toBe(true);
    expect(await skills(worker)).toEqual([{ name: "skill:alpha", sourceLabel: "Claude Code" }, { name: "skill:beta", sourceLabel: "Claude Code" }]);
    await run(worker, "run-2");
    const after = systemTextOf(provider, provider.requests.length - 1);
    expect(after.lastIndexOf("CLAUDE-ALPHA-DESCRIPTION")).toBeGreaterThan(after.lastIndexOf("MINE-ALPHA-DESCRIPTION"));

    // Switching Claude Code's folder off takes all of its skills away.
    expect((await request(worker, { type: "set_skills", skills: { roots: roots.slice(0, 1), disabled: [] } })).success).toBe(true);
    expect(await skills(worker)).toEqual([{ name: "skill:alpha", sourceLabel: "Your skills" }]);
  });

  it("picks up a skill added on disk at the next run", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-skills-disk-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const library = join(workspace, "home-skills");
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "skills-disk-task", undefined, undefined, undefined, undefined, undefined,
      { skills: { roots: [{ path: library, label: "Your skills" }], disabled: [] } }
    );
    cleanup.push(() => worker.shutdown());
    await run(worker, "run-1");
    expect(systemTextOf(provider, 0)).not.toContain("LATE-SKILL-DESCRIPTION");

    await writeSkill(library, "late", "LATE-SKILL-DESCRIPTION");
    await run(worker, "run-2");
    expect(systemTextOf(provider, provider.requests.length - 1)).toContain("LATE-SKILL-DESCRIPTION");
    expect(await skills(worker)).toEqual([{ name: "skill:late", sourceLabel: "Your skills" }]);
  });

  it("never loads a project's own skill folders", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-skills-project-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    await writeSkill(join(workspace, ".agents", "skills"), "project-agents", "PROJECT-AGENTS-DESCRIPTION");
    await writeSkill(join(workspace, ".pi", "skills"), "project-pi", "PROJECT-PI-DESCRIPTION");
    await writeSkill(join(workspace, ".claude", "skills"), "project-claude", "PROJECT-CLAUDE-DESCRIPTION");
    const library = join(workspace, "..", `${workspace.split("/").pop()}-library`);
    cleanup.push(() => rm(library, { recursive: true, force: true }));
    await writeSkill(library, "mine", "MINE-DESCRIPTION");
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "skills-project-task", undefined, undefined, undefined, undefined, undefined,
      { skills: { roots: [{ path: library, label: "Your skills" }], disabled: [] } }
    );
    cleanup.push(() => worker.shutdown());
    expect(await skills(worker)).toEqual([{ name: "skill:mine", sourceLabel: "Your skills" }]);
    await run(worker, "run-1");
    const prompt = systemTextOf(provider, 0);
    expect(prompt).toContain("MINE-DESCRIPTION");
    expect(prompt).not.toContain("PROJECT-");
  });
});

describe("memory (Settings › Memory)", () => {
  async function request(worker: WorkerHarness, command: Record<string, unknown>): Promise<Output> {
    const id = crypto.randomUUID();
    worker.send({ id, ...command });
    return worker.waitFor((output) => output.type === "response" && output.id === id);
  }

  async function run(worker: WorkerHarness, runId: string): Promise<void> {
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId, message: "Go." });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === runId);
  }

  async function toolNames(provider: MockProvider): Promise<string[]> {
    return ((provider.requests[provider.requests.length - 1].body.tools ?? []) as { function?: { name?: string } }[])
      .map((tool) => tool.function?.name ?? "");
  }

  const systemTextOf = (source: MockProvider, index: number) =>
    ((source.requests[index].body.messages ?? []) as { role?: string; content?: unknown }[])
      .filter((message) => message.role === "system" || message.role === "developer")
      .map((message) => typeof message.content === "string"
        ? message.content
        : Array.isArray(message.content) ? message.content.map((part) => (part as { text?: string }).text ?? "").join("") : "")
      .join("\n---\n");

  it("injects the index, reads notes on disk between runs, and applies set_memory live", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-memory-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const memoryRoot = join(workspace, "memory");
    await mkdir(memoryRoot, { recursive: true });
    await writeFile(join(memoryRoot, "feedback_run-worker-tests.md"),
      "---\ntype: feedback\ntitle: Run worker tests\ndescription: Protocol edits need pnpm test:worker\nmodified: 2026-09-28T10:12:00.000Z\n---\nAny protocol.ts change needs pnpm test:worker.\n");
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "memory-task", undefined, undefined, undefined, undefined, undefined,
      { memory: { root: memoryRoot, enabled: true } }
    );
    cleanup.push(() => worker.shutdown());

    await run(worker, "run-1");
    const prompt = systemTextOf(provider, 0);
    expect(prompt).toContain("## Project memory");
    expect(prompt).toContain("- [feedback] Run worker tests — Protocol edits need pnpm test:worker (name: feedback_run-worker-tests)");
    // The body itself is never injected — only the one-line index.
    expect(prompt).not.toContain("Any protocol.ts change needs pnpm test:worker.");
    expect(await toolNames(provider)).toContain("memory_save");

    // A note written on disk between runs (Settings, or another chat in the same project)
    // reaches the next run's index.
    await writeFile(join(memoryRoot, "user_prefers.md"),
      "---\ntype: user\ntitle: Prefers terse answers\ndescription: No filler\n---\nSkip preamble.\n");
    await run(worker, "run-2");
    const second = systemTextOf(provider, provider.requests.length - 1);
    expect(second).toContain("(name: user_prefers)");

    // Switching memory off withdraws the tools and the section on the next turn.
    expect((await request(worker, { type: "set_memory", memory: null })).success).toBe(true);
    await run(worker, "run-3");
    const names = await toolNames(provider);
    expect(names).not.toContain("memory_save");
    expect(names).not.toContain("memory_recall");
    expect(systemTextOf(provider, provider.requests.length - 1)).not.toContain("## Project memory");
  });

  it("offers no memory tools and no section while off at init", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-memory-off-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "memory-off-task", undefined, undefined, undefined, undefined, undefined,
      { memory: { root: join(workspace, "memory"), enabled: false } }
    );
    cleanup.push(() => worker.shutdown());
    await run(worker, "run-1");
    const names = await toolNames(provider);
    expect(names).not.toContain("memory_save");
    expect(systemTextOf(provider, 0)).not.toContain("## Project memory");
  });

  it("activates the tools on an empty project switched on live", async () => {
    // Regression: the index text is identical (empty) on and off, so a toggle gated on the
    // index changing would leave the tools inactive forever — and the agent could never save
    // the note that would have created them.
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-memory-live-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const memoryRoot = join(workspace, "memory");
    await mkdir(memoryRoot, { recursive: true });
    const { worker } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, "memory-live-task", undefined, undefined, undefined, undefined, undefined,
      { memory: { root: memoryRoot, enabled: false } }
    );
    cleanup.push(() => worker.shutdown());
    await run(worker, "run-1");
    expect(await toolNames(provider)).not.toContain("memory_save");
    expect((await request(worker, { type: "set_memory", memory: { root: memoryRoot, enabled: true } })).success).toBe(true);
    await run(worker, "run-2");
    expect(await toolNames(provider)).toContain("memory_save");
  });
});

describe("commands (Settings › Commands)", () => {
  async function request(worker: WorkerHarness, command: Record<string, unknown>): Promise<Output> {
    const id = crypto.randomUUID();
    worker.send({ id, ...command });
    return worker.waitFor((output) => output.type === "response" && output.id === id);
  }

  async function settle(worker: WorkerHarness, runId: string): Promise<SnapshotView> {
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === runId);
    if (!worker.view) throw new Error("No snapshot after the run");
    return worker.view;
  }

  function lastUserText(provider: MockProvider): string {
    return userParts(provider.requests[provider.requests.length - 1]).filter((part) => part.type === "text").map((part) => part.text).join("");
  }

  it("lists custom commands ahead of package templates, applies set_commands live", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-commands-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const dir = join(workspace, "commands");
    await mkdir(dir, { recursive: true });
    const mine = join(dir, "review.md");
    await writeFile(mine, "---\ndescription: Mine\nargument-hint: <files>\n---\nReview $ARGUMENTS\n");
    const theirs = join(workspace, "review.md");
    await writeFile(theirs, "---\ndescription: Theirs\n---\nPackage review.\n");
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "commands-task", undefined, undefined,
      { extensions: [], skills: [], prompts: [theirs], themes: [] }, undefined, undefined,
      { commands: { dir, disabled: [] } });
    cleanup.push(() => worker.shutdown());

    const listed = await request(worker, { type: "list_commands" });
    const commands = listed.result as unknown as Array<{ id: string; name: string; source: string; sourceLabel: string; argumentHint?: string }>;
    const custom = commands.find((entry) => entry.source === "custom");
    expect(custom).toEqual(expect.objectContaining({ name: "review", argumentHint: "<files>", sourceLabel: "Your commands", id: `custom:${mine}` }));
    // The user's `review` wins the name; the package template keeps working renamed.
    expect(commands.find((entry) => entry.source === "prompt")).toEqual(expect.objectContaining({ name: "prompt:review" }));

    expect((await request(worker, { type: "execute_command", commandId: custom!.id, args: "the diff", runId: "cmd-1" })).success).toBe(true);
    await settle(worker, "cmd-1");
    expect(lastUserText(provider)).toContain("Review the diff");

    // Switching it off frees the name and refuses the stale key with a clear reason.
    expect((await request(worker, { type: "set_commands", commands: { dir, disabled: [custom!.id] } })).success).toBe(true);
    const after = (await request(worker, { type: "list_commands" })).result as unknown as Array<{ name: string; source: string }>;
    expect(after.find((entry) => entry.source === "custom")).toBeUndefined();
    expect(after.find((entry) => entry.source === "prompt")).toEqual(expect.objectContaining({ name: "review" }));
    const refused = await request(worker, { type: "execute_command", commandId: custom!.id, args: "", runId: "cmd-2" });
    expect(refused.success).toBe(false);
    expect(refused.error).toMatch(/switched off/);
  });

  it("picks up a command added on disk at the next run, and a switched-off template never expands", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-commands-disk-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const dir = join(workspace, "commands");
    await mkdir(dir, { recursive: true });
    const theirs = join(workspace, "brief.md");
    await writeFile(theirs, "---\ndescription: Brief\n---\nBRIEF-EXPANDED $ARGUMENTS\n");
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "commands-disk-task", undefined, undefined,
      { extensions: [], skills: [], prompts: [theirs], themes: [] }, undefined, undefined,
      { commands: { dir, disabled: [`prompt:${theirs}`] } });
    cleanup.push(() => worker.shutdown());

    // A switched-off package template does not expand when its name is typed as a message.
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "raw-1", message: "/brief notes" });
    await settle(worker, "raw-1");
    expect(lastUserText(provider)).toContain("/brief notes");
    expect(lastUserText(provider)).not.toContain("BRIEF-EXPANDED");

    await writeFile(join(dir, "late.md"), "---\ndescription: LATE-DESCRIPTION\n---\nDo it.\n");
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "raw-2", message: "check" });
    await settle(worker, "raw-2");
    const commands = (await request(worker, { type: "list_commands" })).result as unknown as Array<{ name: string; source: string }>;
    expect(commands).toEqual(expect.arrayContaining([expect.objectContaining({ name: "late", source: "custom" })]));
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
    background?: boolean;
    results: Array<{ jobId?: string; agent: string; task: string; readOnly: boolean; status: string; model?: string; output?: string; error?: string; activity: Array<{ tool: string; subject: string }>; usage: { input: number; output: number; turns: number } }>;
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

  async function request(worker: WorkerHarness, command: Record<string, unknown>): Promise<Output> {
    const id = crypto.randomUUID();
    worker.send({ id, ...command });
    return worker.waitFor((output) => output.type === "response" && output.id === id);
  }

  const backgroundPrompt = (tag: string, launch: Record<string, unknown>, next?: { name: string; args: Record<string, unknown> }) => `subagent background: ${JSON.stringify({ tag, launch, next })}`;
  const parentRequests = (provider: MockProvider) => provider.requests.filter((request) => !JSON.stringify(request.body.messages).includes("SCOUT-PROMPT-MARKER") && !JSON.stringify(request.body.messages).includes("WORKER-PROMPT-MARKER"));
  const backgroundResultText = (provider: MockProvider) => parentRequests(provider).filter((request) => JSON.stringify(request.body.messages).includes("Job "));

  it("the live UI mock launches once, settles the parent, and resumes once without a launch loop", async () => {
    const mock = spawn(process.execPath, [resolve("../scripts/mock-provider.mjs")], { env: { ...process.env, WACKCODE_MOCK_PORT: "0", WACKCODE_MOCK_DELAY_MS: "1000" }, stdio: ["ignore", "pipe", "pipe"] });
    cleanup.push(async () => { mock.kill("SIGKILL"); });
    const baseUrl = await new Promise<string>((resolve, reject) => {
      mock.once("error", reject);
      mock.once("exit", () => reject(new Error("UI mock exited before startup")));
      mock.stdout.on("data", (data: Buffer) => { const match = /http:\/\/127\.0\.0\.1:\d+\/v1/.exec(data.toString()); if (match) resolve(match[0]); });
    });
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-live-mock-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(baseUrl, "mock-only-no-credential", workspace, "live-mock", undefined, undefined, undefined, undefined, undefined, { subagents: config({ agents: [editor] }) });
    cleanup.push(() => worker.shutdown());
    const launches = () => worker.view?.messages.flatMap((message) => message.blocks).filter((block) => block.type === "tool-call" && block.toolName === "subagent") ?? [];
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "live-mock-stop", message: "background fixture until stopped" });
    await worker.waitFor((output) => output.workActivity?.parent === "idle" && output.workActivity.subagents === 1);
    expect(launches()).toHaveLength(1);
    worker.send({ id: crypto.randomUUID(), type: "abort" });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "live-mock-finish", message: "background fixture" });
    await worker.waitFor((output) => output.workActivity?.parent === "idle" && output.workActivity.subagents === 1 && launches().length === 2);
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "The background child finished; its result has arrived.")) === true);
    expect(launches()).toHaveLength(2);
    expect(worker.outputs.filter((output) => output.type === "worker_error")).toEqual([]);
  });

  it("answers before background children finish, does parent work, then automatically resumes with each result and durable cards", async () => {
    const { provider, workspace, worker } = await start("subagents-background", { subagents: config() });
    const gate = provider.holdChildren(2, true);
    const prompt = backgroundPrompt("work", { tasks: [{ agent: "worker", task: "child-write: background-one" }, { agent: "scout", task: "child-ls: background-two" }] }, { name: "write", args: { path: "parent.txt", content: "parent continued" } });
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "background-work", message: prompt });
    await gate.arrived;
    await worker.waitFor((output) => output.type === "run_state" && output.workActivity?.parent === "idle" && output.workActivity.subagents === 2);
    expect(await readFile(join(workspace, "parent.txt"), "utf8")).toBe("parent continued");
    expect(await readFile(join(workspace, "background-one.txt"), "utf8")).toBe("from a sub-agent\n");
    expect(subagentResult(worker)?.details.background).toBe(true);
    expect(subagentResult(worker)?.details.results.map((child) => child.status)).toEqual(["running", "running"]);
    expect(worker.outputs.filter((output) => output.type === "run_state" && output.state === "idle")).toEqual([]);
    const launch = worker.view?.messages.find((message) => message.blocks.some((block) => block.toolName === "subagent" && block.type === "tool-call"));
    expect(launch).toBeDefined();
    const refused = await request(worker, { type: "navigate", kind: "rewind", entryId: launch!.entryId });
    expect(refused.success).toBe(false);
    expect(refused.error).toContain("sub-agents");
    gate.release();
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    await worker.waitFor((output) => emitted(output) && output.view?.messages.filter((message) => message.blocks.some((block) => block.text === "Finished alpha.")).length! >= 2);
    expect(subagentResult(worker)?.details.results.map((child) => child.status)).toEqual(["done", "done"]);
    expect(backgroundResultText(provider).length).toBeGreaterThan(0);
    const persisted = await readFile(worker.view!.sessionFile!, "utf8");
    expect(persisted).toContain('"customType":"wackcode-subagent-background"');
    expect(persisted).toContain('"customType":"wackcode-subagent-results"');
    const delivered = persisted.split("\n").filter((line) => line.includes('"customType":"wackcode-subagent-results"')).map((line) => JSON.parse(line));
    expect(delivered.flatMap((entry) => entry.details.jobIds)).toHaveLength(2);
    expect(JSON.stringify(worker.view)).not.toContain('"transcript"');
    await expectSavedHistory(worker.view!);
    worker.send({ id: crypto.randomUUID(), type: "watch_subagent", target: { toolCallId: "call-background-work-0", index: 0 } });
    const transcript = await worker.waitFor((output) => output.type === "subagent_stream" && output.toolCallId === "call-background-work-0" && output.reset === true);
    expect(transcript.live).toBe(false);
    expect(JSON.stringify(transcript.upserts)).toContain("background-one.txt");
  });

  it("an explicit wait owns completed results, including failures, without an extra automatic continuation", async () => {
    const { provider, worker } = await start("subagents-background-wait", { subagents: config() });
    const gate = provider.holdChildren(2);
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "background-wait", message: backgroundPrompt("wait", { tasks: [{ agent: "scout", task: "child-ls: wait-target" }, { agent: "worker", task: "child-fail: wait-target" }] }, { name: "subagent_job", args: { action: "wait" } }) });
    await gate.arrived;
    await worker.waitFor((output) => output.workActivity?.parent === "waiting");
    gate.release();
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    expect(subagentResult(worker)?.details.results.map((child) => child.status)).toEqual(["done", "failed"]);
    const finalRequest = parentRequests(provider).at(-1)!;
    const results = toolResultContents(finalRequest.body);
    expect(results.at(-1)).toContain("Fixture child failure.");
    expect(backgroundResultText(provider)).toHaveLength(1); // The wait's tool result contains Job IDs.
    const persisted = await readFile(worker.view!.sessionFile!, "utf8");
    expect(persisted).not.toContain('"customType":"wackcode-subagent-results"');
    expect(worker.outputs.filter((output) => output.type === "worker_error")).toEqual([]);
  });

  it("delivers a child completion once at an active parent's next safe boundary", async () => {
    const { provider, worker } = await start("subagents-background-active", { subagents: config() });
    const child = provider.holdChildren(1, true);
    const parent = provider.holdRequest((entry) => entry.text.startsWith("subagent background:") && hasToolMessage(entry.body));
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "background-active", message: backgroundPrompt("active", { agent: "scout", task: "child-ls: active-parent" }) });
    await Promise.all([child.arrived, parent.arrived]);
    child.release();
    await worker.waitFor((output) => output.workActivity?.parent === "running" && output.workActivity.pendingResults === 1);
    parent.release();
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    const saved = await readFile(worker.view!.sessionFile!, "utf8");
    const deliveries = saved.split("\n").filter((line) => line.includes('"customType":"wackcode-subagent-results"')).map((line) => JSON.parse(line));
    expect(deliveries.flatMap((entry) => entry.details.jobIds)).toHaveLength(1);
    expect(subagentResult(worker)?.details.results[0].status).toBe("done");
    expect(worker.outputs.filter((output) => output.type === "worker_error")).toEqual([]);
  });

  it("refuses terminal plans while a background helper is outstanding", async () => {
    const { provider, worker } = await start("subagents-background-plan", { subagents: config() }, "plan");
    const child = provider.holdChildren(1);
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "background-plan", message: backgroundPrompt("plan", { agent: "scout", task: "child-ls: before-plan" }, { name: "plan_mode_complete", args: { plan: "# Finished plan" } }) });
    await child.arrived;
    await worker.waitFor((output) => output.workActivity?.parent === "idle" && output.workActivity.subagents === 1);
    expect(worker.view?.messages.flatMap((message) => message.blocks).find((block) => block.type === "tool-result" && block.toolName === "plan_mode_complete")?.text).toContain("sub-agents");
    expect(worker.outputs.some((output) => output.type === "plan_state" && output.phase === "ready")).toBe(false);
    child.release();
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
  });

  it("defers goal verification until background results have reached the parent", async () => {
    const { provider, worker } = await start("subagents-background-goal", { subagents: config() });
    const child = provider.holdChildren(1);
    worker.send({ id: crypto.randomUUID(), type: "goal_control", action: "set", objective: backgroundPrompt("goal", { agent: "scout", task: "child-ls: goal-helper" }), runId: "background-goal", startedAt: Date.now() });
    await child.arrived;
    await worker.waitFor((output) => output.workActivity?.parent === "idle" && output.workActivity.subagents === 1);
    expect(provider.requests.filter((entry) => entry.text.startsWith("<goal>"))).toHaveLength(0);
    child.release();
    await worker.waitFor((output) => output.type === "goal_state" && output.goal?.phase === "complete");
    const verdicts = provider.requests.filter((entry) => entry.text.startsWith("<goal>"));
    expect(verdicts).toHaveLength(1);
    expect(JSON.stringify(verdicts[0].body.messages)).toContain("goal-helper");
    expect(JSON.stringify(verdicts[0].body.messages)).toContain("Job ");
  });

  it("captures queued children's settings and keeps removed credentials redacted until cleanup", async () => {
    const second = await startMockProvider();
    cleanup.push(second.close);
    const providerConfig = { provider: { id: "second-connection", name: "Second connection", kind: "custom", baseUrl: second.baseUrl, api: "openai-completions", models: [{ id: "small-model", name: "Small model", contextWindow: 8_192, maxTokens: 256, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null } }] }, apiKey: "second-secret" };
    const agents = [{ ...scout, model: { providerId: "second-connection", modelId: "small-model", thinkingLevel: "off" } }];
    const { provider, worker } = await start("subagents-background-settings", { subagents: config({ maxConcurrency: 1, agents, providers: [providerConfig] }) });
    const child = second.holdRequest((entry) => entry.text.startsWith("child-ls:"));
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "background-settings", message: backgroundPrompt("settings", { tasks: [0, 1].map((index) => ({ agent: "scout", task: `child-ls: leak second-secret ${index}` })) }) });
    await child.arrived;
    await worker.waitFor((output) => output.workActivity?.parent === "idle" && output.workActivity.subagents === 2);
    const change = await request(worker, { type: "set_subagents", subagents: config({ maxConcurrency: 1 }) });
    expect(change.success).toBe(true);
    child.release();
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    expect(childRequests(provider, "SCOUT-PROMPT-MARKER")).toHaveLength(0);
    expect(second.requests).toHaveLength(4);
    expect(second.requests.every((entry) => entry.authorization === "Bearer second-secret" && entry.body.model === "small-model")).toBe(true);
    expect(subagentResult(worker)?.details.results.every((child) => child.status === "done" && !child.output?.includes("second-secret"))).toBe(true);
    worker.send({ id: crypto.randomUUID(), type: "watch_subagent", target: { toolCallId: "call-background-settings-0", index: 1 } });
    const transcript = await worker.waitFor((output) => output.type === "subagent_stream" && output.toolCallId === "call-background-settings-0" && output.reset === true);
    expect(JSON.stringify(transcript.upserts)).not.toContain("second-secret");
  });

  it("runs a new user message alongside background children, then cancels everything from a parent-idle Stop", async () => {
    const { provider, workspace, worker } = await start("subagents-background-stop", { subagents: config({ maxConcurrency: 1 }) });
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "background-stop", message: backgroundPrompt("stop", { tasks: [0, 1, 2].map((index) => ({ agent: "worker", task: `child-wait: background-${index}` })) }) });
    await provider.waitForSlowRequest();
    await worker.waitFor((output) => output.type === "run_state" && output.workActivity?.parent === "idle" && output.workActivity.subagents === 3);
    const queued = await request(worker, { type: "queue_message", message: 'mcp: write {"path":"new-user.txt","content":"handled immediately"}' });
    expect(queued.success).toBe(true);
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.role === "user" && message.blocks.some((block) => block.text?.includes("new-user.txt"))) === true);
    await worker.waitFor((output) => output.type === "run_state" && output.workActivity?.parent === "idle" && output.workActivity.subagents === 3 && worker.outputs.indexOf(output) > worker.outputs.indexOf(queued));
    expect(await readFile(join(workspace, "new-user.txt"), "utf8")).toBe("handled immediately");
    worker.send({ id: crypto.randomUUID(), type: "abort" });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    await worker.waitFor((output) => emitted(output) && subagentResult(worker)?.details.results.every((child) => child.status === "aborted") === true);
    expect(worker.outputs.some((output) => output.type === "run_finished" && output.runId === "background-stop" && output.outcome === "stopped")).toBe(true);
    expect(provider.requests.filter((entry) => entry.text.startsWith("child-wait:"))).toHaveLength(1);
    expect(backgroundResultText(provider)).toHaveLength(0);
    expect(worker.outputs.filter((output) => output.type === "worker_error")).toEqual([]);
  });

  it("disabling sub-agents cancels a blocked wait without queueing behind it or resuming later", async () => {
    const { provider, worker } = await start("subagents-background-disable", { subagents: config({ maxConcurrency: 1 }) });
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "background-disable", message: backgroundPrompt("disable", { tasks: [0, 1].map((index) => ({ agent: "worker", task: `child-wait: disable-${index}` })) }, { name: "subagent_job", args: { action: "wait" } }) });
    await provider.waitForSlowRequest();
    await worker.waitFor((output) => output.workActivity?.parent === "waiting");
    expect((await request(worker, { type: "set_subagents", subagents: null })).success).toBe(true);
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    expect(subagentResult(worker)?.details.results.every((child) => child.status === "aborted")).toBe(true);
    expect(provider.requests.filter((entry) => entry.text.startsWith("child-wait:"))).toHaveLength(1);
    expect(await readFile(worker.view!.sessionFile!, "utf8")).not.toContain('"customType":"wackcode-subagent-results"');
    expect(worker.view?.activeTools).not.toContain("subagent");
    expect(worker.view?.activeTools).not.toContain("subagent_job");
  });

  it("cold history interrupts live jobs, and a restarted worker never relaunches them", async () => {
    const { provider, worker, workspace } = await start("subagents-background-restart", { subagents: config() });
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "restart-child", message: backgroundPrompt("restart", { agent: "scout", task: "child-wait: restart" }) });
    await provider.waitForSlowRequest();
    await worker.waitFor((output) => output.workActivity?.parent === "idle" && output.workActivity.subagents === 1);
    const cold = await readSavedSession({ sessionFile: worker.view!.sessionFile, taskId: "restart", mode: "build", thinkingLevel: "off" });
    const card = cold.messages.flatMap((message) => message.blocks).find((block) => block.type === "tool-result" && block.toolName === "subagent");
    expect((card?.details as Details).results[0].status).toBe("interrupted");
    const path = worker.view!.sessionFile;
    await worker.shutdown();
    const reopened = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "background-reopened", path, undefined, undefined, undefined, undefined, { subagents: config() });
    cleanup.push(() => reopened.worker.shutdown());
    expect(provider.requests.filter((entry) => entry.text === "child-wait: restart")).toHaveLength(1);
    expect(reopened.ready.snapshot?.workActivity).toEqual({ parent: "idle", subagents: 0, pendingResults: 0 });
    const restored = reopened.ready.snapshot?.messages.flatMap((message) => message.blocks).find((block) => block.type === "tool-result" && block.toolName === "subagent");
    expect((restored?.details as Details).results[0].status).toBe("interrupted");
  });

  const accessCases = (["build", "plan", "ultraplan"] as TaskMode[]).flatMap((mode) =>
    [false, true].flatMap((unrestrictedPlanning) => [false, true].map((unrestrictedSubagents) => ({
      mode, unrestrictedPlanning, unrestrictedSubagents,
    }))));

  it.each(accessCases)("enforces child access in $mode (plan=$unrestrictedPlanning, children=$unrestrictedSubagents)", async ({ mode, unrestrictedPlanning, unrestrictedSubagents }) => {
    const agents = [
      { ...scout, builtin: true },
      { ...scout, name: "reviewer", builtin: true },
      { ...scout, name: "custom", builtin: false },
      editor,
      { ...editor, name: "custom-editor", builtin: false },
    ];
    const policy = { unrestrictedPlanning, unrestrictedSubagents };
    const { provider, workspace, worker, ready } = await start(`access-${mode}-${unrestrictedPlanning}-${unrestrictedSubagents}`, {
      subagents: config({ agents }), executionPolicy: policy,
    }, mode);
    expect(ready.snapshot?.executionPolicy).toEqual(policy);
    for (const role of agents) {
      const runId = `access-${role.name}`;
      const file = `access-${role.name}.txt`;
      worker.send({ id: crypto.randomUUID(), type: "prompt", runId, message: `subagent access: ${JSON.stringify({ agent: role.name, task: `child-write: access-${role.name}` })}` });
      await worker.waitFor((o) => o.type === "run_state" && o.state === "idle" && o.runId === runId);
      const ceiling = mode !== "build" && !unrestrictedPlanning;
      const canWrite = !ceiling && (!role.readOnly || unrestrictedSubagents);
      if (canWrite) expect(await readFile(join(workspace, file), "utf8")).toBe("from a sub-agent\n");
      else await expect(readFile(join(workspace, file), "utf8")).rejects.toThrow();
      const block = worker.view?.messages.flatMap((m) => m.blocks).filter((b) => b.toolName === "subagent" && b.type === "tool-result").at(-1);
      const details = block?.details as Details | undefined;
      if (ceiling && !role.readOnly) expect(block?.text).toContain("Plan mode only runs read-only sub-agents");
      else expect(details?.results[0].readOnly).toBe(role.readOnly && !canWrite);
    }
    expect(provider.requests.length).toBeGreaterThan(0);
  });

  it("allows unrestricted child shell commands, while retaining global tool switches", async () => {
    const { workspace, worker } = await start("subagents-unrestricted-shell", {
      subagents: config(), executionPolicy: { unrestrictedPlanning: true, unrestrictedSubagents: true },
    }, "plan");
    await writeFile(join(workspace, "keep.txt"), "fixture");
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "shell", message: "subagent guard" });
    await worker.waitFor((o) => o.type === "run_state" && o.runId === "shell" && o.state === "idle");
    await expect(readFile(join(workspace, "keep.txt"), "utf8")).rejects.toThrow();
    worker.send({ id: crypto.randomUUID(), type: "set_tools", disabledTools: ["write", "bash"] });
    await worker.waitFor((o) => o.type === "snapshot" && o.snapshot?.activeTools?.includes("write") === false);
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "disabled", message: `subagent access: ${JSON.stringify({ agent: "scout", task: "child-write: disabled" })}` });
    await worker.waitFor((o) => o.type === "run_state" && o.runId === "disabled" && o.state === "idle");
    await expect(readFile(join(workspace, "disabled.txt"), "utf8")).rejects.toThrow();
  });

  it.each([false, true])("queues child override changes until the active turn finishes (initial=%s)", async (initial) => {
    const active = { unrestrictedPlanning: false, unrestrictedSubagents: initial };
    const next = { unrestrictedPlanning: false, unrestrictedSubagents: !initial };
    const { provider, worker, workspace } = await start("subagents-queued-access", { subagents: config(), executionPolicy: active });
    const barrier = provider.holdChildren(1);
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "active", message: 'subagent access: {"agent":"scout","task":"child-write: active"}' });
    await barrier.arrived;
    const mark = worker.outputs.length;
    worker.send({ id: crypto.randomUUID(), type: "set_execution_policy", executionPolicy: next });
    expect(worker.view?.executionPolicy).toEqual(active);
    barrier.release();
    await worker.waitFor((o) => worker.outputs.indexOf(o) >= mark && o.type === "snapshot" && o.snapshot?.executionPolicy?.unrestrictedSubagents === !initial);
    expect(subagentResult(worker)?.details.results[0].readOnly).toBe(!initial);
    if (initial) expect(await readFile(join(workspace, "active.txt"), "utf8")).toContain("from a sub-agent");
    else await expect(readFile(join(workspace, "active.txt"), "utf8")).rejects.toThrow();
    expect(worker.child.exitCode).toBeNull();
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "next", message: 'subagent access: {"agent":"scout","task":"child-write: next"}' });
    await worker.waitFor((o) => o.type === "run_state" && o.runId === "next" && o.state === "idle");
    const block = worker.view?.messages.flatMap((m) => m.blocks).filter((b) => b.toolName === "subagent" && b.type === "tool-result").at(-1);
    expect((block?.details as Details).results[0].readOnly).toBe(initial);
    if (!initial) expect(await readFile(join(workspace, "next.txt"), "utf8")).toContain("from a sub-agent");
    else await expect(readFile(join(workspace, "next.txt"), "utf8")).rejects.toThrow();
  });

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

  it("adds the shell companion only for children whose role and global switches allow bash", async () => {
    const { provider, worker, ready } = await start("subagents-shell-companion", {
      subagents: config({ agents: [{ ...scout, tools: ["read", "ls"] }] }),
    });
    const run = async (runId: string) => {
      const before = provider.requests.length;
      worker.send({ id: crypto.randomUUID(), type: "prompt", runId, message: "subagent single" });
      await worker.waitFor((output) => output.type === "run_state" && output.runId === runId && output.state === "idle");
      return provider.requests.slice(before).filter((request) => request.text.startsWith("child-"))[0];
    };
    expect(offered(await run("without-bash"))).toEqual(["ls", "read"]);
    const rosterId = crypto.randomUUID();
    worker.send({ id: rosterId, type: "set_subagents", subagents: config() });
    await worker.waitFor((output) => output.type === "response" && output.id === rosterId && output.success === true);
    const roleTools = (ready.snapshot?.tools ?? []).filter((tool) => tool.available && scout.tools.includes(tool.name)).map((tool) => tool.name);
    expect(offered(await run("with-bash"))).toEqual([...roleTools, "bash_job"].sort());
    const toolsId = crypto.randomUUID();
    worker.send({ id: toolsId, type: "set_tools", disabledTools: ["bash"] });
    await worker.waitFor((output) => output.type === "response" && output.id === toolsId && output.success === true);
    expect(offered(await run("disabled-bash"))).toEqual(roleTools.filter((name) => name !== "bash").sort());
    // The parent companion is always on; that must not smuggle it into a bash-less child.
    expect(worker.view?.activeTools).toContain("bash_job");
  });

  it.each(["build", "plan"] as const)("lets a read-only child check in and finish a named test in %s with private job ownership", async (mode) => {
    const reviewer = { ...scout, name: "reviewer", prompt: "REVIEWER-CHECK-IN-MARKER. Run relevant tests." };
    const { provider, workspace, worker } = await start(`subagents-shell-${mode}`, { subagents: config({ agents: [reviewer] }) }, mode);
    const fixture = join(workspace, "test fixture");
    await mkdir(fixture);
    await writeFile(join(fixture, "package.json"), JSON.stringify({
      private: true, scripts: { "test:unit": "node --test --test-reporter=tap fixture.test.mjs" },
    }));
    await writeFile(join(fixture, "fixture.test.mjs"), `
      import test from 'node:test';
      import { appendFileSync, existsSync } from 'node:fs';
      test('read-only-shell-check-in-evidence', async () => {
        appendFileSync('executions.txt', 'once\\n');
        while (!existsSync('finish')) await new Promise((resolve) => setTimeout(resolve, 10));
      });
    `);
    const command = "pnpm --dir 'test fixture' run test:unit";
    const task = shellJobPrompt({ command, yieldTimeout: 0.03, actions: [{ action: "status" }, { action: "wait", waitSeconds: 0.1 }] }, true);
    const yielded = provider.holdRequest((request) => request.text === task && toolResultContents(request.body).length === 1);
    cleanup.push(async () => { yielded.release(); worker.send({ id: crypto.randomUUID(), type: "abort" }); });
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "child-check-in", message: `subagent access: ${JSON.stringify({ agent: "reviewer", task })}` });
    await yielded.arrived;
    const firstResult = toolResultContents(provider.requests.at(-1)!.body)[0];
    const jobId = RUNNING_SHELL_JOB.exec(firstResult)?.[1];
    expect(jobId).toBeTruthy();
    expect(offered(provider.requests.at(-1)!)).toContain("bash_job");
    expect(offered(provider.requests.at(-1)!)).not.toContain("write");
    await writeFile(join(fixture, "finish"), "go");
    yielded.release();
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "child-check-in" && output.state === "idle", 5_000);
    const requests = provider.requests.filter((request) => request.text === task);
    const final = toolResultContents(requests.at(-1)!.body).at(-1)!;
    expect(final).toContain(`Job ${jobId} completed`);
    expect(final).toContain("read-only-shell-check-in-evidence");
    expect(final).toContain("# fail 0");
    expect(final).not.toContain("This sub-agent is read-only");
    const calls = toolCallsOf(requests.at(-1)!.body);
    expect(calls.filter((call) => call.name === "bash")).toHaveLength(1);
    expect(calls.filter((call) => call.name === "bash_job").every((call) => call.args.jobId === jobId)).toBe(true);
    expect(await readFile(join(fixture, "executions.txt"), "utf8")).toBe("once\n");
    expect(subagentResult(worker)?.details.results[0]).toMatchObject({ status: "done", readOnly: true });

    // An id visible in the provider transcript is still NOT owned by the parent session.
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "foreign-job", message: `mcp: bash_job ${JSON.stringify({ jobId, action: "status" })}` });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "foreign-job" && output.state === "idle");
    expect(toolResultContents(provider.requests.at(-1)!.body)[0]).toContain("no longer available in this session");

    const blockedTask = shellJobPrompt({ command: "touch blocked-child.txt", actions: [{ action: "wait", waitSeconds: 0 }] }, true);
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "guard", message: `subagent access: ${JSON.stringify({ agent: "reviewer", task: blockedTask })}` });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "guard" && output.state === "idle");
    const blocked = provider.requests.filter((request) => request.text === blockedTask).at(-1)!;
    expect(toolResultContents(blocked.body)[0]).toContain("This sub-agent is read-only");
    expect(toolCallsOf(blocked.body).map((call) => call.name)).toEqual(["bash"]);
    await expect(readFile(join(workspace, "blocked-child.txt"))).rejects.toMatchObject({ code: "ENOENT" });
    if (mode === "plan") {
      worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "parent-guard", message: shellJobPrompt({ command: "touch blocked-parent.txt", actions: [] }) });
      await worker.waitFor((output) => output.type === "run_state" && output.runId === "parent-guard" && output.state === "idle");
      expect(toolResultContents(provider.requests.at(-1)!.body)[0]).toContain("Plan mode only allows read-only shell commands");
      await expect(readFile(join(workspace, "blocked-parent.txt"))).rejects.toMatchObject({ code: "ENOENT" });
    }
    expect(worker.outputs.filter((output) => output.type === "worker_error")).toEqual([]);
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
      const roleTools = (ready.snapshot?.tools ?? []).filter((tool) => tool.available && scout.tools.includes(tool.name)).map((tool) => tool.name);
      expect(tools).toEqual([...roleTools, "bash_job"].sort());
      for (const hidden of ["edit", "write", "subagent", "todo", "ask_user_question", "plan_mode_complete"]) {
        expect(tools).not.toContain(hidden);
      }
    }
    expect(provider.requests.filter((request) => request.text === "subagent single")).toHaveLength(2);

    // The children's usage is part of the chat's totals: two parent requests and two child ones.
    expect(worker.view?.stats?.tokens.input).toBeGreaterThanOrEqual(96);
    const records = worker.outputs.filter((o) => o.type === "usage_record").map((o) => o.record!);
    expect(records.filter((r) => r.purpose === "subagent")).toHaveLength(2);
    expect(records.filter((r) => r.purpose === "chat")).toHaveLength(2);
    expect(records.filter((r) => r.subagent_id)).toHaveLength(2);
    expect(records).toHaveLength(provider.requests.length);
  });

  it("runs readers and editors concurrently and streams each child independently", async () => {
    const { provider, workspace, worker } = await start("subagents-parallel", { subagents: config() });
    const barrier = provider.holdChildren(4, true);
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent parallel" });
    await barrier.arrived;
    await worker.waitFor((output) => {
      const details = output.detail?.details as Details | undefined;
      return details?.results?.filter((child) => child.status === "running").length === 4;
    });
    // Both editors have written their own file and are still awaiting their final answer.
    for (const index of [1, 3]) {
      worker.send({ id: crypto.randomUUID(), type: "watch_subagent", target: { toolCallId: "call-alpha", index } });
      const frame = await worker.waitFor((output) => output.type === "subagent_stream" && output.index === index && output.live === true);
      expect(JSON.stringify(frame.upserts)).toContain(index === 1 ? "one.txt" : "two.txt");
      expect(JSON.stringify(frame.upserts)).not.toContain(index === 1 ? "two.txt" : "one.txt");
    }
    barrier.release();
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
    // The two editors overlapped too.
    const one = window("child-write: one");
    const two = window("child-write: two");
    expect(one.first).toBeLessThan(two.last);
    expect(two.first).toBeLessThan(one.last);
  });

  it("keeps successful siblings when an editing child is unavailable", async () => {
    const { worker } = await start("subagents-partial", {
      subagents: config({ agents: [scout, { ...editor, unavailable: "Test connection unavailable." }] })
    });
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent parallel" });
    await worker.waitFor((output) => emitted(output) && subagentResult(worker) !== undefined);
    expect(subagentResult(worker)?.details.results.map((child) => child.status)).toEqual(["done", "failed", "done", "failed"]);
    expect(subagentResult(worker)?.details.results[1].error).toBe("Test connection unavailable.");
    expect(subagentResult(worker)?.isError).not.toBe(true);
  });

  it("cancels active editors and never sends queued children to the provider", async () => {
    const { provider, worker } = await start("subagents-batch-abort", { subagents: config({ maxConcurrency: 2 }) });
    const barrier = provider.holdChildren(2);
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent batch wait" });
    await barrier.arrived;
    await worker.waitFor((output) => {
      const details = output.detail?.details as Details | undefined;
      return details?.results?.filter((child) => child.status === "running").length === 2;
    });
    expect(provider.requests.filter((request) => request.text.startsWith("child-wait"))).toHaveLength(2);
    barrier.release();
    await provider.waitForSlowRequest();
    worker.send({ id: crypto.randomUUID(), type: "abort" });
    await worker.waitFor((output) => emitted(output) && subagentResult(worker) !== undefined);
    expect(subagentResult(worker)?.details.results.map((child) => child.status)).toEqual(["aborted", "aborted", "aborted", "aborted"]);
    expect(provider.requests.filter((request) => request.text.startsWith("child-wait")).map((request) => request.text).sort()).toEqual(["child-wait: 0", "child-wait: 1"]);
    expect(worker.outputs.filter((output) => output.type === "worker_error")).toEqual([]);
  });

  it.each([
    { mode: "build" as TaskMode, passes: true },
    { mode: "build" as TaskMode, passes: false },
    { mode: "plan" as TaskMode, passes: true },
  ])("lets the reviewer run a named test script in $mode and receive its result (passes=$passes)", async ({ mode, passes }) => {
    const reviewer = { ...scout, name: "reviewer", prompt: "REVIEWER-PROMPT-MARKER. Run relevant tests." };
    const { provider, workspace, worker } = await start(`subagents-test-${mode}-${passes}`, {
      subagents: config({ agents: [reviewer] }),
    }, mode);
    const fixture = join(workspace, "test fixture");
    await mkdir(fixture);
    await writeFile(join(fixture, "package.json"), JSON.stringify({
      private: true, scripts: { "test:unit": "node --test --test-reporter=tap fixture.test.mjs" },
    }));
    await writeFile(join(fixture, "fixture.test.mjs"), `
      import test from 'node:test';
      import assert from 'node:assert/strict';
      test('reviewer-test-evidence', () => assert.equal(${passes}, true));
    `);
    const command = "pnpm --dir 'test fixture' run test:unit";
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: `subagent test: ${command}` });
    await worker.waitFor((output) => emitted(output) && subagentResult(worker) !== undefined);

    const requests = childRequests(provider, "REVIEWER-PROMPT-MARKER");
    expect(requests).toHaveLength(2);
    expect(offered(requests[0])).toContain("bash");
    expect(offered(requests[0])).toContain("bash_job");
    expect(offered(requests[0])).not.toContain("write");
    expect(offered(requests[0])).not.toContain("edit");
    const messages = requests[1].body.messages as Array<{ role: string; content?: string }>;
    const result = messages.find((message) => message.role === "tool")?.content ?? "";
    expect(result).toContain("reviewer-test-evidence");
    expect(result).not.toContain("This sub-agent is read-only");
    if (passes) expect(result).toContain("# fail 0");
    else expect(result).toContain("Command exited with code 1");
    expect(subagentResult(worker)?.details.results[0].activity).toEqual([{ tool: "bash", subject: command }]);
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

  const target = { toolCallId: "call-alpha", index: 0 };
  const streamFrames = (worker: WorkerHarness) => worker.outputs.filter((output) => output.type === "subagent_stream");

  /** The watched child's transcript as the renderer rebuilds it: resets replace, the rest chain by rev. */
  function replay(frames: Output[]): SnapshotView["messages"] {
    let messages: SnapshotView["messages"] = [];
    let rev = -1;
    for (const frame of frames) {
      if (frame.reset) {
        messages = [...(frame.upserts ?? [])];
      } else {
        expect(frame.rev).toBe(rev + 1);
        const removed = new Set(frame.removed);
        messages = messages.filter((message) => !removed.has(message.id ?? ""));
        for (const upsert of frame.upserts ?? []) {
          const at = messages.findIndex((message) => message.id === upsert.id);
          if (at >= 0) messages[at] = upsert;
          else messages.push(upsert);
        }
      }
      rev = frame.rev ?? rev;
    }
    return messages;
  }

  it("streams a watched sub-agent from before it starts until it stops", async () => {
    const { provider, worker } = await start("subagents-stream", { subagents: config() });
    // Watching is sticky: the child doesn't exist yet, so the first frame says so.
    const watchId = crypto.randomUUID();
    worker.send({ id: watchId, type: "watch_subagent", target });
    const first = await worker.waitFor((output) => output.type === "subagent_stream");
    expect(first).toMatchObject({ taskId: "subagents-stream", ...target, rev: 0, reset: true, upserts: [], removed: [], partial: null, live: false, missing: true });
    // The reset frame goes out before the command's answer.
    const answered = await worker.waitFor((output) => output.type === "response" && output.id === watchId);
    expect(answered.success).toBe(true);
    expect(worker.outputs.indexOf(first)).toBeLessThan(worker.outputs.indexOf(answered));

    // The frames follow the child once it begins, while its call still holds the command queue.
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent wait" });
    await provider.waitForSlowRequest();
    const live = await worker.waitFor((output) => output.type === "subagent_stream" && output.live === true);
    expect(live.missing).toBeUndefined();

    worker.send({ id: crypto.randomUUID(), type: "abort" });
    await worker.waitFor((output) => output.type === "subagent_stream" && output.rev !== undefined && output.rev > (live.rev ?? 0) && output.live === false);
    replay(streamFrames(worker));
    expect(worker.outputs.filter((output) => output.type === "worker_error")).toEqual([]);

    // Stopping the watch stops the frames.
    worker.send({ id: crypto.randomUUID(), type: "watch_subagent", target: null });
    const count = streamFrames(worker).length;
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-2", message: "subagent single" });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "run-2" && output.state === "idle");
    expect(streamFrames(worker)).toHaveLength(count);
  });

  it("saves a sub-agent's transcript on its result, out of snapshots, and serves it after a restart", async () => {
    const { workspace, worker, provider } = await start("subagents-saved", { subagents: config() });
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent single" });
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);

    // Snapshots and live card updates never carry transcripts.
    expect(subagentResult(worker)?.details.results[0]).not.toHaveProperty("transcript");
    const updates = worker.outputs.filter((output) => output.type === "activity" && output.event === "tool_execution_update");
    expect(updates.length).toBeGreaterThan(0);
    expect(JSON.stringify(updates)).not.toContain("transcript");

    worker.send({ id: crypto.randomUUID(), type: "watch_subagent", target });
    const saved = await worker.waitFor((output) => output.type === "subagent_stream");
    expect(saved).toMatchObject({ rev: 0, reset: true, live: false, partial: null });
    expect(saved.missing).toBeUndefined();
    // What the child ran and answered, never its task (that is on the result already).
    const transcript = saved.upserts ?? [];
    expect(transcript.map((message) => message.role)).toEqual(["assistant", "tool", "assistant"]);
    expect(transcript[0].blocks).toEqual([expect.objectContaining({ type: "tool-call", toolName: "ls" })]);
    expect(transcript[1].blocks[0]).toMatchObject({ type: "tool-result", toolName: "ls" });
    expect(transcript[2].blocks).toEqual([expect.objectContaining({ type: "text", text: "Child done: child-ls: look around" })]);

    const sessionFile = worker.view?.sessionFile;
    expect(sessionFile).toBeTruthy();
    expect(await readFile(sessionFile as string, "utf8")).toContain("\"transcript\"");
    await worker.shutdown();

    // A fresh worker reads it back from the session.
    const restored = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "subagents-saved", sessionFile, undefined, undefined, undefined, undefined, { subagents: config() });
    cleanup.push(() => restored.worker.shutdown());
    expect(restored.ready.snapshot?.messages.flatMap((message) => message.blocks)
      .find((block) => block.toolName === "subagent" && block.type === "tool-result")?.details).not.toHaveProperty("results.0.transcript");
    restored.worker.send({ id: crypto.randomUUID(), type: "watch_subagent", target });
    const reread = await restored.worker.waitFor((output) => output.type === "subagent_stream");
    expect(reread).toMatchObject({ rev: 0, reset: true, live: false });
    expect(reread.upserts).toEqual(transcript);

    // A child this chat never ran.
    restored.worker.send({ id: crypto.randomUUID(), type: "watch_subagent", target: { toolCallId: "call-elsewhere", index: 0 } });
    expect(await restored.worker.waitFor((output) => output.type === "subagent_stream" && output.toolCallId === "call-elsewhere"))
      .toMatchObject({ reset: true, upserts: [], missing: true });
  });

  it("redacts a sub-agent's transcript like its answer", async () => {
    const { worker } = await start("subagents-leak", { subagents: config() });
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "run-1", message: "subagent leak" });
    await worker.waitFor((output) => emitted(output) && output.view?.messages.some((message) => message.blocks.some((block) => block.text === "Finished alpha.")) === true);
    expect(subagentResult(worker)?.details.results[0].output).toBe("Child done: child-ls: leak [credential redacted]");

    worker.send({ id: crypto.randomUUID(), type: "watch_subagent", target });
    const frame = await worker.waitFor((output) => output.type === "subagent_stream");
    expect(frame.upserts?.at(-1)?.blocks).toEqual([expect.objectContaining({ text: "Child done: child-ls: leak [credential redacted]" })]);
    // `view` is the harness's own copy of the chat, not part of the frame.
    const { view: _view, ...sent } = frame;
    expect(JSON.stringify(sent)).not.toContain("alpha-secret");
  });

  it("answers a watch sent while the worker is still starting, once it has started", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-subagents-early-watch-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const worker = new WorkerHarness(workspace);
    cleanup.push(() => worker.shutdown());
    worker.send({
      id: crypto.randomUUID(), type: "init", taskId: "subagents-early", cwd: workspace,
      agentDir: join(workspace, ".agent"), sessionDir: join(workspace, ".sessions"),
      provider: { id: "p", name: "P", kind: "custom", baseUrl: provider.baseUrl, api: "openai-completions", models: [{ id: "shared-model", name: "Shared model", contextWindow: 16_384, maxTokens: 321, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null } }] },
      modelId: "shared-model", apiKey: "alpha-secret", thinkingLevel: "off", subagents: config()
    });
    // Sent right behind init, like a chip clicked while the chat's worker is still spawning.
    const watchId = crypto.randomUUID();
    worker.send({ id: watchId, type: "watch_subagent", target });
    expect(await worker.waitFor((output) => output.type === "response" && output.id === watchId)).toMatchObject({ success: true });
    const ready = worker.outputs.findIndex((output) => output.type === "ready");
    const frame = worker.outputs.findIndex((output) => output.type === "subagent_stream");
    expect(ready).toBeGreaterThanOrEqual(0);
    expect(frame).toBeGreaterThan(ready);
    expect(worker.outputs[frame]).toMatchObject({ taskId: "subagents-early", reset: true, missing: true });
  });

  it("refuses to watch something that isn't a sub-agent", async () => {
    const { worker } = await start("subagents-watch-invalid", { subagents: config() });
    const id = crypto.randomUUID();
    worker.send({ id, type: "watch_subagent", target: { toolCallId: "", index: -1 } });
    expect(await worker.waitFor((output) => output.type === "response" && output.id === id)).toMatchObject({ success: false, error: "That sub-agent is not in this chat." });
    // Answered to the caller alone: no banner in the chat.
    expect(worker.outputs.filter((output) => output.type === "worker_error")).toEqual([]);
  });
});

describe("MCP servers", () => {
  const MOCK_MCP_SERVER = resolve("../scripts/mock-mcp-server.mjs");
  const ENV_SECRET = "mcp-env-secret-1234";

  function stdioServer(overrides: Record<string, unknown> = {}) {
    return {
      id: "mcp-mock", name: "Mock", slug: "mock", transport: "stdio", timeoutMs: 10_000,
      command: process.execPath, args: [MOCK_MCP_SERVER], env: { MOCK_MCP_SECRET: ENV_SECRET }, disabledTools: [],
      ...overrides
    };
  }

  async function startHttpServer(token: string): Promise<string> {
    const child = spawn(process.execPath, [MOCK_MCP_SERVER, "--http", "0", "--require-auth", token], { stdio: ["ignore", "pipe", "pipe"] });
    cleanup.push(async () => { child.kill(); });
    const line = await new Promise<string>((resolveLine) => createInterface({ input: child.stdout }).once("line", resolveLine));
    return /(http:\/\/\S+\/mcp)/.exec(line)![1];
  }

  function offeredTools(request: { body: Record<string, unknown> }): string[] {
    return ((request.body.tools ?? []) as Array<{ function: { name: string } }>).map((tool) => tool.function.name);
  }

  function toolResult(request: { body: Record<string, unknown> }): string {
    const message = (request.body.messages as Array<{ role?: string; content?: unknown }>).findLast((entry) => entry.role === "tool");
    return typeof message?.content === "string" ? message.content : JSON.stringify(message?.content);
  }

  /** Send a prompt and wait for its run to end; returns the index of its first provider request. */
  async function runPrompt(worker: WorkerHarness, provider: MockProvider, message: string, requests = 2): Promise<number> {
    const before = provider.requests.length;
    const runId = crypto.randomUUID();
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId, message });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === runId && output.state === "idle" && provider.requests.length >= before + requests);
    return before;
  }

  async function start(taskId: string, servers: unknown[], mode?: TaskMode) {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), `wackcode-${taskId}-`));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker, ready } = await initializeWorker(
      provider.baseUrl, "alpha-secret", workspace, taskId, undefined, undefined, undefined, mode, undefined, { mcp: servers }
    );
    cleanup.push(() => worker.shutdown());
    return { provider, workspace, worker, ready };
  }

  it("starts a stdio server at the first run, offers its tools, and gives it its environment in the workspace", async () => {
    const { provider, workspace, worker, ready } = await start("mcp-stdio", [stdioServer()]);
    // Opening a chat starts nothing.
    expect(ready.snapshot?.tools?.some((tool) => tool.name.startsWith("mcp__"))).toBe(false);

    const first = await runPrompt(worker, provider, "mcp: mcp__mock__whoami {}");
    expect(offeredTools(provider.requests[first])).toEqual(expect.arrayContaining(["mcp__mock__echo", "mcp__mock__peek", "mcp__mock__whoami"]));
    expect(JSON.parse(toolResult(provider.requests[first + 1]))).toEqual({ authorization: null, secret: ENV_SECRET, cwd: await realpath(workspace) });
    expect(worker.outputs.some((output) => output.type === "activity" && output.event === "mcp_connect_start")).toBe(true);
    expect(worker.outputs.some((output) => output.type === "activity" && output.event === "mcp_connect_end")).toBe(true);
    const tool = worker.view?.tools?.find((entry) => entry.name === "mcp__mock__echo");
    expect(tool?.source).toMatchObject({ kind: "mcp", serverId: "mcp-mock" });
    expect(tool?.description).toContain("(MCP server \"Mock\")");

    // An error result reaches the model as a failed tool call, and the next run reuses the connection.
    const second = await runPrompt(worker, provider, "mcp: mcp__mock__fail {}");
    expect(toolResult(provider.requests[second + 1])).toContain("The mock tool failed on purpose.");
    expect(worker.outputs.filter((output) => output.type === "activity" && output.event === "mcp_connect_start")).toHaveLength(1);
    expect(worker.outputs.some((output) => output.type === "worker_error")).toBe(false);
  });

  it("switches a single tool, then the whole server, off live", async () => {
    const { provider, worker } = await start("mcp-live", [stdioServer()]);
    await runPrompt(worker, provider, "mcp: mcp__mock__peek {}");

    const after = (mark: number) => (output: Output) => worker.outputs.indexOf(output) >= mark && output.type === "snapshot";
    let mark = worker.outputs.length;
    worker.send({ id: crypto.randomUUID(), type: "set_mcp", servers: [stdioServer({ disabledTools: ["echo"] })] });
    const switched = await worker.waitFor(after(mark));
    expect(switched.view?.activeTools).toContain("mcp__mock__peek");
    expect(switched.view?.activeTools).not.toContain("mcp__mock__echo");
    const refused = await runPrompt(worker, provider, "mcp: mcp__mock__echo {\"text\":\"hi\"}");
    expect(offeredTools(provider.requests[refused])).not.toContain("mcp__mock__echo");

    mark = worker.outputs.length;
    worker.send({ id: crypto.randomUUID(), type: "set_mcp", servers: [] });
    const removed = await worker.waitFor(after(mark));
    expect(removed.view?.activeTools?.some((name) => name.startsWith("mcp__"))).toBe(false);
    const plain = await runPrompt(worker, provider, "Plain message.", 1);
    expect(offeredTools(provider.requests[plain]).some((name) => name.startsWith("mcp__"))).toBe(false);
  });

  it("sends an HTTP server its headers, and a Streamable HTTP and an SSE server both work", async () => {
    const url = await startHttpServer("http-token-5678");
    const headers = { Authorization: "Bearer http-token-5678" };
    const { provider, worker } = await start("mcp-http", [
      { id: "mcp-http", name: "Remote", slug: "remote", transport: "http", timeoutMs: 10_000, url, headers, disabledTools: [] },
      { id: "mcp-sse", name: "Legacy", slug: "legacy", transport: "sse", timeoutMs: 10_000, url: url.replace(/\/mcp$/, "/sse"), headers, disabledTools: [] }
    ]);
    const first = await runPrompt(worker, provider, "mcp: mcp__remote__whoami {}");
    expect(JSON.parse(toolResult(provider.requests[first + 1])).authorization).toBe("Bearer http-token-5678");
    const second = await runPrompt(worker, provider, "mcp: mcp__legacy__echo {\"text\":\"over sse\"}");
    expect(toolResult(provider.requests[second + 1])).toContain("over sse");
  });

  it.each(["plan", "ultraplan"] as const)("times out tools and enforces MCP access in %s with overrides and server switches", async (mode) => {
    const { provider, worker } = await start("mcp-plan", [stdioServer({ timeoutMs: 1_000 })]);
    const hung = await runPrompt(worker, provider, "mcp: mcp__mock__hang {}");
    expect(toolResult(provider.requests[hung + 1])).toContain("did not respond within 1000 ms");

    worker.send({ id: crypto.randomUUID(), type: "set_mode", mode });
    await worker.waitFor((output) => output.type === "plan_state" && output.mode === mode);
    const peek = await runPrompt(worker, provider, "mcp: mcp__mock__peek {}");
    expect(toolResult(provider.requests[peek + 1])).toContain("peeked");
    const echo = await runPrompt(worker, provider, "mcp: mcp__mock__echo {\"text\":\"hi\"}");
    expect(toolResult(provider.requests[echo + 1])).toContain("its MCP server doesn't mark it read-only");

    worker.send({ id: crypto.randomUUID(), type: "set_execution_policy", executionPolicy: { unrestrictedPlanning: true, unrestrictedSubagents: false } });
    await worker.waitFor((o) => o.type === "snapshot" && o.snapshot?.executionPolicy?.unrestrictedPlanning === true);
    const unlocked = await runPrompt(worker, provider, 'mcp: mcp__mock__echo {"text":"unrestricted"}');
    expect(toolResult(provider.requests[unlocked + 1])).toContain("unrestricted");
    const mark = worker.outputs.length;
    worker.send({ id: crypto.randomUUID(), type: "set_mcp", servers: [stdioServer({ disabledTools: ["echo"] })] });
    await worker.waitFor((o) => worker.outputs.indexOf(o) >= mark && o.type === "snapshot");
    const disabled = await runPrompt(worker, provider, 'mcp: mcp__mock__echo {"text":"disabled"}');
    expect(offeredTools(provider.requests[disabled])).not.toContain("mcp__mock__echo");
  });

  it("reports a server that can't start once, without holding up later runs or leaking its secrets", async () => {
    const { provider, worker } = await start("mcp-broken", [
      stdioServer({ command: "wackcode-no-such-command", env: { TOKEN: "broken-secret-9876" } }),
      { id: "mcp-refused", name: "Refused", slug: "refused", transport: "http", timeoutMs: 5_000, url: await startHttpServer("right-token"), headers: { Authorization: "Bearer wrong-token-4321" }, disabledTools: [] }
    ]);
    await runPrompt(worker, provider, "Hello.", 1);
    const notices = () => worker.outputs.filter((output) => output.type === "extension_notice" && output.level === "warning");
    await worker.waitFor(() => notices().length === 2);
    const text = notices().map((output) => output.message).join("\n");
    expect(text).toContain("MCP server \"Mock\" isn't available: Command not found: wackcode-no-such-command.");
    expect(text).toContain("MCP server \"Refused\" isn't available: The server refused the request (HTTP 401).");
    expect(text).not.toContain("wrong-token-4321");

    const started = Date.now();
    await runPrompt(worker, provider, "Hello again.", 1);
    expect(Date.now() - started).toBeLessThan(3_000);
    expect(notices()).toHaveLength(2);
  });

  it("stops the run cleanly while a server is still starting", async () => {
    const silent = stdioServer({ args: ["-e", "setInterval(() => {}, 1000)"], timeoutMs: 60_000 });
    const { provider, worker } = await start("mcp-stop", [silent]);
    const runId = crypto.randomUUID();
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId, message: "Hello." });
    await worker.waitFor((output) => output.type === "activity" && output.event === "mcp_connect_start");
    worker.send({ id: crypto.randomUUID(), type: "abort" });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === runId && output.state === "idle", 5_000);
    await worker.waitFor((output) => output.type === "activity" && output.event === "mcp_connect_end", 5_000);
    expect(provider.requests).toHaveLength(0);
    expect(worker.outputs.some((output) => output.type === "worker_error" || output.type === "extension_notice")).toBe(false);
  });
});

describe("message queueing", () => {
  /** Starts the run whose bash tool sleeps between two lines, leaving a deterministic
   *  mid-run window for queue commands, and waits until that window is open. */
  async function startSlowRun(secret: string, taskName: string, message = "stream tool") {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), `wackcode-${taskName}-`));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, secret, workspace, `${taskName}-task`);
    cleanup.push(() => worker.shutdown());
    const runId = crypto.randomUUID();
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId, message });
    await worker.waitFor((output) =>
      output.type === "activity" && output.event === "tool_execution_update" && output.detail?.text?.includes("first line") === true);
    return { provider, worker, runId, workspace };
  }

  async function startHeldRun(vision = false) {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-held-queue-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "held-task", undefined, undefined, undefined, undefined, vision);
    cleanup.push(() => worker.shutdown());
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "held-run", message: "wait until stopped" });
    await provider.waitForSlowRequest();
    return { provider, worker, workspace };
  }

  async function enqueue(worker: WorkerHarness, message: string, extra: Record<string, unknown> = {}) {
    const id = crypto.randomUUID();
    worker.send({ id, type: "queue_message", message, ...extra });
    const accepted = await worker.waitFor((output) => output.type === "response" && output.id === id);
    expect(accepted.success).toBe(true);
    return id;
  }

  it("queues by default and sends FIFO only after the active work completes", async () => {
    const { provider, worker, runId } = await startSlowRun("alpha-secret", "follow-up");
    const first = await enqueue(worker, "Follow-up: then summarize.");
    const second = await enqueue(worker, "Then run the tests.");
    await worker.waitFor((output) => output.type === "queue_state" && output.messages?.some((message) => message.id === second) === true);
    expect(worker.outputs.find((output) => output.type === "queue_state" && output.messages?.length === 2)?.messages).toEqual([
      { id: first, text: "Follow-up: then summarize." }, { id: second, text: "Then run the tests." }
    ]);
    const completed = await worker.waitFor((output) => output.type === "run_finished" && output.runId === runId);
    expect(completed.outcome).toBe("completed");
    await worker.waitFor(() => provider.requests.some((request) => request.text === "Then run the tests."));
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    const followUp = provider.requests.find((request) => request.text === "Follow-up: then summarize.");
    expect(JSON.stringify(followUp?.body)).toContain("second line");
    expect(JSON.stringify(followUp?.body)).toContain("Finished alpha.");
    const texts = worker.view?.messages.filter((message) => message.role === "user").map((message) => message.blocks[0].text);
    expect(texts).toEqual(["stream tool", "Follow-up: then summarize.", "Then run the tests."]);
    expect(worker.outputs.filter((output) => output.type === "run_state" && output.state === "running")).toHaveLength(3);
    expect(worker.outputs.filter((output) => output.type === "queue_state").at(-1)?.messages).toEqual([]);
  });

  it("interrupts a held LLM request and sends the selected message before other queued work", async () => {
    const { provider, worker } = await startHeldRun();
    await enqueue(worker, "First queued message.");
    const selected = await enqueue(worker, "Send this now.");
    await enqueue(worker, "Last queued message.");
    // Enqueueing never interrupts the held request or launches another model call.
    expect(provider.requests).toHaveLength(1);
    const id = crypto.randomUUID();
    worker.send({ id, type: "steer_message", messageId: selected, runId: "steered-run", checkpoint: { id: "steered-checkpoint" } });
    expect((await worker.waitFor((output) => output.type === "response" && output.id === id)).success).toBe(true);
    await worker.waitFor(() => provider.requests.some((request) => request.text === "Last queued message."), 5_000);
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    const stopped = worker.outputs.find((output) => output.type === "run_finished" && output.runId === "held-run");
    expect(stopped?.outcome).toBe("stopped");
    const users = worker.view?.messages.filter((message) => message.role === "user");
    expect(users?.map((message) => message.blocks[0].text)).toEqual([
      "wait until stopped", "Send this now.", "First queued message.", "Last queued message."
    ]);
    expect(users?.[1].checkpoint).toEqual({ id: "steered-checkpoint" });
    expect(worker.outputs.some((output) => output.type === "worker_error")).toBe(false);
  });

  it("cancels a long-running tool instead of waiting for its next boundary", async () => {
    const { provider, worker, runId, workspace } = await startSlowRun("alpha-secret", "instant-tool", "stream tool until stopped");
    const selected = await enqueue(worker, "Change direction now.");
    worker.send({ id: crypto.randomUUID(), type: "steer_message", messageId: selected, runId: "tool-steered" });
    await worker.waitFor(() => provider.requests.some((request) => request.text === "Change direction now."), 5_000);
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "tool-steered" && output.state === "idle");
    expect(worker.outputs.find((output) => output.type === "run_finished" && output.runId === runId)?.outcome).toBe("stopped");
    await expect(readFile(join(workspace, "completed.txt"))).rejects.toMatchObject({ code: "ENOENT" });
    expect(worker.outputs.some((output) => output.type === "worker_error")).toBe(false);
  });

  it("selects duplicate text by id and preserves images and literal file payloads", async () => {
    const { provider, worker } = await startHeldRun(true);
    const image = { type: "image", mimeType: "image/png", data: solidPng(4, 4) };
    await enqueue(worker, "/brief literal text", { literal: true });
    const selected = await enqueue(worker, "/brief literal text", { literal: true, images: [image] });
    const fileText = "Use this file.\n\n<attached-files>\nnotes.txt: keep the complete file contents\n</attached-files>";
    await enqueue(worker, fileText);
    worker.send({ id: crypto.randomUUID(), type: "steer_message", messageId: selected, runId: "image-steered" });
    await worker.waitFor(() => provider.requests.some((request) => request.text === fileText));
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    const requests = provider.requests.filter((request) => request.text === "/brief literal text");
    const content = (request: typeof requests[number]) => (request.body.messages as Array<{ role: string; content: unknown }>).filter((message) => message.role === "user").at(-1)?.content;
    expect(requests).toHaveLength(3); // selected tool + answer, then the older duplicate
    expect(JSON.stringify(content(requests[0]))).toContain("image_url");
    expect(JSON.stringify(content(requests.at(-1)!))).not.toContain("image_url");
    expect(worker.view?.messages.filter((message) => message.role === "user").map((message) => message.blocks[0].text)).toEqual([
      "wait until stopped", "/brief literal text", "/brief literal text", fileText
    ]);
  });

  it("treats a stale Steer click as a no-op without interrupting newer work", async () => {
    const { provider, worker } = await startHeldRun();
    const selected = await enqueue(worker, "Handle this once.");
    worker.send({ id: crypto.randomUUID(), type: "steer_message", messageId: selected, runId: "once-run" });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "once-run" && output.state === "idle");
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "new-held", message: "wait until stopped" });
    await worker.waitFor(() => provider.requests.filter((request) => request.text === "wait until stopped").length === 2);
    const offset = worker.outputs.length;
    const id = crypto.randomUUID();
    worker.send({ id, type: "steer_message", messageId: selected, runId: "must-not-repeat" });
    expect((await worker.waitFor((output) => output.type === "response" && output.id === id)).success).toBe(true);
    expect(worker.outputs.slice(offset).some((output) => output.type === "run_state")).toBe(false);
    expect(provider.requests.filter((request) => request.text === "Handle this once.")).toHaveLength(2);
    worker.send({ id: crypto.randomUUID(), type: "abort" });
  });

  it("leaves queued work paused after Stop until the user explicitly steers or sends", async () => {
    const { provider, worker } = await startHeldRun();
    const selected = await enqueue(worker, "Keep this pending.");
    worker.send({ id: crypto.randomUUID(), type: "abort" });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "held-run" && output.state === "idle");
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(provider.requests).toHaveLength(1);
    expect(worker.outputs.filter((output) => output.type === "queue_state").at(-1)?.messages).toEqual([{ id: selected, text: "Keep this pending." }]);
    worker.send({ id: crypto.randomUUID(), type: "steer_message", messageId: selected, runId: "resume-pending" });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "resume-pending" && output.state === "idle");
    expect(provider.requests.some((request) => request.text === "Keep this pending.")).toBe(true);
  });

  it("honours queue then Steer sent before init has settled", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-early-steer-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const worker = launchWorker(provider.baseUrl, "alpha-secret", workspace, "early-steer-task");
    cleanup.push(() => worker.shutdown());
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "early-held", message: "wait until stopped" });
    const messageId = crypto.randomUUID();
    worker.send({ id: messageId, type: "queue_message", message: "Early correction." });
    worker.send({ id: crypto.randomUUID(), type: "steer_message", messageId, runId: "early-correction" });
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "early-correction" && output.state === "idle");
    expect(provider.requests.some((request) => request.text === "Early correction.")).toBe(true);
    expect(worker.outputs.find((output) => output.type === "run_finished" && output.runId === "early-held")?.outcome).toBe("stopped");
    expect(worker.outputs.some((output) => output.type === "worker_error")).toBe(false);
  });

  it("keeps provenance on a queued prompt template when it is delivered", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-queued-command-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const prompt = join(workspace, "brief.md");
    await writeFile(prompt, "---\ndescription: Brief\n---\nBrief $ARGUMENTS\n");
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "queued-command-task", undefined, undefined,
      { extensions: [], skills: [], prompts: [prompt], themes: [] });
    cleanup.push(() => worker.shutdown());
    const listId = crypto.randomUUID();
    worker.send({ id: listId, type: "list_commands" });
    await worker.waitFor((output) => output.type === "response" && output.id === listId && output.success === true);

    const runId = crypto.randomUUID();
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId, message: "stream tool" });
    await worker.waitFor((output) =>
      output.type === "activity" && output.event === "tool_execution_update" && output.detail?.text?.includes("first line") === true);
    const queueId = crypto.randomUUID();
    worker.send({ id: queueId, type: "queue_message", message: "/brief the diff" });
    await worker.waitFor((output) => output.type === "response" && output.id === queueId && output.success === true);
    // The same command provenance survives instant promotion, not just natural delivery.
    worker.send({ id: crypto.randomUUID(), type: "steer_message", messageId: queueId, runId: "command-steered" });
    await worker.waitFor(() => provider.requests.some((request) => request.text.includes("Brief the diff")));
    await worker.waitFor((output) => output.type === "run_state" && output.runId === "command-steered" && output.state === "idle");

    const delivered = [...(worker.view?.messages ?? [])].reverse().find((message) =>
      message.role === "user" && message.blocks.some((block) => block.text?.includes("Brief the diff")));
    expect(delivered?.commandPresentation).toEqual({
      id: `prompt:${prompt}`, name: "brief", arguments: "the diff", kind: "command"
    });
  });

  it("restores queued messages to the caller without delivering them", async () => {
    const { provider, worker, runId } = await startSlowRun("alpha-secret", "dequeue");
    await enqueue(worker, "Keep this one back.");
    await enqueue(worker, "And this later.");
    await worker.waitFor((output) => output.type === "queue_state" && output.messages?.length === 2);

    const id = crypto.randomUUID();
    worker.send({ id, type: "dequeue" });
    const cleared = await worker.waitFor((output) => output.type === "response" && output.id === id);
    expect(cleared.success).toBe(true);
    expect(cleared.result).toEqual({ steering: [], followUp: ["Keep this one back.", "And this later."] });

    await worker.waitFor((output) => output.type === "run_state" && output.runId === runId && output.state === "idle");
    expect(provider.requests.some((request) => request.text.includes("Keep this one back.") || request.text.includes("And this later."))).toBe(false);
    const queueStates = worker.outputs.filter((output) => output.type === "queue_state");
    expect(queueStates[queueStates.length - 1]?.messages).toEqual([]);
    expect(worker.view?.messages.some((message) =>
      message.blocks.some((block) => block.text === "Keep this one back." || block.text === "And this later."))).toBe(false);
  });

  it("runs a queued message as a fresh prompt when the run has already settled", async () => {
    const { provider, worker, runId } = await startSlowRun("alpha-secret", "queue-fallback");
    await worker.waitFor((output) => output.type === "run_state" && output.runId === runId && output.state === "idle");
    const before = provider.requests.length;

    const queueId = crypto.randomUUID();
    const outputsBefore = worker.outputs.length;
    worker.send({ id: queueId, type: "queue_message", message: "Fresh prompt for an idle chat." });
    const accepted = await worker.waitFor((output) => output.type === "response" && output.id === queueId);
    expect(accepted.success).toBe(true);
    await worker.waitFor(() => provider.requests.length > before && provider.requests[before].text === "Fresh prompt for an idle chat.");
    // Acceptance is immediate; a fresh run starts once the serial queue is available.
    const runnings = worker.outputs.filter((output, index) =>
      index >= outputsBefore && output.type === "run_state" && output.state === "running");
    expect(runnings).toHaveLength(1);
    const newRunId = runnings[0]?.runId;
    expect(newRunId).toBeTruthy();
    expect(newRunId).not.toBe(runId);
    await worker.waitFor((output) => output.type === "run_state" && output.runId === newRunId && output.state === "idle");
    expect(worker.view?.messages.some((message) =>
      message.role === "user" && message.blocks.some((block) => block.text === "Fresh prompt for an idle chat."))).toBe(true);
  });

  it("iterates a goal until the verifier passes, without flashing idle between rounds", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-goal-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "goal-task");
    cleanup.push(() => worker.shutdown());

    // Fail the first check, pass the second: one continuation, then done.
    provider.verdicts.push(
      '{"passed": false, "reason": "nothing written yet", "nextAction": "write the file"}',
    );
    worker.send({
      id: crypto.randomUUID(), type: "goal_control", action: "set",
      objective: "goal: write the file", runId: "goal-run", startedAt: Date.now()
    });

    const done = await worker.waitFor((output) => output.type === "goal_state" && output.goal?.phase === "complete");
    expect(done.goal).toMatchObject({ objective: "goal: write the file", iteration: 2 });

    // Phase sequence: active → verifying → active → verifying → complete. (The worker also
    // publishes a null goal at session start, which clears any stale banner.)
    const phases = worker.outputs
      .filter((output) => output.type === "goal_state" && output.goal != null)
      .map((output) => output.goal?.phase);
    expect(phases).toEqual(["active", "verifying", "active", "verifying", "complete"]);

    // Two working turns plus two verifier calls, all on the chat's own model; the verifier
    // call carries no tools and the continuation re-states the gap and next action.
    const verifierRequests = provider.requests.filter((request) => request.text.startsWith("<goal>"));
    expect(verifierRequests).toHaveLength(2);
    expect(worker.outputs.filter((o) => o.record?.purpose === "goal_verification")).toHaveLength(2);
    for (const request of verifierRequests) expect(request.body.tools).toBeUndefined();
    const continuation = provider.requests.find((request) => request.text.startsWith("Goal continuation"));
    expect(continuation?.text).toContain("write the file");
    expect(continuation?.text).toContain("nothing written yet");

    // The loop's continuation is a nested run outside runPrompt, so exactly one idle arrives —
    // at the end. The settled handler's suppression keeps the chat busy between rounds.
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle");
    expect(worker.outputs.filter((output) => output.type === "run_state" && output.state === "idle")).toHaveLength(1);
    expect(worker.outputs.filter((output) => output.type === "run_state" && output.state === "running")).toHaveLength(1);
    expect(worker.outputs.filter((output) => output.type === "run_finished")).toEqual([
      expect.objectContaining({ runId: "goal-run", outcome: "completed" })
    ]);
    const goalMessages = (worker.view?.messages ?? []).filter((message) => message.role === "user");
    expect(goalMessages[0]?.commandPresentation).toEqual({
      id: "app:goal", name: "goal", arguments: "goal: write the file", kind: "command"
    });
    expect(goalMessages[1]?.commandPresentation).toEqual({
      id: "app:goal", name: "goal", arguments: "", kind: "goal-continuation", round: 2, nextAction: "write the file"
    });
  });

  it("fails a goal open on a malformed verdict and stops on a dead end", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-goal-open-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "goal-open-task");
    cleanup.push(() => worker.shutdown());

    provider.verdicts.push("the verifier rambled without JSON");
    worker.send({
      id: crypto.randomUUID(), type: "goal_control", action: "set",
      objective: "goal: malformed", runId: "goal-open", startedAt: Date.now()
    });
    const done = await worker.waitFor((output) => output.type === "goal_state" && output.goal?.phase === "complete");
    expect(done.goal?.note ?? done.goal?.lastReason).toBeTruthy();

    // A clean fail that names no next step stops the loop rather than spinning.
    const workspace2 = await mkdtemp(join(tmpdir(), "wackcode-goal-stop-"));
    cleanup.push(() => rm(workspace2, { recursive: true, force: true }));
    const { worker: worker2 } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace2, "goal-stop-task");
    cleanup.push(() => worker2.shutdown());
    provider.verdicts.push('{"passed": false, "reason": "nothing useful left"}');
    worker2.send({
      id: crypto.randomUUID(), type: "goal_control", action: "set",
      objective: "goal: dead end", runId: "goal-stop", startedAt: Date.now()
    });
    const stopped = await worker2.waitFor((output) => output.type === "goal_state" && output.goal?.phase === "stopped");
    expect(stopped.goal?.note ?? stopped.goal?.lastReason).toContain("nothing useful left");
    expect(provider.requests.filter((request) => request.text.startsWith("Goal continuation"))).toHaveLength(0);
  });

  it("pauses a live goal with goal_control and resumes it from idle", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-goal-pause-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "goal-pause-task");
    cleanup.push(() => worker.shutdown());

    provider.verdicts.push(
      '{"passed": false, "reason": "round one", "nextAction": "step a"}',
      '{"passed": false, "reason": "round two", "nextAction": "step b"}',
    );
    worker.send({
      id: crypto.randomUUID(), type: "goal_control", action: "set",
      objective: "goal: pausable", runId: "goal-pause", startedAt: Date.now()
    });

    // Pause lands mid-run (the command bypasses the serial queue); the paused state sticks.
    await worker.waitFor((output) => output.type === "goal_state" && output.goal?.phase === "verifying");
    const pauseId = crypto.randomUUID();
    worker.send({ id: pauseId, type: "goal_control", action: "pause" });
    const paused = await worker.waitFor((output) => output.type === "goal_state" && output.goal?.phase === "paused");
    expect(paused.goal?.note).toBe("Paused.");

    // Resume from a settled chat re-kicks a run; the next verdict continues it once more,
    // then the default (no scripted verdict) completes it.
    const resumeId = crypto.randomUUID();
    worker.send({ id: resumeId, type: "goal_control", action: "resume", runId: "goal-resume", startedAt: Date.now() });
    await worker.waitFor((output) => output.type === "goal_state" && output.goal?.phase === "complete");
    expect(provider.requests.filter((request) => request.text.startsWith("Goal continuation")).length).toBeGreaterThanOrEqual(1);
    expect((worker.view?.messages ?? []).some((message) => message.commandPresentation?.kind === "goal-resume")).toBe(true);

    const clearId = crypto.randomUUID();
    worker.send({ id: clearId, type: "goal_control", action: "clear" });
    await worker.waitFor((output) => output.type === "goal_state" && output.goal === null);
    // The state clears in snapshots too — the delta carries goalState:null, debounced ~32ms.
    await worker.waitFor((output) => output.type === "snapshot_delta" && output.delta?.goalState === null);
    expect(worker.view?.goalState).toBeUndefined();
  });

  it("rejects /goal while a planning mode is on", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-goal-plan-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "goal-plan-task", undefined, undefined, undefined, "plan");
    cleanup.push(() => worker.shutdown());

    const id = crypto.randomUUID();
    worker.send({ id, type: "goal_control", action: "set", objective: "goal: in plan mode", runId: "goal-plan", startedAt: Date.now() });
    const result = await worker.waitFor((output) => output.type === "response" && output.id === id);
    expect(result.success).toBe(false);
    expect(result.error).toContain("planning mode");
    expect(provider.requests).toHaveLength(0);
  });

  it("runs /skill-creator through prepare and a terminating preview card", async () => {
    const provider = await startMockProvider();
    cleanup.push(provider.close);
    const workspace = await mkdtemp(join(tmpdir(), "wackcode-skill-creator-"));
    cleanup.push(() => rm(workspace, { recursive: true, force: true }));
    const { worker, ready } = await initializeWorker(provider.baseUrl, "alpha-secret", workspace, "skill-creator-task");
    cleanup.push(() => worker.shutdown());
    expect(ready.snapshot?.tools?.some((tool) => tool.name === "skill_creator")).toBe(true);
    expect(ready.snapshot?.skillCreator).toBeUndefined();

    // The harness stands in for the Rust host: it answers the worker's skill_creator requests,
    // creating the managed draft under the task's agent dir like `skill_creator.rs` would.
    const agentDir = join(workspace, ".agent", "skill-creator-task");
    const revision = "f".repeat(64);
    let draftId = "";
    const answer = (request: Output) => {
      const payload = request.request ?? {};
      if (payload.op === "prepare") {
        draftId = crypto.randomUUID();
        const draftRoot = join(agentDir, "skill-creator", draftId);
        const skillDir = join(draftRoot, "skill");
        void mkdir(skillDir, { recursive: true }).then(() =>
          writeFile(join(skillDir, "SKILL.md"), "---\nname: greeting-skill\ndescription: Write a short friendly greeting.\n---\n\nGreet the user warmly.\n"));
        worker.send({ id: crypto.randomUUID(), type: "skill_creator_response", requestId: request.requestId, success: true,
          result: { draftId, draftRoot, skillDir, evalsDir: join(draftRoot, "evals"), name: "greeting-skill" } });
      } else {
        worker.send({ id: crypto.randomUUID(), type: "skill_creator_response", requestId: request.requestId, success: true,
          result: { draftId: String(payload.draftId ?? draftId), revision, name: "greeting-skill", description: "Write a short friendly greeting.",
            manual: false, target: "new", bodyPreview: "Greet the user warmly.", bodyTruncated: false, files: [], fileCount: 1, totalBytes: 96, warnings: [] } });
      }
    };
    const answered = new Set<string>();
    const answerRequests = (async () => {
      for (let round = 0; round < 2; round += 1) {
        const request = await worker.waitFor((output) =>
          output.type === "skill_creator_request" && !answered.has(output.requestId ?? ""));
        answered.add(request.requestId ?? "");
        answer(request);
      }
    })();

    worker.send({ id: crypto.randomUUID(), type: "skill_creator", request: "make a greeting skill", runId: "skill-creator-run", startedAt: Date.now() });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "skill-creator-run");
    await answerRequests;

    // Two model turns: the guide prompt (prepare), then the authoring turn (preview). The
    // preview's terminate:true ends the run without a third request.
    expect(provider.requests).toHaveLength(2);
    expect(provider.requests[0].text).toContain("create or improve an Agent Skill");
    expect(JSON.stringify(provider.requests[0].body.tools)).toContain("skill_creator");
    // Both native requests were answered on the queue-bypassing channel.
    const requests = worker.outputs.filter((output) => output.type === "skill_creator_request");
    expect(requests).toHaveLength(2);
    expect(requests[0].request?.op).toBe("prepare");
    expect(requests[1].request?.op).toBe("preview");

    // The card's details landed in the transcript, carried by the tool result.
    const details = worker.view?.messages.flatMap((message) => message.blocks)
      .filter((block) => block.type === "tool-result" && block.toolName === "skill_creator" && block.details !== undefined)
      .at(-1);
    expect(details?.isError).toBeFalsy();
    expect(details?.details).toMatchObject({ v: 1, source: "skill_creator_preview", ownerTaskId: "skill-creator-task", revision });
    // The command-generated prompt is labelled, and the workflow state reached the snapshot.
    const prompt = worker.view?.messages.find((message) => message.commandPresentation?.id === "app:skill-creator");
    expect(prompt?.blocks.some((block) => block.type === "text" && block.text?.includes("make a greeting skill"))).toBe(true);
    // The state publish is debounced behind the run's own emissions; wait for it explicitly.
    await worker.waitFor((output) => output.view?.skillCreator !== undefined);
    expect(worker.view?.skillCreator).toEqual({ draftId, name: "greeting-skill", revision });

    // A feedback turn (an ordinary prompt, no command run) still previews: the workflow entry
    // carries the draft.
    const followUpAnswer = worker.waitFor((output) =>
      output.type === "skill_creator_request" && !answered.has(output.requestId ?? ""))
      .then((request) => {
        answered.add(request.requestId ?? "");
        worker.send({ id: crypto.randomUUID(), type: "skill_creator_response", requestId: request.requestId, success: true,
          result: { draftId, revision, name: "greeting-skill", description: "Warmer.", manual: true, target: "new",
            bodyPreview: "Warmer.", bodyTruncated: false, files: [], fileCount: 1, totalBytes: 96, warnings: [] } });
      });
    worker.send({ id: crypto.randomUUID(), type: "prompt", runId: "feedback-run", message: "skill preview again" });
    await worker.waitFor((output) => output.type === "run_state" && output.state === "idle" && output.runId === "feedback-run");
    await followUpAnswer;
    expect(worker.view?.skillCreator).toMatchObject({ draftId, name: "greeting-skill", revision });
    expect(worker.child.exitCode).toBeNull();
  });
});
