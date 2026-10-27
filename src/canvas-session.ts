/**
 * A chat's Canvas viewers outlive their transcript positions. Moving a board to a later turn
 * or expanding it replaces its host, so camera, selection and measurements belong here rather
 * than to that host. Only camera and height are local conveniences; no chat metadata changes.
 */
import { useSyncExternalStore } from "react";
import { cameraStorageKey, parseCamera, type Camera, type Size } from "./canvas-camera";

export const MIN_CANVAS_HEIGHT = 240;
export const DEFAULT_CANVAS_HEIGHT = 420;

export interface CanvasViewState {
  camera: Camera;
  viewport?: Size;
  arrivals: string[];
  fitted: boolean;
  selected?: string;
  entered?: string;
  measured: Readonly<Record<string, number>>;
  following: boolean;
  active: boolean;
  expanded: boolean;
  height: number;
  signatures?: Readonly<Record<string, string>>;
  /** Navigation requests are consumed once, even when a viewer changes hosts. */
  focusNonce?: number;
}

export interface CanvasSession {
  subscribe: (listener: () => void) => () => void;
  /** Only interaction/expansion locks change this snapshot; camera frames do not move turns. */
  getVersion: () => number;
  ensure: (taskId: string, canvasId: string) => CanvasViewState;
  get: (canvasId: string) => CanvasViewState | undefined;
  update: (canvasId: string, patch: Partial<CanvasViewState>) => void;
  activate: (canvasId: string) => void;
  deactivate: () => void;
  setExpanded: (canvasId?: string) => void;
  isLocked: (canvasId: string) => boolean;
}

export function canvasHeightStorageKey(taskId: string, canvasId: string): string {
  return `${cameraStorageKey(taskId, canvasId)}:height`;
}

export function createCanvasSession(): CanvasSession {
  const views = new Map<string, CanvasViewState>();
  const tasks = new Map<string, string>();
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  const listeners = new Set<() => void>();
  let version = 0;
  let expandedId: string | undefined;
  const emit = (locksChanged: boolean) => { if (locksChanged) version += 1; for (const listener of listeners) listener(); };
  const update = (canvasId: string, patch: Partial<CanvasViewState>) => {
    const previous = views.get(canvasId);
    if (!previous || Object.entries(patch).every(([key, value]) => previous[key as keyof CanvasViewState] === value)) return;
    views.set(canvasId, { ...previous, ...patch });
    const taskId = tasks.get(canvasId);
    if (taskId && patch.camera) {
      clearTimeout(timers.get(canvasId));
      // This timer belongs to the session, so moving/unmounting a host cannot lose its last pan.
      const camera = patch.camera;
      timers.set(canvasId, setTimeout(() => {
        try { localStorage.setItem(cameraStorageKey(taskId, canvasId), JSON.stringify(camera)); } catch { /* convenience only */ }
        timers.delete(canvasId);
      }, 300));
    }
    if (taskId && patch.height !== undefined) {
      try { localStorage.setItem(canvasHeightStorageKey(taskId, canvasId), String(patch.height)); } catch { /* convenience only */ }
    }
    emit((patch.active !== undefined && patch.active !== previous.active) || (patch.expanded !== undefined && patch.expanded !== previous.expanded));
  };
  const deactivate = () => {
    for (const [id, view] of views) if (view.active || view.entered) update(id, { active: false, entered: undefined });
  };
  return {
    subscribe: (listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    getVersion: () => version,
    ensure: (taskId, canvasId) => {
      let view = views.get(canvasId);
      if (!view) {
        let camera: Camera | undefined;
        let height = DEFAULT_CANVAS_HEIGHT;
        try {
          const raw = localStorage.getItem(cameraStorageKey(taskId, canvasId));
          camera = raw ? parseCamera(JSON.parse(raw)) : undefined;
          const savedHeight = Number(localStorage.getItem(canvasHeightStorageKey(taskId, canvasId)));
          if (Number.isFinite(savedHeight) && savedHeight >= MIN_CANVAS_HEIGHT) height = Math.min(1200, savedHeight);
        } catch { /* malformed or unavailable storage is harmless */ }
        view = { camera: camera ?? { x: 0, y: 0, zoom: 1 }, fitted: camera !== undefined, measured: {}, arrivals: [], following: true, active: false, expanded: canvasId === expandedId, height };
        tasks.set(canvasId, taskId);
        views.set(canvasId, view);
      }
      return view;
    },
    get: (canvasId) => views.get(canvasId),
    update,
    activate: (canvasId) => {
      for (const [id, view] of views) if (id !== canvasId && (view.active || view.entered)) update(id, { active: false, entered: undefined });
      update(canvasId, { active: true });
    },
    deactivate,
    setExpanded: (canvasId) => {
      const changed = expandedId !== canvasId;
      expandedId = canvasId;
      const before = version;
      for (const [id, view] of views) {
        if (view.expanded !== (id === canvasId) || view.active || view.entered) update(id, { expanded: id === canvasId, active: false, entered: undefined });
      }
      if (changed && before === version) emit(true);
    },
    isLocked: (canvasId) => {
      const view = views.get(canvasId);
      return canvasId === expandedId || Boolean(view?.active);
    },
  };
}

export function useCanvasView(session: CanvasSession, taskId: string, canvasId: string): CanvasViewState {
  session.ensure(taskId, canvasId);
  return useSyncExternalStore(session.subscribe, () => session.get(canvasId)!);
}
