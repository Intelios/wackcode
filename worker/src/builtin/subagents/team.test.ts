import { afterEach, describe, expect, it, vi } from "vitest";
import type { ToolDefinition } from "@earendil-works/pi-coding-agent";
import { createSubagentsExtension } from "./index.js";
import { KeptSessions } from "./kept.js";
import { emptyUsage } from "./details.js";
import { normalizeSubagentParams } from "./schema.js";
import { subagentDescription, subagentGuidelines } from "./prompt.js";
import type { BuiltinHost, SubagentOutcome, SubagentRunRequest } from "../host.js";
import type { SubagentSpec } from "../../protocol.js";

const cleanup: Array<() => Promise<void>> = [];
afterEach(async () => { for (const close of cleanup.splice(0)) await close(); });
const text = (result: any) => result.content.map((part: { text: string }) => part.text).join("\n");

const WORKER: SubagentSpec = { name: "worker", description: "Edits", prompt: "Instructions", tools: ["read", "write"], readOnly: false };
const SCOUT: SubagentSpec = { name: "scout", description: "Reads", prompt: "Instructions", tools: ["read"], readOnly: true };

interface Harness {
  launch(params: unknown, signal?: AbortSignal, onUpdate?: (value: any) => void): Promise<any>;
  manage(params: unknown, signal?: AbortSignal): Promise<any>;
  controller: ReturnType<typeof createSubagentsExtension>["controller"];
  runs: Array<{ request: SubagentRunRequest; finish: (status?: SubagentOutcome["status"], error?: string) => void }>;
  tools: Map<string, ToolDefinition>;
  /** The retained sessions the extension sees, keyed by jobId. */
  kept: Map<string, { spec: SubagentSpec; rounds: number; busy: boolean; jobId: string }>;
  dropped: number;
  usage: ReturnType<typeof vi.fn>;
}

/** The extension with a Team payload plus a fake retained-session registry on the host. */
async function harness(team = true): Promise<Harness> {
  const runs: Harness["runs"] = [];
  const tools = new Map<string, ToolDefinition>();
  const usage = vi.fn();
  const kept: Harness["kept"] = new Map();
  const h: Partial<Harness> = { dropped: 0 };
  const host = {
    childToolNames: () => ["read", "write"],
    redact: (value: string) => value,
    recordSubagentUsage: usage,
    retainedSubagent: (jobId: string) => kept.get(jobId),
    dropRetainedSubagents: () => { kept.clear(); h.dropped! += 1; },
    runSubagent: (request: SubagentRunRequest) => new Promise<SubagentOutcome>((resolve) => {
      const finish = (status: SubagentOutcome["status"] = "done", error?: string) => {
        request.signal?.removeEventListener("abort", abort);
        const spent = emptyUsage(); spent.input = 10; spent.totalTokens = 10;
        resolve({ status, output: `answer ${request.task}`, error, usage: spent, turns: 1 });
      };
      const abort = () => finish("aborted");
      runs.push({ request, finish });
      request.signal?.addEventListener("abort", abort, { once: true });
      request.observer.started("Fixture model");
      if (request.signal?.aborted) abort();
    }),
  } as unknown as BuiltinHost;
  const extension = createSubagentsExtension(host, () => "build", { name: "fixture", factory: () => {} });
  extension.factory({ registerTool: (tool: ToolDefinition) => tools.set(tool.name, tool), on: () => () => {}, appendEntry: () => {} } as never);
  await extension.controller.configure({
    trigger: "on_request", maxConcurrency: 8,
    agents: [WORKER, SCOUT], providers: [],
    ...(team ? { team: { label: "Cheap · flash" } } : {}),
  });
  cleanup.push(extension.controller.stopAll);
  let calls = 0;
  const call = (name: string, params: unknown, signal?: AbortSignal, onUpdate?: (value: any) => void) =>
    tools.get(name)!.execute(`launch-${++calls}`, params, signal, onUpdate, {} as never) as Promise<any>;
  Object.assign(h, {
    launch: (params: unknown, signal?: AbortSignal, onUpdate?: (value: any) => void) => call("subagent", params, signal, onUpdate),
    manage: (params: unknown, signal?: AbortSignal) => call("subagent_job", params, signal),
    controller: extension.controller, runs, tools, kept, usage,
  });
  return h as Harness;
}

