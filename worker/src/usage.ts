import { AsyncLocalStorage } from "node:async_hooks";
import { randomUUID } from "node:crypto";
import type { ModelRuntime } from "./model-runtime.js";

export type UsagePurpose = "chat" | "subagent" | "title" | "compaction" | "branch_summary" | "goal_verification" | "commit_message";
export interface UsageRecord {
  v: 1;
  id: string;
  ts: number;
  duration_ms: number;
  provider: string;
  model: string;
  purpose: UsagePurpose;
  subagent_id?: string;
  outcome: "completed" | "failed" | "cancelled";
  tokens: { input: number; output: number; cache_read: number; cache_write: number } | null;
}
type Context = { purpose: UsagePurpose; subagent_id?: string };
const context = new AsyncLocalStorage<Context>();
let publish: (record: UsageRecord) => void = () => {};
const instrumented = new WeakSet<ModelRuntime>();
export function setUsagePublisher(callback: typeof publish): void { publish = callback; }
export function withUsage<T>(purpose: UsagePurpose, run: () => T, subagent_id?: string): T {
  return context.run({ purpose, subagent_id: subagent_id ?? context.getStore()?.subagent_id }, run);
}

/** Observe result promises, never iterate the stream (which belongs to Pi). Completion
 * methods delegate to these stream methods, so instrumenting both would count twice. */
export function instrumentUsage(runtime: ModelRuntime): void {
  if (instrumented.has(runtime)) return;
  instrumented.add(runtime);
  for (const key of ["stream", "streamSimple", "streamDeferred"] as const) {
    const original = runtime[key].bind(runtime);
    const wrapped = (...args: Parameters<typeof original>) => {
      const started = Date.now();
      const id = randomUUID();
      const scope = { purpose: "chat" as UsagePurpose, ...context.getStore() };
      const model = args[0];
      const stream = (original as (...values: typeof args) => ReturnType<typeof original>)(...args);
      void stream.result().then((result) => {
        const u = result.usage;
        const count = (n: unknown) => typeof n === "number" && Number.isSafeInteger(n) && n >= 0;
        const known = u && [u.input, u.output, u.cacheRead, u.cacheWrite].every(count)
          && u.input + u.output + u.cacheRead + u.cacheWrite > 0;
        publish({ v: 1, id, ts: Date.now(), duration_ms: Math.max(0, Date.now() - started),
          provider: model.provider, model: model.id, ...scope,
          outcome: result.stopReason === "aborted" ? "cancelled" : result.stopReason === "error" ? "failed" : "completed",
          tokens: known ? { input: u.input, output: u.output, cache_read: u.cacheRead, cache_write: u.cacheWrite } : null });
      }, () => {
        publish({ v: 1, id, ts: Date.now(), duration_ms: Math.max(0, Date.now() - started),
          provider: model.provider, model: model.id, ...scope, outcome: "failed", tokens: null });
      }).catch(() => {});
      return stream;
    };
    Object.defineProperty(runtime, key, { value: wrapped, configurable: true });
  }
}

/** Each session has its own scope, even when children share their parent's runtime. */
export function trackSession(session: Awaited<ReturnType<typeof import("@earendil-works/pi-coding-agent")["createAgentSession"]>>["session"], subagentId?: string): void {
  let purpose: UsagePurpose = subagentId ? "subagent" : "chat";
  const original = session.agent.streamFunction;
  session.agent.streamFunction = (...args) => withUsage(
    purpose === "compaction" ? purpose : session.isCompacting ? "branch_summary" : purpose,
    () => original(...args), subagentId);
  session.subscribe((event) => {
    if (event.type === "compaction_start") purpose = "compaction";
    if (event.type === "compaction_end") purpose = subagentId ? "subagent" : "chat";
  });
}
