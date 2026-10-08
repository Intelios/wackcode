import { afterEach, describe, expect, it, vi } from "vitest";
import type { ToolDefinition } from "@earendil-works/pi-coding-agent";
import { createSubagentsExtension } from "./builtin/subagents/index.js";
import { emptyUsage, MAX_CARD_OUTPUT } from "./builtin/subagents/details.js";
import { backgroundCalls, interruptedDetails, projectBackgroundCards } from "./builtin/subagents/state.js";
import type { BuiltinHost, SubagentOutcome, SubagentRunRequest } from "./builtin/host.js";
import type { NormalizedMessage, SubagentDetails } from "./protocol.js";

const cleanup: Array<() => Promise<void>> = [];
afterEach(async () => { for (const close of cleanup.splice(0)) await close(); });
const text = (result: any) => result.content.map((part: { text: string }) => part.text).join("\n");

async function harness(limit = 2) {
  const runs: Array<{ request: SubagentRunRequest; finish: (status?: SubagentOutcome["status"], error?: string) => void }> = [];
  const entries: unknown[] = [];
  const tools = new Map<string, ToolDefinition>();
  const usage = vi.fn();
  const host = {
    childToolNames: () => ["read", "write"], redact: (value: string) => value.replaceAll("secret", "[redacted]"),
    recordSubagentUsage: usage,
    runSubagent: (request: SubagentRunRequest) => new Promise<SubagentOutcome>((resolve) => {
      const finish = (status: SubagentOutcome["status"] = "done", error?: string) => {
        request.signal?.removeEventListener("abort", abort);
        const spent = emptyUsage(); spent.input = 10; spent.totalTokens = 10;
        resolve({ status, output: `answer secret ${request.task}`, error, usage: spent, turns: 1 });
      };
      const abort = () => finish("aborted");
      runs.push({ request, finish });
      request.signal?.addEventListener("abort", abort, { once: true });
      request.observer.started("Fixture model");
      if (request.signal?.aborted) abort();
    }),
  } as unknown as BuiltinHost;
  const extension = createSubagentsExtension(host, () => "build", { name: "fixture", factory: () => {} });
  extension.factory({ registerTool: (tool: ToolDefinition) => tools.set(tool.name, tool), on: () => () => {}, appendEntry: (customType: string, data: unknown) => entries.push({ type: "custom", customType, data }) } as never);
  await extension.controller.configure({ trigger: "on_request", maxConcurrency: limit, agents: [{ name: "worker", description: "Edits", prompt: "Instructions", tools: ["read", "write"], readOnly: false }], providers: [] });
  cleanup.push(extension.controller.stopAll);
  let calls = 0;
  const call = (name: string, params: unknown, signal?: AbortSignal, onUpdate?: (value: any) => void) => tools.get(name)!.execute(`launch-${++calls}`, params, signal, onUpdate, {} as never) as Promise<any>;
  return { ...extension, runs, entries, usage,
    launch: (params: unknown, signal?: AbortSignal, onUpdate?: (value: any) => void) => call("subagent", params, signal, onUpdate),
    manage: (params: unknown, signal?: AbortSignal) => call("subagent_job", params, signal),
  };
}

const batch = (count: number, background = true) => ({ background, tasks: Array.from({ length: count }, (_, i) => ({ agent: "worker", task: `task-${i}` })) });

