import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Terminal } from "@xterm/xterm";
import { api } from "../api";
import { DEFAULT_APPEARANCE } from "../theme";
import type { TerminalFrame, TerminalInfo } from "../types";
import { TerminalPanel } from "./TerminalPanel";

// xterm measures real layout, so the mock records what the component does with it: output
// written, keystrokes sent back over `onData`, and the custom key handler that lets the app's
// own shortcuts through.
vi.mock("@xterm/xterm", () => ({
  Terminal: class MockTerminal {
    static instances: MockTerminal[] = [];
    cols = 80;
    rows = 24;
    options: Record<string, unknown> = {};
    keyHandler?: (event: { type: string; key: string; metaKey?: boolean; ctrlKey?: boolean; shiftKey?: boolean }) => boolean;
    dataHandler?: (data: string) => void;
    written: string[] = [];
    open = vi.fn();
    loadAddon = vi.fn();
    attachCustomKeyEventHandler = vi.fn((handler: MockTerminal["keyHandler"]) => { this.keyHandler = handler; });
    onData = vi.fn((handler: (data: string) => void) => { this.dataHandler = handler; return { dispose: vi.fn() }; });
    write = vi.fn((data: string) => { this.written.push(data); });
    reset = vi.fn();
    clear = vi.fn();
    focus = vi.fn();
    dispose = vi.fn();
    constructor() { MockTerminal.instances.push(this); }
  }
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit = vi.fn(); } }));

vi.mock("../api", () => ({
  api: {
    openTerminal: vi.fn(),
    writeTerminal: vi.fn(async () => {}),
    resizeTerminal: vi.fn(async () => {}),
    detachTerminal: vi.fn(async () => {}),
    restartTerminal: vi.fn(),
    closeTerminal: vi.fn(async () => {})
  }
}));

const info: TerminalInfo = { sessionId: "s1", shell: "zsh", cwd: "/tmp/work", fresh: true, exit: null, busy: false };
let onFrame: (frame: TerminalFrame) => void = () => undefined;

type KeyEvent = { type: string; key: string; metaKey?: boolean; ctrlKey?: boolean; shiftKey?: boolean };
type MockTerm = { dataHandler?: (data: string) => void; keyHandler?: (event: KeyEvent) => boolean; written: string[]; reset: unknown };

function instances(): MockTerm[] {
  return (Terminal as unknown as { instances: MockTerm[] }).instances;
}

function renderPanel() {
  return render(<TerminalPanel taskId="chat-1" appearance={DEFAULT_APPEARANCE} onClose={() => {}} />);
}

afterEach(cleanup);

beforeEach(() => {
  instances().length = 0;
  vi.clearAllMocks();
  vi.mocked(api.openTerminal).mockImplementation((_taskId, _cols, _rows, handler) => {
    onFrame = handler;
    return Promise.resolve(info);
  });
  vi.mocked(api.restartTerminal).mockImplementation((_taskId, _cols, _rows, handler) => {
    onFrame = handler;
    return Promise.resolve({ ...info, sessionId: "s2" });
  });
});

describe("TerminalPanel", () => {
  it("opens the chat's shell, streams channel output into xterm, and sends keystrokes back", async () => {
    const view = renderPanel();
    await waitFor(() => expect(api.openTerminal).toHaveBeenCalledWith("chat-1", 80, 24, expect.any(Function)));
    const term = instances()[0];

    act(() => onFrame({ type: "output", data: "hello$ " }));
    expect(term.written).toContain("hello$ ");
    act(() => term.dataHandler!("ls\n"));
    expect(api.writeTerminal).toHaveBeenCalledWith("chat-1", "ls\n");

    // Hiding the panel detaches; the shell itself keeps running.
    view.unmount();
    expect(api.detachTerminal).toHaveBeenCalledWith("chat-1");
    expect(api.closeTerminal).not.toHaveBeenCalled();
  });

  it("plays the power-on sweep only for a freshly spawned shell", async () => {
    renderPanel();
    await waitFor(() => expect(api.openTerminal).toHaveBeenCalled());
    expect(document.querySelector(".terminal-screen.power-on")).not.toBeNull();

    cleanup();
    vi.mocked(api.openTerminal).mockImplementation((_t, _c, _r, handler) => {
      onFrame = handler;
      return Promise.resolve({ ...info, fresh: false });
    });
    renderPanel();
    await waitFor(() => expect(api.openTerminal).toHaveBeenCalledTimes(2));
    expect(document.querySelector(".terminal-screen.power-on")).toBeNull();
  });

  it("shows the exit state and offers to start a new shell", async () => {
    renderPanel();
    await waitFor(() => expect(api.openTerminal).toHaveBeenCalled());
    act(() => onFrame({ type: "exit", code: 0, signal: null }));

    await screen.findByText("Shell exited");
    expect(screen.getByText("Exited")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Start a new shell" }));
    await waitFor(() => expect(api.restartTerminal).toHaveBeenCalledWith("chat-1", 80, 24, expect.any(Function)));
    expect(instances()[0].reset).toHaveBeenCalled();
  });

  it("ends an idle shell at once but confirms while a command is running", async () => {
    renderPanel();
    await waitFor(() => expect(api.openTerminal).toHaveBeenCalled());

    // Busy first: ending needs the confirm dialog.
    act(() => onFrame({ type: "busy", busy: true }));
    expect(screen.getByText("Working")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "End terminal session" }));
    await screen.findByText("End the terminal session?");
    expect(api.closeTerminal).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "End session" }));
    await waitFor(() => expect(api.closeTerminal).toHaveBeenCalledWith("chat-1"));
    await screen.findByText("Session ended");
  });

  it("ends an idle shell without asking, and offers a fresh one", async () => {
    renderPanel();
    await waitFor(() => expect(api.openTerminal).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("button", { name: "End terminal session" }));
    expect(screen.queryByText("End the terminal session?")).not.toBeInTheDocument();
    await waitFor(() => expect(api.closeTerminal).toHaveBeenCalledWith("chat-1"));

    fireEvent.click(await screen.findByRole("button", { name: "New shell" }));
    await waitFor(() => expect(api.openTerminal).toHaveBeenCalledTimes(2));
  });

  it("passes the app's own shortcuts through but keeps shell keys inside", async () => {
    renderPanel();
    await waitFor(() => expect(api.openTerminal).toHaveBeenCalled());
    const handler = instances()[0].keyHandler!;

    // These reach the app's keydown listener instead of the shell.
    for (const key of ["n", "o", ","]) {
      expect(handler({ type: "keydown", key, metaKey: true }), key).toBe(false);
    }
    expect(handler({ type: "keydown", key: "c", metaKey: true, shiftKey: true })).toBe(false);
    expect(handler({ type: "keydown", key: "t", metaKey: true, shiftKey: true })).toBe(false);
    // ⇧Tab and ordinary typing belong to the shell.
    expect(handler({ type: "keydown", key: "Tab", shiftKey: true })).toBe(true);
    expect(handler({ type: "keydown", key: "x" })).toBe(true);
    expect(handler({ type: "keyup", key: "n", metaKey: true })).toBe(true);
  });
});
