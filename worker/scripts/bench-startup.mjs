/**
 * Cold-send benchmark: how long from `node worker.js` spawning until a model request leaves
 * it, which is the wall a chat's first message waits on.
 *
 * Spawns the built worker in a temporary git workspace, sends `init` + `prompt` back to back
 * the way the host does, and reports per-run and median timings:
 *
 *   spawn→ready     worker boot + bundle eval + init (session, tools, extensions)
 *   spawn→running   the prompt command reaching runPrompt
 *   spawn→request   the provider seeing the chat request — the user-visible "send" cost
 *   spawn→body      the request fully written
 *   rss@sample      the worker's RSS after the request (and MCP discovery, when enabled)
 *
 *   node scripts/bench-startup.mjs           # 5 runs against dist/worker.js
 *   RUNS=10 WORKER=/path/to/worker.js node scripts/bench-startup.mjs
 *   MCP_DELAY_MS=5000 RUNS=6 node scripts/bench-startup.mjs  # cold discovery + 5 cached chats
 */
import { spawn, execSync } from "node:child_process";
import { createServer } from "node:http";
import { createInterface } from "node:readline";
import { existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { performance } from "node:perf_hooks";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const WORKER = process.env.WORKER ?? join(root, "dist", "worker.js");
const RUNS = Number(process.env.RUNS ?? 5);
const MCP_DELAY_MS = Number(process.env.MCP_DELAY_MS ?? 0);
// Shared app-owned agent root, just like independent chats in the desktop app.
const agentRoot = mkdtempSync(join(tmpdir(), "wc-bench-agent-"));
const mcpReadyFile = join(agentRoot, "mcp-ready");

/** A Chat-Completions mock that answers one streamed sentence and records arrival times. */
async function startProvider() {
  const marks = {};
  const server = createServer((request, response) => {
    marks.requestAt ??= performance.now();
    const chunks = [];
    request.on("data", (chunk) => chunks.push(chunk));
    request.on("end", () => {
      marks.bodyAt ??= performance.now();
      const body = JSON.parse(Buffer.concat(chunks).toString());
      marks.mcpTools = (body.tools ?? []).filter((tool) => tool.function.name.startsWith("mcp__")).length;
      response.writeHead(200, { "content-type": "text/event-stream" });
      const send = (value) => response.write(`data: ${JSON.stringify(value)}\n\n`);
      send({ id: "x", object: "chat.completion.chunk", created: 1, model: "shared-model",
        choices: [{ index: 0, delta: { role: "assistant", content: "ok" } }] });
      send({ id: "x", object: "chat.completion.chunk", created: 2, model: "shared-model",
        choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
        usage: { prompt_tokens: 10, completion_tokens: 2, total_tokens: 12 } });
      response.end("data: [DONE]\n\n");
    });
  });
  await new Promise((resolvePromise) => server.listen(0, "127.0.0.1", resolvePromise));
  return { url: `http://127.0.0.1:${server.address().port}/v1`, marks, close: () => server.close() };
}

async function run(index) {
  const provider = await startProvider();
  rmSync(mcpReadyFile, { force: true });
  const workspace = mkdtempSync(join(tmpdir(), "wc-bench-"));
  execSync("git init -q && git config user.email bench@local && git config user.name bench && echo x > a.txt && git add a.txt && git commit -qm init", { cwd: workspace, shell: "/bin/sh" });
  mkdirSync(join(agentRoot, String(index)), { recursive: true });
  mkdirSync(join(workspace, ".sessions", "task"), { recursive: true });

  const t0 = performance.now();
  const child = spawn(process.env.WACKCODE_TEST_NODE ?? process.execPath, [WORKER], {
    cwd: workspace,
    env: { ...process.env, PI_OFFLINE: "1", PI_TELEMETRY: "0", PI_SKIP_VERSION_CHECK: "1" },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const marks = { spawnAt: t0 };
  let stderr = "";
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  createInterface({ input: child.stdout }).on("line", (line) => {
    let output;
    try { output = JSON.parse(line); } catch { return; }
    if (output.type === "ready" && marks.readyAt === undefined) marks.readyAt = performance.now();
    if (output.type === "run_state" && output.state === "running" && marks.runningAt === undefined) marks.runningAt = performance.now();
  });
  const send = (value) => child.stdin.write(`${JSON.stringify(value)}\n`);
  send({ id: "init-1", type: "init", taskId: "task", cwd: workspace,
    agentDir: join(agentRoot, String(index)), sessionDir: join(workspace, ".sessions", "task"),
    provider: { id: "bench", name: "Bench", kind: "custom", baseUrl: provider.url, api: "openai-completions",
      models: [{ id: "shared-model", name: "Shared", contextWindow: 32_768, maxTokens: 321, reasoning: true,
        thinkingLevels: ["off", "high"], thinkingLevelMap: { off: null, high: "wire-high" } }] },
    modelId: "shared-model", apiKey: "bench-secret", thinkingLevel: "high",
    disabledTools: [], mcp: MCP_DELAY_MS ? [{ id: "mock", name: "Mock", slug: "mock", transport: "stdio",
      command: process.execPath, args: [resolve(root, "../scripts/mock-mcp-server.mjs"), "--startup-delay", String(MCP_DELAY_MS), "--ready-file", mcpReadyFile],
      env: {}, disabledTools: [], timeoutMs: 15_000 }] : [], prompts: {}, web: { fetchRoute: "host" } });
  send({ id: "p-1", type: "prompt", runId: "run-1", message: "hi", startedAt: Date.now(), mode: "build", literal: false });

  const deadline = Date.now() + 30_000;
  while ((provider.marks.bodyAt === undefined || (MCP_DELAY_MS && marks.mcpAt === undefined)) && Date.now() < deadline && child.exitCode === null) {
    if (MCP_DELAY_MS && marks.mcpAt === undefined && existsSync(mcpReadyFile)) marks.mcpAt = performance.now();
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 5));
  }
  const rssKb = (() => { try { return Number(execSync(`ps -o rss= -p ${child.pid}`).toString().trim()); } catch { return null; } })();
  send({ id: "s", type: "shutdown" });
  await new Promise((resolvePromise) => { child.once("exit", resolvePromise); setTimeout(resolvePromise, 1_500); });
  if (child.exitCode === null) child.kill("SIGKILL");
  provider.close();
  rmSync(workspace, { recursive: true, force: true });
  const ms = (a, b) => (a === undefined || b === undefined ? null : Math.round(a - b));
  return { run: index,
    "spawn→ready": ms(marks.readyAt, t0),
    "spawn→running": ms(marks.runningAt, t0),
    "spawn→request": ms(provider.marks.requestAt, t0),
    "spawn→body": ms(provider.marks.bodyAt, t0),
    "ready→request": ms(provider.marks.requestAt, marks.readyAt),
    ...(MCP_DELAY_MS ? { "spawn→mcp": ms(marks.mcpAt, t0), mcpTools: provider.marks.mcpTools ?? 0 } : {}),
    "rss@sample": rssKb === null ? null : Math.round(rssKb / 1024),
    stderr: stderr.trim().slice(0, 200) || undefined };
}

const rows = [];
for (let i = 0; i < RUNS; i++) rows.push(await run(i));
const keys = Object.keys(rows[0]).filter((key) => !["run", "stderr", "mcpTools"].includes(key));
const median = (key, samples = rows) => {
  const values = samples.map((row) => row[key]).filter((value) => value !== null).sort((a, b) => a - b);
  return values.length ? values[Math.floor(values.length / 2)] : null;
};
for (const row of rows) console.log(JSON.stringify(row));
console.log("--- medians ---");
for (const key of keys) console.log(`${key}: ${median(key)}${key === "rss@sample" ? " MB" : " ms"}`);
if (MCP_DELAY_MS && rows.length > 1) {
  console.log("--- subsequent-chat medians (shared catalogue) ---");
  for (const key of keys) console.log(`${key}: ${median(key, rows.slice(1))}${key === "rss@sample" ? " MB" : " ms"}`);
}
if (rows.some((row) => row.stderr)) console.log("--- worker stderr ---\n" + rows.map((row) => row.stderr).filter(Boolean).join("\n"));
rmSync(agentRoot, { recursive: true, force: true });
if (rows.some((row) => row["spawn→request"] === null || (MCP_DELAY_MS && (row.mcpTools !== 5 || row["spawn→mcp"] === null)))) {
  process.exitCode = 1;
}
