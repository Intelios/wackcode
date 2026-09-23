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

describe("Transcript sub-agent calls", () => {
  const call: NormalizedMessage = {
    id: "assistant-1",
    role: "assistant",
    blocks: [{ type: "tool-call", toolName: "subagent", toolCallId: "call-1", arguments: { agent: "scout", task: "Map the worker" } }]
  };
  const usage = { input: 10, output: 5, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 1 };

  it("shows the card from the call's arguments, then live progress, then the final result", () => {
    const view = render(<Transcript messages={[call]} running />);
    expect(screen.getByText("Map the worker")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Queued" })).toBeInTheDocument();

    const live = { v: 1, mode: "single", results: [{ agent: "scout", task: "Map the worker", readOnly: true, status: "running", activity: [{ tool: "read", subject: "worker/src/index.ts" }], usage }] };
    view.rerender(<Transcript messages={[call]} running liveToolDetails={{ "call-1": live }} />);
    expect(screen.getByText("worker/src/index.ts")).toBeInTheDocument();

    const result: NormalizedMessage = {
      id: "tool-2",
      role: "tool",
      blocks: [{ type: "tool-result", toolName: "subagent", toolCallId: "call-1", text: "Mapped.", details: { ...live, results: [{ ...live.results[0], status: "done", output: "Mapped." }] } }]
    };
    view.rerender(<Transcript messages={[call, result]} running={false} />);
    expect(screen.getByRole("img", { name: "Done" })).toBeInTheDocument();
  });

  it("falls back to a plain row when the call was refused before any sub-agent ran", () => {
    const refused: NormalizedMessage = {
      id: "tool-2",
      role: "tool",
      blocks: [{ type: "tool-result", toolName: "subagent", toolCallId: "call-1", text: "Sub-agents are switched off in Settings.", isError: true, details: {} }]
    };
    render(<Transcript messages={[call, refused]} running={false} />);
    expect(screen.getByRole("button", { name: /Ran sub-agent/ })).toBeInTheDocument();
    expect(screen.getByText("failed")).toBeInTheDocument();
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

describe("Transcript message actions", () => {
  const user: NormalizedMessage = {
    id: "u2", entryId: "u2", role: "user", blocks: [{ type: "text", text: "Make it blue" }],
    versions: { index: 1, total: 2, previous: "u1", group: "versions:root" },
    checkpoint: { id: "a".repeat(40) }
  };
  const answer: NormalizedMessage = {
    id: "a2", entryId: "a2", role: "assistant", blocks: [{ type: "text", text: "Done." }],
    turn: { userEntryId: "u2", endEntryId: "a2" }
  };

  it("offers retry, edit, rewind, version switching and fork only while idle", () => {
    const onMessageAction = vi.fn();
    const view = render(<Transcript messages={[user, answer]} running={false} actionsEnabled onMessageAction={onMessageAction} />);
    expect(screen.getByText("2/2")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Previous version" }));
    expect(onMessageAction).toHaveBeenLastCalledWith({ type: "switch", entryId: "u1" });
    expect(screen.getByRole("button", { name: "Next version" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onMessageAction).toHaveBeenLastCalledWith({ type: "retry", message: answer });
    fireEvent.click(screen.getByRole("button", { name: "Rewind to here" }));
    expect(onMessageAction).toHaveBeenLastCalledWith({ type: "rewind", message: user });
    fireEvent.click(screen.getByRole("button", { name: "Fork from here" }));
    expect(onMessageAction).toHaveBeenLastCalledWith({ type: "fork", message: answer });

    view.rerender(<Transcript messages={[user, answer]} running actionsEnabled={false} onMessageAction={onMessageAction} />);
    expect(screen.queryByRole("button", { name: "Retry" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Fork from here" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Previous version" })).toBeDisabled();
    expect(screen.getAllByRole("button", { name: "Copy" })).toHaveLength(2);
  });

  it("edits a message in place and closes the editor once the edit is sent", async () => {
    const onMessageAction = vi.fn().mockResolvedValue(true);
    render(<Transcript messages={[user, answer]} running={false} actionsEnabled onMessageAction={onMessageAction} />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const editor = screen.getByRole("textbox", { name: "Edit message" });
    expect(editor).toHaveValue("Make it blue");
    fireEvent.change(editor, { target: { value: "Make it green" } });
    await act(async () => { fireEvent.keyDown(editor, { key: "Enter" }); });
    expect(onMessageAction).toHaveBeenCalledWith({ type: "edit", message: user, text: "Make it green", removeImages: [] });
    expect(screen.queryByRole("textbox", { name: "Edit message" })).not.toBeInTheDocument();
  });

  it("keeps the editor open when the edit was not sent, and Escape cancels it", async () => {
    const onMessageAction = vi.fn().mockResolvedValue(false);
    render(<Transcript messages={[user, answer]} running={false} actionsEnabled onMessageAction={onMessageAction} />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const editor = screen.getByRole("textbox", { name: "Edit message" });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Send" })); });
    expect(screen.getByRole("textbox", { name: "Edit message" })).toBeInTheDocument();
    fireEvent.keyDown(editor, { key: "Escape" });
    expect(screen.queryByRole("textbox", { name: "Edit message" })).not.toBeInTheDocument();
    expect(screen.getByText("Make it blue")).toBeInTheDocument();
  });

  it("refuses to send kept images to a model without vision until they are removed", async () => {
    const withImage: NormalizedMessage = { ...user, versions: undefined, blocks: [...user.blocks, { type: "image", imageId: "image-1", mimeType: "image/png" }] };
    const onMessageAction = vi.fn().mockResolvedValue(true);
    render(<Transcript messages={[withImage]} running={false} actionsEnabled vision={false} modelName="Text Model" onMessageAction={onMessageAction} />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(screen.getByRole("status")).toHaveTextContent("Text Model doesn't accept images");
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Remove image 1" }));
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Send" })); });
    expect(onMessageAction).toHaveBeenCalledWith({ type: "edit", message: withImage, text: "Make it blue", removeImages: [0] });
  });

  it("offers retry on an unanswered latest message and shows the empty state after rewinding the first one", () => {
    const onUndoRewind = vi.fn();
    const unanswered: NormalizedMessage = { ...user, versions: undefined };
    const view = render(<Transcript messages={[unanswered]} running={false} actionsEnabled onMessageAction={vi.fn()} />);
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();

    const system: NormalizedMessage = { id: "s", role: "system", blocks: [{ type: "text", text: "" }] };
    view.rerender(<Transcript messages={[system]} running={false} actionsEnabled onUndoRewind={onUndoRewind} />);
    expect(screen.getByText("What should we build?")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Undo rewind/ }));
    expect(onUndoRewind).toHaveBeenCalled();
  });
});
