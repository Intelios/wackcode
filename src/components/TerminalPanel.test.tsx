import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { Terminal } from "@xterm/xterm";
import { api } from "../api";
import { DEFAULT_APPEARANCE } from "../theme";
import type { TerminalFrame, TerminalInfo } from "../types";
import { ContextMenuProvider } from "./ui/ContextMenu";
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
    selection = "";
    selectionRange: { start: { x: number; y: number }; end: { x: number; y: number } } | undefined;
    selectionHandler?: () => void;
    written: string[] = [];
    open = vi.fn();
    loadAddon = vi.fn();
    attachCustomKeyEventHandler = vi.fn((handler: MockTerminal["keyHandler"]) => { this.keyHandler = handler; });
    onData = vi.fn((handler: (data: string) => void) => { this.dataHandler = handler; return { dispose: vi.fn() }; });
    onSelectionChange = vi.fn((handler: () => void) => { this.selectionHandler = handler; return { dispose: vi.fn() }; });
    getSelection = vi.fn(() => this.selection);
    getSelectionPosition = vi.fn(() => (this.selection ? this.selectionRange : undefined));
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
type Range = { start: { x: number; y: number }; end: { x: number; y: number } };
type MockTerm = {
  dataHandler?: (data: string) => void;
  keyHandler?: (event: KeyEvent) => boolean;
  selectionHandler?: () => void;
  selection: string;
  selectionRange?: Range;
  written: string[];
  reset: unknown;
};

function instances(): MockTerm[] {
  return (Terminal as unknown as { instances: MockTerm[] }).instances;
}

/** Report a selection the way xterm does: the state first, then the change event. */
function select(text: string, range: Range = { start: { x: 0, y: 2 }, end: { x: 10, y: 3 } }) {
  const term = instances()[0];
  term.selection = text;
  term.selectionRange = range;
  act(() => term.selectionHandler?.());
}

/** The panel's context menu needs the app's provider behind it. */
function withMenu(node: ReactNode) {
  return (
    <ContextMenuProvider scope="test" items={[]} copyText={vi.fn(async () => undefined)} readText={vi.fn(async () => "")} openLink={vi.fn(async () => undefined)} onError={vi.fn()}>
      {node}
    </ContextMenuProvider>
  );
}

function renderPanel() {
  return render(<TerminalPanel taskId="chat-1" appearance={DEFAULT_APPEARANCE} />);
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

  it("offers a selection to the draft — tidied — from the pill and the context menu", async () => {
    const onAttach = vi.fn();
    render(withMenu(<TerminalPanel taskId="chat-1" appearance={DEFAULT_APPEARANCE} onAttach={onAttach} />));
    await waitFor(() => expect(api.openTerminal).toHaveBeenCalled());
    expect(screen.queryByRole("button", { name: "Add to draft" })).toBeNull();

    select("ls --long   \n\ntotal 12  \n\n");
    fireEvent.click(await screen.findByRole("button", { name: "Add to draft" }));
    expect(onAttach).toHaveBeenCalledWith("ls --long\n\ntotal 12");

    fireEvent.contextMenu(document.querySelector(".terminal-screen")!);
    fireEvent.click(await screen.findByRole("menuitem", { name: "Add selection to draft" }));
    expect(onAttach).toHaveBeenCalledTimes(2);
    // The selection survives the hand-off; adding it again stays the user's call.
    expect(instances()[0].selection).toBe("ls --long   \n\ntotal 12  \n\n");
  });

  it("hides the pill mid-drag so it can't steal the selection's mouse-up", async () => {
    render(<TerminalPanel taskId="chat-1" appearance={DEFAULT_APPEARANCE} onAttach={vi.fn()} />);
    await waitFor(() => expect(api.openTerminal).toHaveBeenCalled());
    const host = document.querySelector(".terminal-screen")!;
    const term = instances()[0];

    fireEvent.mouseDown(host);
    term.selection = "hello";
    term.selectionRange = { start: { x: 0, y: 2 }, end: { x: 5, y: 2 } };
    act(() => term.selectionHandler?.());
    expect(screen.queryByRole("button", { name: "Add to draft" })).toBeNull();

    fireEvent.mouseUp(host);
    expect(screen.getByRole("button", { name: "Add to draft" })).toBeInTheDocument();
  });

  it("says why a selection didn't attach, and clears the note on the next change", async () => {
    const onAttach = vi.fn(() => { throw new Error("Attach at most 8 files to one message."); });
    render(<TerminalPanel taskId="chat-1" appearance={DEFAULT_APPEARANCE} onAttach={onAttach} />);
    await waitFor(() => expect(api.openTerminal).toHaveBeenCalled());

    select("hello");
    fireEvent.click(await screen.findByRole("button", { name: "Add to draft" }));
    expect(onAttach).toHaveBeenCalledWith("hello");
    await screen.findByText("Attach at most 8 files to one message.");

    select("more output");
    expect(screen.queryByText("Attach at most 8 files to one message.")).toBeNull();
  });

  it("disables the menu's add item while nothing is selected", async () => {
    render(withMenu(<TerminalPanel taskId="chat-1" appearance={DEFAULT_APPEARANCE} onAttach={vi.fn()} />));
    await waitFor(() => expect(api.openTerminal).toHaveBeenCalled());
    fireEvent.contextMenu(document.querySelector(".terminal-screen")!);
    expect(await screen.findByRole("menuitem", { name: "Add selection to draft" })).toBeDisabled();
  });
});
