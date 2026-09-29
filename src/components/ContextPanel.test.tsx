import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { SessionSnapshot } from "../types";
import { ContextPanel } from "./ContextPanel";

afterEach(cleanup);

const stats = (percent: number | null): SessionSnapshot["stats"] => ({
  tokens: { input: 18_000, output: 2_200, cacheRead: 0, cacheWrite: 0, total: 20_200 },
  cost: 0.0421,
  contextUsage: { tokens: percent == null ? null : 84_000, contextWindow: 200_000, percent },
  contextBreakdown: { entries: [{ id: "user", tokens: 84_000 }], cacheHitRate: 0.5 }
});

describe("ContextPanel", () => {
  it("shows a ring labelled with how full the context window is", () => {
    const { container } = render(<ContextPanel stats={stats(42)} />);
    expect(screen.getByRole("button", { name: "Context window 42% full" })).toBeInTheDocument();
    expect(container.querySelector(".context-ring-fill")).toHaveAttribute("stroke-dashoffset", "58");
  });

  it("reveals the details on hover and on keyboard focus", () => {
    render(<ContextPanel stats={stats(42)} />);
    const trigger = screen.getByRole("button", { name: /Context window/ });
    fireEvent.mouseEnter(trigger);
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
});
