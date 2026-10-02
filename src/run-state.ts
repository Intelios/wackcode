import type { RunEvent, RunInfo } from "./types";

export interface RunRegistry {
  sessions: Record<string, RunInfo>;
  /** Keep a retirement watermark so a late exit/query cannot resurrect a removed run. */
  retired: Record<string, number>;
}
export const EMPTY_RUN_REGISTRY: RunRegistry = { sessions: {}, retired: {} };

/** Events and invoke replies can cross. A replacement or newer state always wins. */
export function applyRunEvent(current: RunRegistry, event: RunEvent): RunRegistry {
  if (event.type === "removed") {
    const retired = Math.max(current.retired[event.cwd] ?? 0, event.generation);
    const previous = current.sessions[event.cwd];
    if (retired === current.retired[event.cwd] && (!previous || previous.generation > retired)) return current;
    const sessions = { ...current.sessions };
    if (previous && previous.generation <= retired) delete sessions[event.cwd];
    return { sessions, retired: { ...current.retired, [event.cwd]: retired } };
  }
  const run = event.run;
  if (run.generation <= (current.retired[run.cwd] ?? 0)) return current;
  const previous = current.sessions[run.cwd];
  if (previous && (previous.generation > run.generation
    || (previous.generation === run.generation && previous.revision >= run.revision))) return current;
  return { ...current, sessions: { ...current.sessions, [run.cwd]: run } };
}

export function runIsActive(run?: RunInfo): boolean {
  return run?.status === "running" || run?.status === "stopping";
}

export function runStatusLabel(run: RunInfo): string {
  switch (run.status) {
    case "running": return "Running";
    case "stopping": return "Stopping…";
    case "finished": return "Finished";
    case "stopped": return "Stopped";
    case "failed": return run.exit?.signal ? "Failed (" + run.exit.signal + ")" : "Failed (code " + (run.exit?.code ?? -1) + ")";
  }
}