/** Register a finished child as retained, as the runner would after its first round. */
function retain(h: Harness, jobId: string, rounds = 1, busy = false): void {
  h.kept.set(jobId, { spec: WORKER, rounds, busy, jobId });
}

describe("team continuations", () => {
  it("offers `job` only while the chat's Team is on", async () => {
    const withTeam = await harness(true);
    const params = withTeam.tools.get("subagent")!.parameters as any;
    expect(params.properties.job).toBeDefined();
    expect(params.properties.tasks.items.properties.job).toBeDefined();
    const withoutTeam = await harness(false);
    const plain = withoutTeam.tools.get("subagent")!.parameters as any;
    expect(plain.properties.job).toBeUndefined();
    expect(plain.properties.tasks.items.properties.job).toBeUndefined();
  });

  it("parses a continuation like a launch, refusing agent+job together and job without Team", () => {
    expect(normalizeSubagentParams({ job: "j1", task: "fix x" }, true)).toEqual({
      ok: true, mode: "single", tasks: [{ agent: "job:j1", task: "fix x", job: "j1" }],
    });
    expect(normalizeSubagentParams({ agent: "worker", job: "j1", task: "x" }, true).ok).toBe(false);
    expect(normalizeSubagentParams({ job: "j1", task: "x" }, false).ok).toBe(false);
    expect(normalizeSubagentParams({ tasks: [{ job: "j1", task: "x" }] }, false).ok).toBe(false);
    expect(normalizeSubagentParams({ tasks: [{ agent: "worker", job: "j1", task: "x" }] }, true).ok).toBe(false);
    expect(normalizeSubagentParams({ job: "j1" }, true).ok).toBe(false);
  });

  it("continues a finished worker with its own jobId, role and next round", async () => {
    const h = await harness(true);
    // Blocking: start, let the child begin, finish it, then read the settled launch.
    const first = h.launch({ agent: "worker", task: "first" });
    await vi.waitFor(() => expect(h.runs).toHaveLength(1));
    h.runs[0].finish();
    const launched = await first;
    // Team chats hand every child a jobId, blocking launches included.
    const jobId = launched.details.results[0].jobId;
    expect(jobId).toBeTruthy();
    expect(h.runs[0].request.jobId).toBe(jobId);
    retain(h, jobId);

    const again = h.launch({ job: jobId, task: "change x to y" });
    await vi.waitFor(() => expect(h.runs).toHaveLength(2));
    h.runs[1].finish();
    const result = (await again).details.results[0];
    expect(result.agent).toBe("worker");
    expect(result.round).toBe(2);
    expect(result.jobId).toBe(jobId);
    expect(h.runs[1].request.continuation).toBe(jobId);
    expect(h.runs[1].request.jobId).toBe(jobId);
  });

  it("mixes continuations and fresh launches in one call", async () => {
    const h = await harness(true);
    const first = h.launch({ agent: "worker", task: "a" });
    await vi.waitFor(() => expect(h.runs).toHaveLength(1));
    h.runs[0].finish();
    const jobId = (await first).details.results[0].jobId;
    retain(h, jobId);
    const both = h.launch({ tasks: [{ job: jobId, task: "redo a" }, { agent: "scout", task: "survey" }] });
    await vi.waitFor(() => expect(h.runs).toHaveLength(3));
    h.runs[1].finish(); h.runs[2].finish();
    const results = (await both).details.results;
    expect(results.map((result: any) => [result.agent, result.round])).toEqual([["worker", 2], ["scout", undefined]]);
    expect(h.runs[1].request.continuation).toBe(jobId);
    expect(h.runs[2].request.continuation).toBeUndefined();
  });

  it("refuses a second continuation of the same worker in one call", async () => {
    const h = await harness(true);
    retain(h, "j1");
    await expect(h.launch({ tasks: [{ job: "j1", task: "x" }, { job: "j1", task: "y" }] })).rejects.toThrow("twice in one call");
    expect(h.runs).toHaveLength(0);
  });

  it("refuses a worker whose session is gone or still running", async () => {
    const h = await harness(true);
    await expect(h.launch({ job: "gone", task: "x" })).rejects.toThrow("session has ended");

    // Still running: its job exists without an outcome.
    const slow = h.launch({ background: true, agent: "worker", task: "slow" });
    await vi.waitFor(() => expect(h.runs).toHaveLength(1));
    const runningId = (await slow).details.results[0].jobId;
    retain(h, runningId);
    await expect(h.launch({ job: runningId, task: "x" })).rejects.toThrow("still running");
    h.runs[0].finish();
    await vi.waitFor(() => expect(h.controller.pendingCount()).toBe(1));
  });

  it("refuses a continuation whose result has not been delivered, then allows it after wait", async () => {
    const h = await harness(true);
    const pending = h.launch({ background: true, agent: "worker", task: "done" });
    await vi.waitFor(() => expect(h.runs).toHaveLength(1));
    const id = (await pending).details.results[0].jobId;
    h.runs[0].finish();
    await vi.waitFor(() => expect(h.controller.pendingCount()).toBe(1));
    retain(h, id);
    await expect(h.launch({ job: id, task: "x" })).rejects.toThrow("not reached you yet");
    // Delivered results can be continued.
    await h.manage({ action: "wait", jobIds: [id] });
    const again = h.launch({ job: id, task: "revise" });
    await vi.waitFor(() => expect(h.runs).toHaveLength(2));
    h.runs[1].finish();
    const result = (await again).details.results[0];
    expect(result.round).toBe(2);
    expect(result.jobId).toBe(id);
  });

  it("drops retained sessions when Team turns off or sub-agents are disabled", async () => {
    const h = await harness(true);
    retain(h, "j1");
    await h.controller.configure({ trigger: "on_request", maxConcurrency: 8, agents: [WORKER], providers: [] });
    expect(h.dropped).toBe(1);
    await h.controller.configure(null);
    expect(h.dropped).toBe(2);
  });

  it("resolves a continued role from the current roster instead of its old access grant", async () => {
    const h = await harness(true);
    h.kept.set("j1", { spec: { ...SCOUT, readOnly: false, tools: ["read", "write"] }, rounds: 1, busy: false, jobId: "j1" });
    const next = h.launch({ job: "j1", task: "review again" });
    await vi.waitFor(() => expect(h.runs).toHaveLength(1));
    expect(h.runs[0].request.spec.readOnly).toBe(true);
    expect(h.runs[0].request.tools).not.toContain("write");
    h.runs[0].finish();
    expect((await next).details.results[0].readOnly).toBe(true);
  });

  it("refuses to continue a role that was switched off", async () => {
    const h = await harness(true);
    retain(h, "j1");
    await h.controller.configure({ trigger: "on_request", maxConcurrency: 8, agents: [SCOUT], providers: [], team: { label: "Crew" } });
    await expect(h.launch({ job: "j1", task: "edit again" })).rejects.toThrow("role is no longer enabled");
    expect(h.runs).toHaveLength(0);
  });
});

