/**
 * A chat's Canvas viewers outlive their side-panel page. Switching tabs, swapping canvases or
 * hiding the panel unmounts the board, so camera, selection, tool and measurements belong here
 * rather than to that host. Only the camera is a local convenience; no chat metadata changes.
 */
import { useSyncExternalStore } from "react";
import { cameraStorageKey, parseCamera, type Camera, type Size } from "./canvas-camera";

/** Select clicks pick artboards; Hand only pans, even over a stepped-into artboard. */
export type CanvasTool = "select" | "hand";

export interface CanvasViewState {
  camera: Camera;
  viewport?: Size;
  arrivals: string[];
  fitted: boolean;
  selected?: string;
  entered?: string;
  measured: Readonly<Record<string, number>>;
  following: boolean;
  tool: CanvasTool;
  signatures?: Readonly<Record<string, string>>;
  /** Navigation requests are consumed once, even when the board remounts. */
  focusNonce?: number;
}

export interface CanvasSession {
  subscribe: (listener: () => void) => () => void;
  ensure: (taskId: string, canvasId: string) => CanvasViewState;
  get: (canvasId: string) => CanvasViewState | undefined;
  update: (canvasId: string, patch: Partial<CanvasViewState>) => void;
}

export function createCanvasSession(): CanvasSession {
  const views = new Map<string, CanvasViewState>();
  const tasks = new Map<string, string>();
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  const listeners = new Set<() => void>();
  const update = (canvasId: string, patch: Partial<CanvasViewState>) => {
    const previous = views.get(canvasId);
    if (!previous || Object.entries(patch).every(([key, value]) => previous[key as keyof CanvasViewState] === value)) return;
    views.set(canvasId, { ...previous, ...patch });
    const taskId = tasks.get(canvasId);
    if (taskId && patch.camera) {
      clearTimeout(timers.get(canvasId));
      // This timer belongs to the session, so unmounting the board cannot lose its last pan.
      const camera = patch.camera;
      timers.set(canvasId, setTimeout(() => {
        try { localStorage.setItem(cameraStorageKey(taskId, canvasId), JSON.stringify(camera)); } catch { /* convenience only */ }
        timers.delete(canvasId);
      }, 300));
    }
    for (const listener of listeners) listener();
  };
  return {
    subscribe: (listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    ensure: (taskId, canvasId) => {
      let view = views.get(canvasId);
      if (!view) {
        let camera: Camera | undefined;
        try {
          const raw = localStorage.getItem(cameraStorageKey(taskId, canvasId));
          camera = raw ? parseCamera(JSON.parse(raw)) : undefined;
        } catch { /* malformed or unavailable storage is harmless */ }
        view = { camera: camera ?? { x: 0, y: 0, zoom: 1 }, fitted: camera !== undefined, measured: {}, arrivals: [], following: true, tool: "select" };
        tasks.set(canvasId, taskId);
        views.set(canvasId, view);
      }
      return view;
    },
    get: (canvasId) => views.get(canvasId),
    update,
  };
}

export function useCanvasView(session: CanvasSession, taskId: string, canvasId: string): CanvasViewState {
  session.ensure(taskId, canvasId);
  return useSyncExternalStore(session.subscribe, () => session.get(canvasId)!);
}