describe("background sub-agent ownership", () => {
  it("returns IDs immediately, retains children after parent settlement and delivers redacted results once", async () => {
    const h = await harness();
    const parent = new AbortController();
    const update = vi.fn();
    const launched = await h.launch(batch(2), parent.signal, update);
    const ids = launched.details.results.map((result: any) => result.jobId);
    expect(new Set(ids).size).toBe(2);
    expect(h.controller.hasWork()).toBe(true);
    parent.abort(); // The launch has returned: this signal no longer owns the children.
    await vi.waitFor(() => expect(h.runs).toHaveLength(2));
    expect(h.runs.every((run) => !run.request.signal?.aborted)).toBe(true);
    h.runs[0].finish("failed", `secret ${"x".repeat(30_000)}`); h.runs[1].finish();
    await vi.waitFor(() => expect(h.controller.pendingCount()).toBe(2));
    expect(text(await h.manage({ action: "status", jobIds: ids }))).toContain("[redacted]");
    expect(h.controller.pendingCount()).toBe(2);
    expect(h.controller.calls()[0].details.results[0].error!.length).toBeLessThan(MAX_CARD_OUTPUT + 100);
    expect(h.controller.calls()[0].details.results[0].error).not.toContain("secret");
    expect(h.controller.takeResults()).toMatchObject({ jobIds: ids });
    expect(h.controller.takeResults()).toBeUndefined();
    expect(h.controller.hasWork()).toBe(false);
    expect(h.usage).toHaveBeenCalledTimes(2);
    expect(update).not.toHaveBeenCalled();
    expect(JSON.stringify(h.entries)).not.toContain("answer secret");
  });

  it("restores a failed delivery's claim so the results are re-delivered instead of lost", async () => {
    const h = await harness();
    await h.launch(batch(2));
    await vi.waitFor(() => expect(h.runs).toHaveLength(2));
    h.runs[0].finish(); h.runs[1].finish();
    await vi.waitFor(() => expect(h.controller.pendingCount()).toBe(2));
    const claimed = h.controller.takeResults()!;
    expect(h.controller.pendingCount()).toBe(0);
    expect(h.controller.hasWork()).toBe(false);
    h.controller.restoreResults(claimed.jobIds);
    expect(h.controller.pendingCount()).toBe(2);
    expect(h.controller.hasWork()).toBe(true);
    expect(h.controller.takeResults()).toMatchObject({ jobIds: claimed.jobIds });
    expect(h.controller.takeResults()).toBeUndefined();
    h.controller.restoreResults(["foreign"]);
    expect(h.controller.takeResults()).toBeUndefined();
  });

  it("waits for all selected outcomes and prevents automatic delivery from consuming them", async () => {
    const h = await harness();
    const result = await h.launch(batch(2));
    const ids = result.details.results.map((child: any) => child.jobId);
    let returned = false;
    const wait = h.manage({ action: "wait", jobIds: ids }).then((value) => { returned = true; return value; });
    await vi.waitFor(() => expect(h.runs).toHaveLength(2));
    h.runs[0].finish();
    await vi.waitFor(() => expect(h.controller.pendingCount()).toBe(1));
    expect(returned).toBe(false);
    expect(h.controller.takeResults()).toBeUndefined();
    h.runs[1].finish("failed");
    expect(text(await wait)).toContain("failed");
    expect(h.controller.takeResults()).toBeUndefined();
    expect(text(await h.manage({ action: "wait", jobIds: ids }))).toContain("task-0");
    expect(h.usage).toHaveBeenCalledTimes(2);
  });

  it("shares its concurrency/admission budget with blocking calls, including cancellation of queued children", async () => {
    const h = await harness(2);
    await h.launch(batch(3));
    const foreground = h.launch(batch(2, false));
    await vi.waitFor(() => expect(h.runs).toHaveLength(2));
    expect(h.controller.activeCount()).toBe(5);
    await expect(h.launch(batch(4))).rejects.toThrow("At most 8");
    await h.controller.stopAll();
    await foreground;
    expect(h.runs).toHaveLength(2);
    expect(h.runs.every((run) => run.request.signal?.aborted)).toBe(true);
    expect(h.controller.hasWork()).toBe(false);
  });

  it("blocking calls await children and attribute usage on their tool result only", async () => {
    const h = await harness();
    let returned = false;
    const foreground = h.launch(batch(1, false)).then((value) => { returned = true; return value; });
    await vi.waitFor(() => expect(h.runs).toHaveLength(1));
    expect(returned).toBe(false);
    h.runs[0].finish();
    expect((await foreground).usage.input).toBe(10);
    expect(h.usage).not.toHaveBeenCalled();
  });

  it("a stop keeps owning outcomes, so a late restore never re-delivers them", async () => {
    const h = await harness(1);
    await h.launch(batch(1));
    await vi.waitFor(() => expect(h.runs).toHaveLength(1));
    h.runs[0].finish();
    await vi.waitFor(() => expect(h.controller.pendingCount()).toBe(1));
    const claimed = h.controller.takeResults()!;
    await h.controller.stopAll();
    h.controller.restoreResults(claimed.jobIds);
    expect(h.controller.pendingCount()).toBe(0);
    expect(h.controller.takeResults()).toBeUndefined();
    expect(h.controller.hasWork()).toBe(false);
  });

  it("Stop/disabling and an aborted wait settle active and queued jobs without resuming", async () => {
    const h = await harness(1);
    await h.launch(batch(3));
    await vi.waitFor(() => expect(h.runs).toHaveLength(1));
    const abort = new AbortController();
    const wait = h.manage({ action: "wait" }, abort.signal);
    abort.abort();
    expect(text(await wait)).toContain("stopped");
    expect(h.runs).toHaveLength(1);
    expect(h.controller.takeResults()).toBeUndefined();
    await h.launch(batch(2));
    await h.controller.configure(null);
    expect(h.controller.hasWork()).toBe(false);
    expect(h.controller.takeResults()).toBeUndefined();
    expect(h.controller.inactiveTools()).toEqual(["subagent", "subagent_job"]);
  });

  it("omitted targets are captured once, so later launches do not prolong a wait", async () => {
    const h = await harness();
    await h.launch(batch(1));
    const waiting = h.manage({ action: "wait" });
    await h.launch(batch(1));
    await vi.waitFor(() => expect(h.runs).toHaveLength(2));
    h.runs[0].finish();
    await waiting;
    expect(h.controller.activeCount()).toBe(1);
    expect(h.runs[1].request.signal?.aborted).toBe(false);
  });

  it("stops a selected queued child immediately without waiting for unrelated running work", async () => {
    const h = await harness(1);
    const launch = await h.launch(batch(2));
    await vi.waitFor(() => expect(h.runs).toHaveLength(1));
    const stopped = await h.manage({ action: "stop", jobIds: [launch.details.results[1].jobId] });
    expect(text(stopped)).toContain("stopped");
    expect(h.controller.activeCount()).toBe(1);
    expect(h.runs[0].request.signal?.aborted).toBe(false);
  });

  it("rejects unknown IDs and invalid modes without running extra children", async () => {
    const h = await harness();
    await expect(h.manage({ action: "wait", jobIds: ["foreign"] })).rejects.toThrow("no longer available");
    await expect(h.launch({ agent: "worker", task: "task", background: "true" })).rejects.toThrow("boolean");
    expect(h.runs).toHaveLength(0);
  });
});