describe("team prompts and status", () => {
  it("describes continuations in the tool text and delegates-first guidance", () => {
    const description = subagentDescription([WORKER], true);
    expect(description).toContain("jobId");
    expect(subagentDescription([WORKER], false)).not.toContain("continues with all its context");
    const guidelines = subagentGuidelines("on_request", true, true);
    expect(guidelines[0]).toContain("Team is on");
    expect(guidelines.some((line) => line.includes("jobId"))).toBe(true);
    expect(subagentGuidelines("auto", true, false)[0]).toContain("clearly benefits");
  });

  it("reports a running child's turns and recent activity on status", async () => {
    const h = await harness(true);
    const launched = await h.launch({ background: true, agent: "worker", task: "slow" });
    const id = launched.details.results[0].jobId;
    await vi.waitFor(() => expect(h.runs).toHaveLength(1));
    h.runs[0].request.observer.tool("bash", { command: "pnpm test" });
    h.runs[0].request.observer.tool("read", { path: "src/a.ts" });
    h.runs[0].request.observer.usage({ ...emptyUsage(), input: 5, totalTokens: 5 }, 2);
    const status = text(await h.manage({ action: "status", jobIds: [id] }));
    expect(status).toContain("running");
    expect(status).toContain("Turns so far: 2");
    expect(status).toContain("bash: pnpm test");
    expect(status).toContain("read: src/a.ts");
    h.runs[0].finish();
  });
});

