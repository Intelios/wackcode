/** Renderer-only decoration. Phase selection is shared by Code and Chat, and never examines
 * prompt contents. The selected chat alone supplies runtime; navigation never generates pulses. */
import type { TaskRuntime, TaskStatus } from "./types";

export type BackgroundPhase = "idle" | "working" | "thinking" | "answering" | "background";

export function backgroundPhase(input: {
  status?: TaskStatus;
  runtime?: TaskRuntime;
  sending?: boolean;
  waiting?: boolean;
  settings?: boolean;
}): BackgroundPhase {
  const { status, runtime, sending, waiting, settings } = input;
  if (settings || waiting || status === "stopping" || status === "error" || status === "interrupted") return "idle";
  if (sending) return "working";
  if (status !== "running") return "idle";
  if (runtime?.pendingMessage || runtime?.activity === "starting" || runtime?.compaction) return "working";
  if (runtime?.workActivity?.parent === "idle") {
    return runtime.workActivity.subagents > 0 || runtime.workActivity.pendingResults > 0 ? "background" : "idle";
  }
  const last = runtime?.partial?.blocks.at(-1);
  if (last?.type === "thinking" && last.durationMs === undefined) return "thinking";
  if (last?.type === "text") return "answering";
  return "working";
}

/** Only the exact empty element is eligible, never a descendant's text or controls. A marked
 * container inside an overlay/card is still excluded. No event is cancelled or captured. */
export function isBackgroundClick(target: EventTarget | null): target is HTMLElement {
  return target instanceof HTMLElement && target.hasAttribute("data-dynamic-space")
    && !target.closest('[data-tauri-drag-region], button, a, input, textarea, select, [contenteditable], [role="dialog"], [role="menu"], [role="button"], .composer, .chat-composer, .inline-dialog-wrap, .side-panel, .settings-block');
}

export const PHASE_MOTION: Record<BackgroundPhase, { energy: number; direction: number }> = {
  idle: { energy: 0.18, direction: 0 },
  background: { energy: 0.32, direction: 0.1 },
  working: { energy: 0.55, direction: 0.15 },
  thinking: { energy: 0.8, direction: -1 },
  answering: { energy: 0.65, direction: 1 }
};

export function glyphGrid(width: number, height: number) {
  let cell = Math.max(10, Math.sqrt(width * height / (12_000 * 1.6)));
  let columns = Math.ceil(width / cell);
  let rows = Math.ceil(height / (cell * 1.6));
  while (columns * rows > 12_000) {
    cell *= 1.02;
    columns = Math.ceil(width / cell);
    rows = Math.ceil(height / (cell * 1.6));
  }
  return { cell, columns, rows };
}
