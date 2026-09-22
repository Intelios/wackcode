import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NormalizedMessage } from "../types";
import { Transcript } from "./Transcript";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("Transcript tool output", () => {
  it("keeps live output collapsed, replaces updates, then shows the final result", () => {
    const call: NormalizedMessage = {
      id: "assistant-1",
      role: "assistant",
      blocks: [{ type: "tool-call", toolName: "bash", toolCallId: "call-1", arguments: { command: "slow-command" } }]
    };
    const result: NormalizedMessage = {
      id: "tool-2",
      role: "tool",
      blocks: [{ type: "tool-result", toolName: "bash", toolCallId: "call-1", text: "final output" }]
    };
    const view = render(<Transcript messages={[call]} running liveToolText={{ "call-1": "first update" }} />);
    const row = screen.getByRole("button", { name: /slow-command/ });
    expect(row).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("first update")).not.toBeInTheDocument();

    fireEvent.click(row);
    expect(screen.getByText("first update")).toBeInTheDocument();

    view.rerender(<Transcript messages={[call]} running liveToolText={{ "call-1": "first update\nsecond update" }} />);
    expect(screen.getByText("first update second update")).toBeInTheDocument();
    expect(screen.queryByText("first update", { exact: true })).not.toBeInTheDocument();

    view.rerender(<Transcript messages={[call, result]} running={false} liveToolText={{}} />);
    expect(screen.getByText("final output")).toBeInTheDocument();
    expect(screen.queryByText("second update", { exact: false })).not.toBeInTheDocument();
  });
});

describe("Transcript run durations", () => {
  it("places saved durations between each user prompt and assistant reply", () => {
    const messages: NormalizedMessage[] = [
      { id: "user-1", role: "user", timestamp: 1_000, blocks: [{ type: "text", text: "First prompt" }] },
      { id: "assistant-1", role: "assistant", blocks: [{ type: "text", text: "First answer" }] },
      { id: "user-2", role: "user", timestamp: 2_000, blocks: [{ type: "text", text: "Second prompt" }] },
      { id: "assistant-2", role: "assistant", blocks: [{ type: "text", text: "Second answer" }] }
    ];
    const { container } = render(<Transcript
      messages={messages}
      running={false}
      runTimings={[
        { userMessageId: "user-1", durationMs: 62_000 },
        { userMessageId: "user-2", durationMs: 3_661_000 }
      ]}
    />);

    const rows = container.querySelectorAll(".run-duration");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("Worked for 1m 2s");
    expect(rows[0].previousElementSibling).toHaveClass("msg", "user");
    expect(rows[0].nextElementSibling).toHaveClass("msg", "assistant");
    expect(rows[1]).toHaveTextContent("Worked for 1h 1m 1s");
    expect(rows[1].previousElementSibling).toHaveClass("msg", "user");
    expect(rows[1].nextElementSibling).toHaveClass("msg", "assistant");
  });

  it("ticks the active duration once a second, then keeps the completed total", () => {
    vi.useFakeTimers();
    const startedAt = Date.now() - 1_500;
    const messages: NormalizedMessage[] = [
      { id: "user-1", role: "user", timestamp: Date.now() - 500, blocks: [{ type: "text", text: "Do the work" }] },
      { id: "assistant-1", role: "assistant", blocks: [{ type: "text", text: "Working" }] }
    ];
    const view = render(<Transcript messages={messages} running activeRun={{ startedAt }} />);
    expect(screen.getByText("Working for 1s")).toBeInTheDocument();

    act(() => { vi.advanceTimersByTime(1_200); });
    expect(screen.getByText("Working for 2s")).toBeInTheDocument();

    view.rerender(<Transcript messages={messages} running={false} runTimings={[{ userMessageId: "user-1", durationMs: 4_200 }]} />);
    expect(screen.getByText("Worked for 4s")).toBeInTheDocument();
    expect(screen.queryByText(/Working for/)).not.toBeInTheDocument();
  });
});
