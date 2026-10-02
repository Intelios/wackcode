import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Terminal } from "@xterm/xterm";
import { useReducedMotion } from "motion/react";
import { api } from "../api";
import { DEFAULT_APPEARANCE } from "../theme";
import type { RunFrame, RunInfo } from "../types";
import { RunPanel } from "./RunPanel";

vi.mock("motion/react", () => ({ useReducedMotion: vi.fn(() => false) }));
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    static instances: unknown[] = [];
    cols = 80; rows = 24;
    options: Record<string, unknown>;
    dataHandler?: (data: string) => void;
    open = vi.fn(); loadAddon = vi.fn(); attachCustomKeyEventHandler = vi.fn();
    onData = vi.fn((handler: (data: string) => void) => { this.dataHandler = handler; return { dispose: vi.fn() }; });
    write = vi.fn(); clear = vi.fn(); focus = vi.fn(); dispose = vi.fn();
    constructor(options: Record<string, unknown>) { this.options = options; (Terminal as unknown as { instances: unknown[] }).instances.push(this); }
  }
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit = vi.fn(); } }));
vi.mock("../api", () => ({ api: { attachRun: vi.fn(), detachRun: vi.fn(async () => {}), resizeRun: vi.fn(async () => {}), writeRun: vi.fn(async () => {}) } }));

const run: RunInfo = { sessionId: "one", generation: 1, revision: 1, cwd: "/tree", command: "pnpm dev", status: "running", exit: null };
type MockTerm = { write: ReturnType<typeof vi.fn>; clear: ReturnType<typeof vi.fn>; dispose: ReturnType<typeof vi.fn>; dataHandler?: (data: string) => void; options: Record<string, unknown> };
const instances = () => (Terminal as unknown as { instances: MockTerm[] }).instances;
let frame: (frame: RunFrame) => void;
let attachment: string;
const actions = () => ({ appearance: DEFAULT_APPEARANCE, configured: true, onRun: vi.fn(), onStop: vi.fn(), onClose: vi.fn() });

afterEach(cleanup);
beforeEach(() => {
  instances().length = 0;
  vi.clearAllMocks();
  vi.mocked(useReducedMotion).mockReturnValue(false);
  vi.mocked(api.attachRun).mockImplementation((_session, id, _cols, _rows, handler) => {
    attachment = id; frame = handler; return Promise.resolve(run);
  });
});

describe("Run output panel", () => {
  it("keeps the cursor still with reduced motion while accepting input", async () => {
    vi.mocked(useReducedMotion).mockReturnValue(true);
    render(<RunPanel {...actions()} run={run} />);
    await waitFor(() => expect(instances()[0].options.disableStdin).toBe(false));
    expect(instances()[0].options.cursorBlink).toBe(false);
  });

  it("restores an empty panel without starting a command", () => {
    const props = actions();
    render(<RunPanel {...props} />);
    expect(screen.getByText("No command running")).toBeInTheDocument();
    expect(api.attachRun).not.toHaveBeenCalled();
    expect(props.onRun).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    expect(props.onRun).toHaveBeenCalledOnce();
  });

  it("streams only its attachment, sends input, and detaches without stopping", async () => {
    const props = actions();
    const view = render(<RunPanel {...props} run={run} />);
    await waitFor(() => expect(instances()[0].options.disableStdin).toBe(false));
    act(() => {
      frame({ sessionId: "old", attachmentId: attachment, data: "stale" });
      frame({ sessionId: run.sessionId, attachmentId: "old", data: "stale" });
      frame({ sessionId: run.sessionId, attachmentId: attachment, data: "ready" });
      instances()[0].dataHandler?.("hello\n");
    });
    expect(instances()[0].write).toHaveBeenCalledExactlyOnceWith("ready");
    expect(api.writeRun).toHaveBeenCalledWith("one", "hello\n");
    fireEvent.click(screen.getByRole("button", { name: "Clear Run terminal" }));
    expect(instances()[0].clear).toHaveBeenCalledOnce();
    view.unmount();
    expect(api.detachRun).toHaveBeenCalledWith("one", attachment);
    expect(props.onStop).not.toHaveBeenCalled();
  });

  it("keeps the screen on completion and disables input without reattaching", async () => {
    const props = actions();
    const view = render(<RunPanel {...props} run={run} />);
    await waitFor(() => expect(api.attachRun).toHaveBeenCalledOnce());
    view.rerender(<RunPanel {...props} run={{ ...run, status: "failed", exit: { code: 7, signal: null } }} />);
    expect(screen.getByRole("status")).toHaveTextContent("Failed (code 7)");
    expect(instances()[0].options.disableStdin).toBe(true);
    expect(api.attachRun).toHaveBeenCalledOnce();
    expect(instances()[0].dispose).not.toHaveBeenCalled();
  });

  it("replaces the terminal and enables input when switching between live checkout runs", async () => {
    const props = actions();
    const view = render(<RunPanel {...props} run={run} />);
    await waitFor(() => expect(instances()[0].options.disableStdin).toBe(false));
    view.rerender(<RunPanel {...props} run={{ ...run, sessionId: "two", generation: 2, cwd: "/other" }} />);
    await waitFor(() => expect(instances()[1].options.disableStdin).toBe(false));
    expect(instances()[0].dispose).toHaveBeenCalledOnce();
    expect(api.detachRun).toHaveBeenCalledWith("one", expect.any(String));
    expect(api.attachRun).toHaveBeenCalledTimes(2);
  });
});