it("replays branch-scoped cards, interrupts unfinished cold history, and preserves unchanged row identities", () => {
  const details: SubagentDetails = { v: 1, background: true, mode: "single", results: [{ jobId: "job", agent: "worker", task: "task", readOnly: false, status: "running", activity: [], usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 0 }, transcript: { v: 1, messages: [] } }] };
  const entries = [{ type: "custom", customType: "wackcode-subagent-background", data: { v: 1, toolCallId: "launch", details } }];
  const calls = backgroundCalls(entries);
  const restored = interruptedDetails(calls.get("launch")!.details);
  expect(restored.results[0].status).toBe("interrupted");
  expect(interruptedDetails(details)).toBe(restored);
  const message: NormalizedMessage = { id: "result", role: "tool", blocks: [{ type: "tool-result", toolName: "subagent", toolCallId: "launch", details: {} }] };
  calls.set("launch", { v: 1, toolCallId: "launch", details: restored });
  const cache = new WeakMap();
  const first = projectBackgroundCards([message], calls, cache);
  expect(projectBackgroundCards([message], calls, cache)[0]).toBe(first[0]);
  expect(JSON.stringify(first)).not.toContain("transcript");
  expect(backgroundCalls([]).size).toBe(0);
  expect(message.blocks[0].details).toEqual({});
});
