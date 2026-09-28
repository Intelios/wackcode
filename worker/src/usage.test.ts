import { describe, expect, it } from "vitest";
import { instrumentUsage, setUsagePublisher, trackSession, withUsage, type UsageRecord } from "./usage.js";
import type { ModelRuntime } from "./model-runtime.js";

const model = { provider: "provider", id: "model" };
function runtime() {
  const pending: Array<(value: unknown) => void> = [];
  const stream = () => {
    const result = new Promise((resolve) => pending.push(resolve));
    return { result: () => result, [Symbol.asyncIterator]() { throw new Error("Do not consume the caller's stream"); } };
  };
  const runtime = { stream, streamSimple: stream, streamDeferred: stream,
    completeSimple(...args: unknown[]) { return (this.streamSimple as (...args: unknown[]) => ReturnType<typeof stream>)(...args).result(); }
  } as unknown as ModelRuntime;
  instrumentUsage(runtime);
  return { runtime, pending };
}
const result = (stopReason = "stop") => ({ stopReason, usage: { input: 10, output: 5, cacheRead: 4, cacheWrite: 3 }, content: [{ type: "text", text: "private text" }] });

describe("usage capture", () => {
  it("isolates overlapping scopes and counts completeSimple once without reading content", async () => {
    const records: UsageRecord[] = []; setUsagePublisher((r) => records.push(r));
    const { runtime: r, pending } = runtime();
    const a = withUsage("title", () => r.completeSimple(model as never, { messages: [] }));
    const b = withUsage("subagent", () => r.completeSimple(model as never, { messages: [] }), "child");
    pending[1](result("aborted")); pending[0](result()); await Promise.all([a, b]);
    expect(records.map((r) => r.purpose)).toEqual(["subagent", "title"]);
    expect(records[0].subagent_id).toBe("child"); expect(records[1].subagent_id).toBeUndefined();
    expect(records[0].outcome).toBe("cancelled");
    expect(records[0].tokens).toEqual({ input: 10, output: 5, cache_read: 4, cache_write: 3 });
    expect(records[0].id).not.toBe(records[1].id);
    expect(JSON.stringify(records)).not.toContain("private text");
  });
  it("records failed requests and never manufactures missing usage", async () => {
    const records: UsageRecord[] = []; setUsagePublisher((r) => records.push(r));
    const { runtime: r, pending } = runtime();
    const a = r.completeSimple(model as never, { messages: [] });
    pending[0]({ stopReason: "error", usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } }); await a;
    expect(records[0]).toMatchObject({ outcome: "failed", tokens: null });
  });
  it("uses session lifecycle for compaction, branch summaries, and child identity", async () => {
    const records: UsageRecord[] = []; setUsagePublisher((r) => records.push(r));
    const { runtime: r, pending } = runtime();
    let onEvent = (_event: { type: string }) => {};
    const session = { isCompacting: false, agent: { streamFunction: r.streamSimple.bind(r) }, subscribe: (callback: typeof onEvent) => { onEvent = callback; } };
    trackSession(session as never, "child");
    onEvent({ type: "compaction_start" });
    const a = session.agent.streamFunction(model as never, { messages: [] }); pending[0](result()); await a.result();
    onEvent({ type: "compaction_end" }); session.isCompacting = true;
    const b = session.agent.streamFunction(model as never, { messages: [] }); pending[1](result()); await b.result();
    session.isCompacting = false;
    const c = session.agent.streamFunction(model as never, { messages: [] }); pending[2](result()); await c.result();
    expect(records.map((r) => r.purpose)).toEqual(["compaction", "branch_summary", "subagent"]);
    expect(records.every((r) => r.subagent_id === "child")).toBe(true);
  });
});