describe("kept sessions", () => {
  interface FakeSession { id: string; disposed: boolean }
  const session = (id: string): FakeSession => ({ id, disposed: false });
  const registry = (max = 8) => new KeptSessions<FakeSession>(max, (s) => { s.disposed = true; });

  it("answers by latest or earlier jobId and tracks rounds and busy", () => {
    const kept = registry();
    const record = kept.put("j1", { session: session("a"), data: {} });
    expect(kept.get("j1")).toBe(record);
    expect(record.rounds).toBe(1);
    kept.begin(record, "j2");
    expect(record.rounds).toBe(2);
    expect(record.busy).toBe(true);
    expect(record.jobId).toBe("j2");
    expect(kept.get("j2")).toBe(record);
    // The launch's own jobId still resolves to the same worker.
    expect(kept.get("j1")).toBe(record);
    kept.end(record);
    expect(record.busy).toBe(false);
  });

  it("evicts the least recently used past the cap and disposes it", () => {
    const kept = registry(2);
    const a = kept.put("a", { session: session("a"), data: {} });
    const b = kept.put("b", { session: session("b"), data: {} });
    const c = kept.put("c", { session: session("c"), data: {} });
    expect(kept.get("a")).toBeUndefined();
    expect(a.session.disposed).toBe(true);
    expect(kept.get("b")).toBe(b);
    expect(kept.get("c")).toBe(c);
    // A continuation bumps a record past the newer one.
    kept.begin(b, "b2");
    kept.put("d", { session: session("d"), data: {} });
    expect(kept.get("c")).toBeUndefined();
    expect(kept.get("b2")).toBe(b);
    kept.end(b);
  });

  it("clears everything, aliases included, exactly once per record", () => {
    const kept = registry();
    const record = kept.put("j1", { session: session("a"), data: {} });
    kept.begin(record, "j2");
    kept.end(record);
    const other = kept.put("k", { session: session("b"), data: {} });
    kept.clear();
    expect(kept.keys()).toEqual([]);
    expect(record.session.disposed).toBe(true);
    expect(other.session.disposed).toBe(true);
  });

  it("pins a busy continuation when newer children finish, then makes it eligible again", () => {
    const kept = registry(2);
    const active = kept.put("active", { session: session("active"), data: {} });
    kept.put("old", { session: session("old"), data: {} });
    kept.begin(active, "active");
    kept.put("new-1", { session: session("new-1"), data: {} });
    kept.put("new-2", { session: session("new-2"), data: {} });
    expect(kept.get("active")).toBe(active);
    expect(active.session.disposed).toBe(false);
    expect(kept.get("new-1")).toBeUndefined();
    kept.end(active);
    kept.put("new-3", { session: session("new-3"), data: {} });
    kept.put("new-4", { session: session("new-4"), data: {} });
    expect(kept.get("active")).toBeUndefined();
    expect(active.session.disposed).toBe(true);
    expect(kept.records()).toHaveLength(2);
  });
});
