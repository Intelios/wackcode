import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SessionSnapshot } from "../types";
import { ContextPanel } from "./ContextPanel";

afterEach(cleanup);

const stats = (percent: number | null): SessionSnapshot["stats"] => ({
  tokens: { input: 18_000, output: 2_200, cacheRead: 0, cacheWrite: 0, total: 20_200 },
  cost: 0.0421,
  contextUsage: { tokens: percent == null ? null : 84_000, contextWindow: 200_000, percent },
  contextBreakdown: { entries: [{ id: "user", tokens: 84_000 }], cacheHitRate: 0.5 }
});

const detailed = (): SessionSnapshot["stats"] => ({
  tokens: { input: 18_000, output: 2_200, cacheRead: 0, cacheWrite: 0, total: 20_200 },
  cost: 0.0421,
  contextUsage: { tokens: 84_000, contextWindow: 200_000, percent: 42 },
  contextBreakdown: {
    entries: [{ id: "user", tokens: 84_000 }],
    cacheHitRate: 0.5,
    detail: {
      groups: [
        {
          id: "messages",
          label: "Messages",
          tokens: 50_000,
          children: [
            { label: "User messages", tokens: 10_000 },
            { label: "Assistant replies", tokens: 15_000 },
            { label: "Tool results", tokens: 25_000 }
          ]
        },
        {
          id: "tools:app",
          label: "WackCode tools",
          tokens: 13_000,
          children: [{ label: "subagent", tokens: 8_000 }, { label: "todo", tokens: 5_000 }]
        },
        {
          id: "system",
          label: "System prompt",
          tokens: 21_000,
          children: [{ label: "Core prompt", tokens: 11_000 }, { label: "App tool guide", tokens: 10_000 }]
        }
      ],
      reserveTokens: 16_384
    }
  }
});

const reveal = () => fireEvent.mouseEnter(screen.getByRole("button", { name: /Context window/ }));

describe("ContextPanel", () => {
  it("shows a ring labelled with how full the context window is", () => {
    const { container } = render(<ContextPanel stats={stats(42)} />);
    expect(screen.getByRole("button", { name: "Context window 42% full" })).toBeInTheDocument();
    expect(container.querySelector(".context-ring-fill")).toHaveAttribute("stroke-dashoffset", "58");
  });

  it("reveals the details on hover and on keyboard focus", () => {
    render(<ContextPanel stats={stats(42)} />);
    reveal();
    const panel = screen.getByRole("dialog", { name: "Context window" });
    expect(panel).toHaveTextContent("84.0k of 200.0k tokens");
    expect(panel).toHaveTextContent("116.0k left");
    expect(panel).toHaveTextContent("20.2k");
    expect(panel).not.toHaveTextContent("$");
    expect(panel).toHaveTextContent("50.0%");
    cleanup();

    render(<ContextPanel stats={stats(42)} />);
    fireEvent.focus(screen.getByRole("button", { name: /Context window/ }));
    expect(screen.getByRole("dialog", { name: "Context window" })).toBeInTheDocument();
  });

  it("turns to the warning colour when nearly full, and draws only the track when unknown", () => {
    const { container, rerender } = render(<ContextPanel stats={stats(95)} />);
    expect(container.querySelector(".context-ring")).toHaveClass("full");
    rerender(<ContextPanel stats={stats(null)} />);
    expect(screen.getByRole("button", { name: "Context window usage unknown" })).toBeInTheDocument();
    expect(container.querySelector(".context-ring-fill")).toBeNull();
  });

  it("wears the glass class on its popover only when asked", () => {
    render(<ContextPanel stats={stats(42)} glass />);
    reveal();
    expect(document.querySelector(".context-pop")).toHaveClass("glass");
    cleanup();

    render(<ContextPanel stats={stats(42)} />);
    reveal();
    expect(document.querySelector(".context-pop")).not.toHaveClass("glass");
  });

  it("lists every cost as a share of the window, with the free space underneath", () => {
    render(<ContextPanel stats={detailed()} />);
    reveal();
    const panel = screen.getByRole("dialog", { name: "Context window" });
    expect(panel).toHaveTextContent("Messages");
    expect(panel).toHaveTextContent("50.0k");
    expect(panel).toHaveTextContent("25.0%");
    expect(panel).toHaveTextContent("WackCode tools");
    expect(panel).toHaveTextContent("13.0k");
    expect(panel).toHaveTextContent("System prompt");
    expect(panel).toHaveTextContent("21.0k");
    expect(panel).toHaveTextContent("Free space");
    expect(panel).toHaveTextContent("116.0k");
    expect(panel).toHaveTextContent("58.0%");
  });

  it("reads a tiny but real share as under a tenth of a percent", () => {
    const tiny = detailed();
    tiny.contextBreakdown!.detail!.groups = [
      { id: "messages", label: "Messages", tokens: 6 },
      { id: "system", label: "System prompt", tokens: 83_994 }
    ];
    render(<ContextPanel stats={tiny} />);
    reveal();
    expect(screen.getByRole("dialog", { name: "Context window" })).toHaveTextContent("<0.1%");
  });

  it("says how much room is left before Pi compacts on its own", () => {
    render(<ContextPanel stats={detailed()} />);
    reveal();
    expect(screen.getByRole("dialog", { name: "Context window" })).toHaveTextContent("99.6k until auto-compact");
  });

  it("says auto-compaction is due once the reserve is reached", () => {
    const near = detailed();
    near.contextUsage = { tokens: 190_000, contextWindow: 200_000, percent: 95 };
    render(<ContextPanel stats={near} />);
    reveal();
    expect(screen.getByRole("dialog", { name: "Context window" })).toHaveTextContent("Auto-compaction due");
  });

  it("expands a group to the items that make it up, and folds it again", () => {
    render(<ContextPanel stats={detailed()} />);
    reveal();
    const row = screen.getByRole("button", { name: /Messages/ });
    expect(row).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Tool results")).toBeNull();
    fireEvent.click(row);
    expect(row).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Tool results")).toBeInTheDocument();
    expect(screen.getByText("25.0k")).toBeInTheDocument();
    fireEvent.click(row);
    expect(screen.queryByText("Tool results")).toBeNull();
  });

  it("offers Compact, with the reason it is unavailable when it is", () => {
    const onCompact = vi.fn();
    render(<ContextPanel stats={detailed()} onCompact={onCompact} />);
    reveal();
    fireEvent.click(screen.getByRole("button", { name: "Compact" }));
    expect(onCompact).toHaveBeenCalledTimes(1);
    cleanup();

    render(<ContextPanel stats={detailed()} onCompact={onCompact} compact={{ disabledReason: "Not enough conversation to compact yet." }} />);
    reveal();
    expect(screen.getByRole("button", { name: "Compact" })).toBeDisabled();
    expect(screen.getByRole("dialog", { name: "Context window" })).toHaveTextContent("Not enough conversation to compact yet.");
  });

  it("keeps the coarse split for a chat with no live measurement", () => {
    render(<ContextPanel stats={stats(42)} />);
    reveal();
    const panel = screen.getByRole("dialog", { name: "Context window" });
    expect(panel).toHaveTextContent("User messages");
    expect(panel).not.toHaveTextContent("Free space");
    expect(panel).not.toHaveTextContent("until auto-compact");
    expect(screen.queryByRole("button", { name: "Compact" })).toBeNull();
  });
});
