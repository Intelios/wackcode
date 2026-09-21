import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
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
  snapshot?: {
    sessionFile?: string;
    messages: Array<{ blocks: Array<{ type: string; text?: string }> }>;
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
  sessionFile?: string
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
    thinkingLevel: "high"
  });
  return { worker, ready: await worker.waitFor((output) => output.type === "ready") };
}

const cleanup: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const action of cleanup.splice(0).reverse()) await action();
});

describe("Pi worker integration", () => {
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
