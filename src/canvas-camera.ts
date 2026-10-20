/**
 * The canvas camera: a screen point is `world * zoom + (x, y)`. Pure, so panning, zooming about
 * the pointer and fitting stay testable without a DOM.
 */
import type { Bounds } from "./canvas-layout";

export interface Camera {
  x: number;
  y: number;
  zoom: number;
}

export interface Size {
  width: number;
  height: number;
}

export const MIN_ZOOM = 0.05;
export const MAX_ZOOM = 4;
/** One ⌘+/⌘− step, and the zoom pill's buttons. */
export const ZOOM_STEP = 1.25;
const FIT_PADDING = 48;

export const clampZoom = (zoom: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom));

export function panBy(camera: Camera, dx: number, dy: number): Camera {
  return { ...camera, x: camera.x + dx, y: camera.y + dy };
}

/** Zoom by `factor`, keeping the world point under `point` (screen pixels) where it is. */
export function zoomAt(camera: Camera, factor: number, point: { x: number; y: number }): Camera {
  const zoom = clampZoom(camera.zoom * factor);
  const scale = zoom / camera.zoom;
  return { zoom, x: point.x - (point.x - camera.x) * scale, y: point.y - (point.y - camera.y) * scale };
}

/** Zoom to an exact level about the viewport's centre. */
export function zoomTo(camera: Camera, zoom: number, viewport: Size): Camera {
  return zoomAt(camera, clampZoom(zoom) / camera.zoom, { x: viewport.width / 2, y: viewport.height / 2 });
}

/** The camera that shows `bounds` whole and centred, never zoomed in past `maxZoom`. */
export function fitBounds(bounds: Bounds, viewport: Size, maxZoom = 1, padding = FIT_PADDING): Camera {
  const width = Math.max(1, viewport.width - padding * 2);
  const height = Math.max(1, viewport.height - padding * 2);
  const zoom = clampZoom(Math.min(maxZoom, width / Math.max(1, bounds.width), height / Math.max(1, bounds.height)));
  return {
    zoom,
    x: viewport.width / 2 - (bounds.x + bounds.width / 2) * zoom,
    y: viewport.height / 2 - (bounds.y + bounds.height / 2) * zoom,
  };
}

export function screenToWorld(camera: Camera, point: { x: number; y: number }): { x: number; y: number } {
  return { x: (point.x - camera.x) / camera.zoom, y: (point.y - camera.y) / camera.zoom };
}

/** Whether any of `bounds` is on screen. */
export function intersects(camera: Camera, bounds: Bounds, viewport: Size): boolean {
  const left = bounds.x * camera.zoom + camera.x;
  const top = bounds.y * camera.zoom + camera.y;
  return left < viewport.width && top < viewport.height
    && left + bounds.width * camera.zoom > 0 && top + bounds.height * camera.zoom > 0;
}

/** Restores a saved camera, or undefined when the saved value is not one. */
export function parseCamera(value: unknown): Camera | undefined {
  if (!value || typeof value !== "object") return undefined;
  const { x, y, zoom } = value as Record<string, unknown>;
  if (typeof x !== "number" || typeof y !== "number" || typeof zoom !== "number") return undefined;
  if (![x, y, zoom].every(Number.isFinite) || zoom <= 0) return undefined;
  return { x, y, zoom: clampZoom(zoom) };
}

/** `wackcode:canvas:<task>:<canvas>`: a per-viewer convenience, like the side panel's width. */
export function cameraStorageKey(taskId: string, canvasId: string): string {
  return `wackcode:canvas:${taskId}:${canvasId}`;
}
