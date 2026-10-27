import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

// jsdom has no canvas and logs "Not implemented" for every getContext call. Components treat
// a null context as "no drawing" (CompactingStage's gravity well), so stub it once here.
vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);

// The app's default window (tauri.conf.json): wide enough for the sidebar, the chat column and a
// side panel together, so `solveShell` keeps the sidebar on screen unless a test narrows it.
Object.defineProperty(window, "innerWidth", { configurable: true, writable: true, value: 1440 });
