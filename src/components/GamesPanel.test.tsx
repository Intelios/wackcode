import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { clearSave, SAVE_KEY, writeSave } from "../games/save";
import { clearRun } from "../games/session";
import { createQuack } from "../games/quack";
import { GamesPanel } from "./GamesPanel";

afterEach(() => {
  cleanup();
  clearRun("quack");
  clearSave();
  localStorage.clear();
});

function props(overrides: Partial<React.ComponentProps<typeof GamesPanel>> = {}) {
  return { status: "idle" as const, watchKey: "chat-1", agentName: "Nova", onClose: vi.fn(), onBackToChat: vi.fn(), ...overrides };
}

describe("GamesPanel", () => {
  it("opens on Quack Survivors' title and starts a run", async () => {
    render(<GamesPanel {...props()} />);
    expect(screen.getByRole("heading", { name: "Quack Survivors", level: 2 })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Play" })).toBeNull());
    expect(screen.getByRole("application", { name: "Quack Survivors" })).toBeInTheDocument();
  });

  it("pauses on Esc and comes back paused after a remount", () => {
    const { unmount } = render(<GamesPanel {...props()} />);
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    fireEvent.keyDown(screen.getByRole("application", { name: "Quack Survivors" }), { key: "Escape" });
    expect(screen.getByRole("heading", { name: "Paused" })).toBeInTheDocument();
    unmount();

    render(<GamesPanel {...props()} />);
    expect(screen.getByRole("heading", { name: "Paused" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "End run" }));
    expect(screen.getByRole("button", { name: "Play" })).toBeInTheDocument();
  });

  it("auto-saves the run, so a cold mount can continue it", () => {
    const { unmount } = render(<GamesPanel {...props()} />);
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    // The throttle would also let the 1s tick write; the pause force-writes immediately.
    fireEvent.keyDown(screen.getByRole("application", { name: "Quack Survivors" }), { key: "Escape" });
    expect(localStorage.getItem(SAVE_KEY)).toContain("\"version\":1");
    unmount();
    // Simulate the app having quit: nothing in memory, only the save on disk.
    clearRun("quack");

    render(<GamesPanel {...props()} />);
    const cont = screen.getByRole("button", { name: /^Continue ·/ });
    fireEvent.click(cont);
    expect(screen.getByRole("heading", { name: "Paused" })).toBeInTheDocument();
  });

  it("offers a saved run's continue on a fresh mount and drops it on end run", () => {
    const saved = createQuack(9);
    saved.time = 83_000;
    saved.player.level = 4;
    saved.kills = 31;
    writeSave(saved, true);
    // No in-memory run: this is the cold-start path after quitting the app.
    clearRun("quack");

    render(<GamesPanel {...props()} />);
    fireEvent.click(screen.getByRole("button", { name: /^Continue ·/ }));
    fireEvent.keyDown(screen.getByRole("application", { name: "Quack Survivors" }), { key: "Escape" });
    expect(screen.getByRole("button", { name: "Save & quit" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "End run" }));
    expect(screen.queryByRole("button", { name: /^Continue ·/ })).toBeNull();
  });

  it("Save & quit leaves for the chat and keeps the run saved", () => {
    const p = props();
    const saved = createQuack(9);
    writeSave(saved, true);

    render(<GamesPanel {...p} />);
    fireEvent.click(screen.getByRole("button", { name: /^Continue ·/ }));
    fireEvent.keyDown(screen.getByRole("application", { name: "Quack Survivors" }), { key: "Escape" });
    fireEvent.click(screen.getByRole("button", { name: "Save & quit" }));
    expect(p.onBackToChat).toHaveBeenCalledOnce();
    expect(localStorage.getItem(SAVE_KEY)).not.toBeNull();
  });

  it("shows the agent working, then a toast when its run ends", async () => {
    const p = props();
    const { rerender } = render(<GamesPanel {...p} />);
    expect(screen.queryByText(/finished/)).toBeNull();
    rerender(<GamesPanel {...p} status="running" />);
    expect(screen.getByText("Nova is working")).toBeInTheDocument();
    rerender(<GamesPanel {...p} status="idle" />);
    expect(screen.getByText("Nova finished")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Back to chat" }));
    expect(p.onBackToChat).toHaveBeenCalledOnce();
    await waitFor(() => expect(screen.queryByText("Nova finished")).toBeNull());
  });

  it("names a failed or stopped run, and dismisses itself", () => {
    vi.useFakeTimers();
    try {
      const p = props({ status: "running" });
      const { rerender } = render(<GamesPanel {...p} />);
      rerender(<GamesPanel {...p} status="error" />);
      expect(screen.getByText("Nova hit an error")).toBeInTheDocument();
      act(() => { vi.advanceTimersByTime(10_000); });
      rerender(<GamesPanel {...p} status="running" />);
      rerender(<GamesPanel {...p} status="interrupted" />);
      expect(screen.getByText("Nova stopped")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("raises no toast when the open chat changes rather than finishes", () => {
    const p = props({ status: "running" });
    const { rerender } = render(<GamesPanel {...p} />);
    rerender(<GamesPanel {...p} watchKey="chat-2" status="idle" />);
    expect(screen.queryByText(/finished/)).toBeNull();
  });

  it("closes from its header", () => {
    const p = props();
    render(<GamesPanel {...p} />);
    fireEvent.click(screen.getByRole("button", { name: "Close Games panel" }));
    expect(p.onClose).toHaveBeenCalledOnce();
  });
});
