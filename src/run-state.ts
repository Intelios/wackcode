import type { ProjectCommandKind, RunEvent, RunInfo } from "./types";

export interface RunRegistry {
  sessions: Record<string, RunInfo>;
  /** Keep a retirement watermark so a late exit/query cannot resurrect a removed run. */
  retired: Record<string, number>;
}
export const EMPTY_RUN_REGISTRY: RunRegistry = { sessions: {}, retired: {} };

/** Both actions share checkout identity, but never replace or retire each other's session. */
export function runKey(cwd: string, kind: ProjectCommandKind): string {
  return `${kind}:${cwd}`;
}

/** Events and invoke replies can cross. A replacement or newer state always wins. */
export function applyRunEvent(current: RunRegistry, event: RunEvent): RunRegistry {
  if (event.type === "removed") {
    const key = runKey(event.cwd, event.kind);
    const retired = Math.max(current.retired[key] ?? 0, event.generation);
    const previous = current.sessions[key];
    if (retired === current.retired[key] && (!previous || previous.generation > retired)) return current;
    const sessions = { ...current.sessions };
    if (previous && previous.generation <= retired) delete sessions[key];
    return { sessions, retired: { ...current.retired, [key]: retired } };
  }
  const run = event.run;
  const key = runKey(run.cwd, run.kind);
  if (run.generation <= (current.retired[key] ?? 0)) return current;
  const previous = current.sessions[key];
  if (previous && (previous.generation > run.generation
    || (previous.generation === run.generation && previous.revision >= run.revision))) return current;
  return { ...current, sessions: { ...current.sessions, [key]: run } };
}

export function runIsActive(run?: RunInfo): boolean {
  return run?.status === "running" || run?.status === "stopping";
}

export function runStatusLabel(run: RunInfo): string {
  switch (run.status) {
    case "running": return run.kind === "build" ? "Building" : "Running";
    case "stopping": return "Stopping…";
    case "finished": return "Finished";
    case "stopped": return "Stopped";
    case "failed": return run.exit?.signal ? "Failed (" + run.exit.signal + ")" : "Failed (code " + (run.exit?.code ?? -1) + ")";
  }
}
