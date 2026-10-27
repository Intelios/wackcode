import type { CanvasTouch } from "./canvas-state";

/** Canvas placement is presentation-only: the visible branch's last accepted touch owns the
 * live board. Interaction may hold an earlier surviving occurrence until the user is done. */
export interface CanvasTurn {
  userKey: string;
  canvases: readonly CanvasTouch[];
}

export function canvasOwners(
  turns: Iterable<CanvasTurn>,
  previous: ReadonlyMap<string, string> = new Map(),
  locked: (canvasId: string) => boolean = () => false,
): Map<string, string> {
  const latest = new Map<string, string>();
  const occurrences = new Map<string, Set<string>>();
  for (const turn of turns) {
    for (const touch of turn.canvases) {
      latest.set(touch.canvas, turn.userKey);
      const owners = occurrences.get(touch.canvas) ?? new Set<string>();
      owners.add(turn.userKey);
      occurrences.set(touch.canvas, owners);
    }
  }
  for (const [canvasId, owner] of latest) {
    const before = previous.get(canvasId);
    if (before && before !== owner && locked(canvasId) && occurrences.get(canvasId)?.has(before)) latest.set(canvasId, before);
  }
  return latest;
}

/** Occurrences keep their identity when a live board becomes a reference. A canvas-global
 * anchor would make reading restoration chase the board to its newer turn. */
export function canvasAnchor(userKey: string, canvasId: string): string {
  return `canvas:${JSON.stringify([userKey, canvasId])}`;
}
