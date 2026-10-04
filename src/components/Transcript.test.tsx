import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NormalizedMessage } from "../types";
import { composeFileSection } from "../attachment-utils";
import { ExploreGroupingEnabled } from "./ExploreGroup";
import { SubagentPanelLink } from "./SubagentChip";
import { ThinkingPreviewEnabled } from "./ThinkingRow";
import { Transcript } from "./Transcript";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("Transcript model switches", () => {
  const messages: NormalizedMessage[] = [
    { id: "u", role: "user", blocks: [{ type: "text", text: "Hello" }] },
    { id: "a", role: "assistant", blocks: [{ type: "text", text: "Hi" }] }
  ];

  it("interleaves named dividers and supports a trailing switch", () => {
    const { container } = render(<Transcript messages={messages} running={false} modelSwitches={[
      { id: "between", at: 1, from: "GPT 5", to: "Claude Sonnet 4" },
      { id: "trailing", at: 2, from: "Claude Sonnet 4", to: "GPT 5" }
    ]} />);
    const transcript = container.querySelector(".transcript");
    const children = Array.from(transcript?.children ?? []);
    expect(children.map((entry) => entry.textContent?.trim())).toEqual([
      "Hello",
      "Model switched GPT 5 → Claude Sonnet 4",
      "Hi",
      "Model switched Claude Sonnet 4 → GPT 5"
    ]);
  });
});

describe("Transcript pending echo", () => {
  it("shows a just-sent message as the active turn while the run has no recorded message yet", () => {
    const startedAt = 1_000;
    const echo: NormalizedMessage = {
      id: `pending:${startedAt}`,
      role: "user",
      timestamp: startedAt,
      blocks: [{ type: "text", text: "Ship it" }]
    };
    const history: NormalizedMessage[] = [
      { id: "u", role: "user", timestamp: 500, blocks: [{ type: "text", text: "Hello" }] },
      { id: "a", role: "assistant", blocks: [{ type: "text", text: "Hi" }] }
    ];
    const { container } = render(<Transcript messages={[...history, echo]} running activeRun={{ startedAt }} activity="starting" />);
    const transcript = container.querySelector(".transcript");
    const rows = Array.from(transcript?.children ?? []).map((entry) => entry.textContent?.trim() ?? "");
    expect(rows.slice(0, 3)).toEqual(["Hello", "Hi", "Ship it"]);
    // The echo is the active turn: the run's clock chip sits under it, not as a bare row.
    expect(rows[3]).toMatch(/^Working for/);
    expect(rows).toContain("Working…");
    // No message actions offer retry or edit while the run is live (actionsEnabled stays off
    // for a running chat).
    expect(screen.queryByRole("button", { name: /retry/i })).toBeNull();
  });
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

describe("Transcript thinking", () => {
  const user: NormalizedMessage = { id: "user-1", role: "user", timestamp: 1_000, blocks: [{ type: "text", text: "Why?" }] };
  const streamed = (blocks: NormalizedMessage["blocks"]): NormalizedMessage => ({ id: "assistant-2000-0", role: "assistant", timestamp: 2_000, blocks });

  it("can be expanded while the model reasons, and stays open with its measured time once answered and saved", () => {
    const view = render(<Transcript messages={[user]} running partial={streamed([{ type: "thinking", text: "Weighing the options" }])} />);
    const row = screen.getByRole("button", { name: "Thinking…" });
    expect(row).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(row);
    // Scoped: the stream keeps its copy of the text until its exit animation finishes.
    expect(within(screen.getByRole("region", { name: "Reasoning" })).getByText("Weighing the options")).toBeInTheDocument();

    // The answer has started: the worker has clocked the reasoning, though the message still streams.
    const answering = streamed([{ type: "thinking", text: "Weighing the options", durationMs: 4_200 }, { type: "text", text: "Because" }]);
    view.rerender(<Transcript messages={[user]} running partial={answering} />);
    expect(screen.getByRole("button", { name: "Thought for 4s" })).toHaveAttribute("aria-expanded", "true");

    // The saved message replaces the streamed one as a separate element, and keeps the row open.
    const saved: NormalizedMessage = { ...answering, id: "entry-assistant", blocks: [answering.blocks[0], { type: "text", text: "Because." }] };
    view.rerender(<Transcript messages={[user, saved]} running={false} />);
    expect(screen.getByRole("button", { name: "Thought for 4s" })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Weighing the options")).toBeInTheDocument();
  });

  it("passes the worker's block start to the live timer and switches to the measured final duration", () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    const view = render(<Transcript messages={[user]} running partial={streamed([{ type: "thinking", text: "", startedAt: 8_000 }])} />);
    expect(screen.getByRole("button", { name: "Thinking… 2s" })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(3_000));
    expect(screen.getByRole("button", { name: "Thinking… 5s" })).toBeInTheDocument();
    view.rerender(<Transcript messages={[user]} running partial={streamed([
      { type: "thinking", text: "Done", durationMs: 5_100 }, { type: "text", text: "Answer" }
    ])} />);
    expect(screen.getByRole("button", { name: "Thought for 5s" })).toBeInTheDocument();
    expect(screen.queryByText("Thinking…")).not.toBeInTheDocument();
  });

  it("stops an unfinished thinking timer when the run is interrupted, without losing the reasoning", () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    const partial = streamed([{ type: "thinking", text: "Unfinished reasoning", startedAt: 8_000 }]);
    const view = render(<Transcript messages={[user]} running partial={partial} />);
    expect(screen.getByRole("button", { name: "Thinking… 2s" })).toBeInTheDocument();
    view.rerender(<Transcript messages={[user]} running={false} partial={partial} />);
    fireEvent.click(screen.getByRole("button", { name: "Reasoning" }));
    expect(within(screen.getByRole("region", { name: "Reasoning" })).getByText("Unfinished reasoning")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(3_000));
    expect(screen.queryByText("Thinking…")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reasoning" })).toBeInTheDocument();
  });

  it("labels saved reasoning with its duration, and without one when it was never clocked", () => {
    const message = (id: string, timestamp: number, durationMs?: number): NormalizedMessage => ({
      id, role: "assistant", timestamp, blocks: [{ type: "thinking", text: "Hmm", ...(durationMs === undefined ? {} : { durationMs }) }, { type: "text", text: "Done" }]
    });
    render(<Transcript messages={[user, message("a", 2_000, 400), message("b", 3_000, 75_000), message("c", 4_000)]} running={false} />);
    expect(screen.getByRole("button", { name: "Thought for <1s" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Thought for 1m 15s" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reasoning" })).toBeInTheDocument();
    expect(screen.queryByText("Thinking…")).not.toBeInTheDocument();
  });
});

describe("Transcript thinking preview", () => {
  const user: NormalizedMessage = { id: "user-1", role: "user", timestamp: 1_000, blocks: [{ type: "text", text: "Why?" }] };
  const thinking = (text: string, durationMs?: number): NormalizedMessage => ({
    id: "assistant-2000-0", role: "assistant", timestamp: 2_000,
    blocks: [{ type: "thinking", text, ...(durationMs === undefined ? {} : { durationMs }) }]
  });
  const reasoning = "I read the config. Now I check the tes";

  it("streams the reasoning tail beside a live row, unfinished words included", () => {
    render(<Transcript messages={[user]} running partial={thinking(reasoning)} />);
    expect(screen.getByText("I read the config. Now I check the tes")).toBeInTheDocument();
  });

  it("hides it once the reasoning is clocked, while expanded, and when switched off", async () => {
    const view = render(<Transcript messages={[user]} running partial={thinking(reasoning)} />);
    fireEvent.click(screen.getByRole("button", { name: /Thinking…/ }));
    // The stream animates out when the row opens.
    await waitFor(() => expect(view.container.querySelector(".thinking-stream")).toBeNull());

    view.rerender(<Transcript messages={[user]} running partial={thinking(reasoning, 3_000)} />);
    expect(screen.getByRole("button", { name: "Thought for 3s" })).toBeInTheDocument();
    // The stream animates out once the block is clocked.
    await waitFor(() => expect(view.container.querySelector(".thinking-stream")).toBeNull());
    cleanup();

    render(
      <ThinkingPreviewEnabled.Provider value={false}>
        <Transcript messages={[user]} running partial={thinking(reasoning)} />
      </ThinkingPreviewEnabled.Provider>
    );
    expect(screen.getByRole("button", { name: "Thinking…" })).toBeInTheDocument();
    expect(screen.queryByText("I read the config. Now I check the tes")).not.toBeInTheDocument();
  });
});

describe("Transcript sub-agent calls", () => {
  const call: NormalizedMessage = {
    id: "assistant-1",
    role: "assistant",
    blocks: [{ type: "tool-call", toolName: "subagent", toolCallId: "call-1", arguments: { agent: "scout", task: "Map the worker" } }]
  };
  const usage = { input: 10, output: 5, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 1 };

  it("shows a SubAgent chip from the call's arguments, then live progress, then the final result", () => {
    const view = render(<Transcript messages={[call]} running />);
    const chip = screen.getByRole("button", { name: "SubAgent Scout, queued: Map the worker" });
    expect(within(chip).getByText("SubAgent")).toBeInTheDocument();
    expect(within(chip).getByText("Scout")).toBeInTheDocument();
    expect(within(chip).getByText("Map the worker")).toBeInTheDocument();
    expect(within(chip).getByText("Queued")).toBeInTheDocument();

    const live = { v: 1, mode: "single", results: [{ agent: "scout", task: "Map the worker", readOnly: true, status: "running", activity: [{ tool: "read", subject: "worker/src/index.ts" }], usage }] };
    view.rerender(<Transcript messages={[call]} running liveToolDetails={{ "call-1": live }} />);
    expect(screen.getByRole("button", { name: /^SubAgent Scout, working/ })).toBeInTheDocument();
    expect(screen.getByText("Reading")).toBeInTheDocument();
    expect(screen.getByText("index.ts")).toBeInTheDocument();

    const result: NormalizedMessage = {
      id: "tool-2",
      role: "tool",
      blocks: [{ type: "tool-result", toolName: "subagent", toolCallId: "call-1", text: "Mapped.", details: { ...live, results: [{ ...live.results[0], status: "done", output: "Mapped.", startedAt: 1_000, endedAt: 13_000 }] } }]
    };
    view.rerender(<Transcript messages={[call, result]} running={false} />);
    const done = screen.getByRole("button", { name: /^SubAgent Scout, done/ });
    expect(within(done).getByText("12s")).toBeInTheDocument();
  });

  it("opens a chip's sub-agent in the side panel, and marks the chip it shows", () => {
    const onOpen = vi.fn();
    const view = render(<SubagentPanelLink.Provider value={{ onOpen }}><Transcript messages={[call]} running /></SubagentPanelLink.Provider>);
    const chip = screen.getByRole("button", { name: /^SubAgent Scout/ });
    expect(chip).toHaveAttribute("aria-expanded", "false");
    expect(chip).toHaveAttribute("aria-controls", "side-panel");
    fireEvent.click(chip);
    expect(onOpen).toHaveBeenCalledWith("call-1", 0);

    view.rerender(<SubagentPanelLink.Provider value={{ onOpen, open: { toolCallId: "call-1", index: 0 } }}><Transcript messages={[call]} running /></SubagentPanelLink.Provider>);
    expect(screen.getByRole("button", { name: /^SubAgent Scout/ })).toHaveAttribute("aria-expanded", "true");
  });

  it("gives each child of a parallel call its own chip under one summary", () => {
    const parallel: NormalizedMessage = {
      id: "assistant-1",
      role: "assistant",
      blocks: [{ type: "tool-call", toolName: "subagent", toolCallId: "call-1", arguments: { tasks: [{ agent: "scout", task: "One" }, { agent: "code-reviewer", task: "Two" }] } }]
    };
    const onOpen = vi.fn();
    render(<SubagentPanelLink.Provider value={{ onOpen }}><Transcript messages={[parallel]} running /></SubagentPanelLink.Provider>);
    expect(screen.getByText("2 SubAgents in parallel")).toBeInTheDocument();
    expect(screen.getByText("2 queued")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^SubAgent Code Reviewer/ }));
    expect(onOpen).toHaveBeenCalledWith("call-1", 1);
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

describe("Transcript plan cards", () => {
  const plan = "# The plan\n\n- Ship it";
  const messages: NormalizedMessage[] = [
    { id: "assistant-1", role: "assistant", blocks: [{ type: "tool-call", toolName: "plan_mode_complete", toolCallId: "call-1", arguments: { plan } }] },
    { id: "tool-2", role: "tool", blocks: [{ type: "tool-result", toolName: "plan_mode_complete", toolCallId: "call-1", text: plan, details: { version: 1, source: "plan_mode_complete", plan } }] }
  ];

  it("keeps the review actions on the awaiting plan in Plan and Ultra Plan, not in Build", () => {
    const view = render(<Transcript messages={messages} running={false} planState={{ mode: "ultraplan", phase: "ready", plan }} onPlanAction={() => undefined} />);
    expect(screen.getByRole("button", { name: "Approve & implement" })).toBeInTheDocument();
    view.rerender(<Transcript messages={messages} running={false} planState={{ mode: "plan", phase: "ready", plan }} onPlanAction={() => undefined} />);
    expect(screen.getByRole("button", { name: "Approve & implement" })).toBeInTheDocument();
    view.rerender(<Transcript messages={messages} running={false} planState={{ mode: "build", phase: "planning" }} onPlanAction={() => undefined} />);
    expect(screen.queryByRole("button", { name: "Approve & implement" })).not.toBeInTheDocument();
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
    const row = () => view.container.querySelectorAll(".run-duration");
    expect(row()).toHaveLength(1);
    expect(row()[0]).toHaveTextContent("Working for 1s");
    expect(row()[0]).toHaveClass("live");

    act(() => { vi.advanceTimersByTime(1_200); });
    expect(row()[0]).toHaveTextContent("Working for 2s");

    view.rerender(<Transcript messages={messages} running={false} runTimings={[{ userMessageId: "user-1", durationMs: 4_200 }]} />);
    expect(row()).toHaveLength(1);
    expect(row()[0]).toHaveTextContent("Worked for 4s");
    expect(row()[0]).not.toHaveClass("live");
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
    expect(onMessageAction).toHaveBeenCalledWith({ type: "edit", message: user, text: "Make it green", files: [], removeImages: [] });
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
    expect(onMessageAction).toHaveBeenCalledWith({ type: "edit", message: withImage, text: "Make it blue", files: [], removeImages: [0] });
  });

  it("shows attached files as chips and lets the edit drop one", async () => {
    const withFiles: NormalizedMessage = {
      ...user,
      versions: undefined,
      blocks: [{ type: "text", text: composeFileSection("Make it blue", [{ name: "notes.txt", text: "one\ntwo" }, { name: "more.txt", text: "x" }]) }]
    };
    const onMessageAction = vi.fn().mockResolvedValue(true);
    render(<Transcript messages={[withFiles]} running={false} actionsEnabled onMessageAction={onMessageAction} />);
    // The words show without the generated section; each file is a chip that opens its text.
    expect(screen.getByText("Make it blue")).toBeInTheDocument();
    expect(screen.queryByText(/attached-files/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /notes\.txt/ }));
    expect(screen.getByText(/one\s*two/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(screen.getByRole("textbox", { name: "Edit message" })).toHaveValue("Make it blue");
    fireEvent.click(screen.getByRole("button", { name: "Remove file 1" }));
    await act(async () => { fireEvent.keyDown(screen.getByRole("textbox", { name: "Edit message" }), { key: "Enter" }); });
    expect(onMessageAction).toHaveBeenCalledWith({
      type: "edit", message: withFiles, text: "Make it blue",
      files: [{ name: "more.txt", text: "x" }], removeImages: []
    });
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

describe("Transcript command messages", () => {
  it("shows the compact invocation and opens the exact sent prompt", async () => {
    const onMessageAction = vi.fn().mockResolvedValue(true);
    const command: NormalizedMessage = {
      id: "goal-1",
      role: "user",
      blocks: [{ type: "text", text: "\nThe user has set a goal.\n<objective>\nFix @src/App.tsx\n</objective>\n" }],
      commandPresentation: {
        id: "app:goal",
        name: "goal",
        arguments: "Fix @src/App.tsx",
        kind: "command"
      }
    };
    const { container } = render(<Transcript messages={[command]} running={false} onMessageAction={onMessageAction} />);

    const trigger = screen.getByRole("button", { name: "goal command. View sent prompt" });
    expect(trigger).toHaveTextContent("goal");
    expect(container.querySelector(".command-message")).toHaveTextContent("goal·Fix @src/App.tsx");
    expect(container.querySelector(".command-summary .mention")).toHaveTextContent("@src/App.tsx");
    expect(screen.queryByText(/The user has set a goal/)).not.toBeInTheDocument();

    fireEvent.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("dialog", { name: "goal prompt sent to the agent" })).toBeInTheDocument();
    expect(screen.getByText(/The user has set a goal/)).toHaveTextContent("Fix @src/App.tsx");
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Copy prompt" })); });
    expect(onMessageAction).toHaveBeenCalledWith({ type: "copy-prompt", text: command.blocks[0].text });
    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });

  it("labels automatic and resumed rounds while historical messages remain ordinary", () => {
    const automatic: NormalizedMessage = {
      id: "auto",
      role: "user",
      blocks: [{ type: "text", text: "Full automatic prompt" }],
      commandPresentation: { id: "app:goal", name: "goal", arguments: "", kind: "goal-continuation", round: 2, nextAction: "Run tests" }
    };
    const resumed: NormalizedMessage = {
      id: "resume",
      role: "user",
      blocks: [{ type: "text", text: "Full resume prompt" }],
      commandPresentation: { id: "app:goal", name: "goal", arguments: "resume", kind: "goal-resume", round: 3, nextAction: "Fix failure" }
    };
    const historical: NormalizedMessage = { id: "old", role: "user", blocks: [{ type: "text", text: "Expanded old prompt" }] };
    render(<Transcript messages={[automatic, resumed, historical]} running={false} />);
    expect(screen.getByText("Automatic round 2 · Run tests")).toBeInTheDocument();
    expect(screen.getByText("Resumed round 3 · Fix failure")).toBeInTheDocument();
    expect(screen.getByText("Expanded old prompt")).toBeInTheDocument();
  });
});

describe("Transcript exploration groups", () => {
  const user: NormalizedMessage = { id: "user-1", role: "user", timestamp: 1_000, blocks: [{ type: "text", text: "Look around" }] };
  const read = (id: string, path: string) => ({ type: "tool-call" as const, toolName: "read", toolCallId: id, arguments: { path } });
  const result = (id: string, text = "contents"): NormalizedMessage => ({ id: `tool-${id}`, role: "tool", blocks: [{ type: "tool-result", toolCallId: id, text }] });
  const first: NormalizedMessage = { id: "a-2000", role: "assistant", timestamp: 2_000, blocks: [{ type: "text", text: "Reading first." }, read("r1", "src/one.ts")] };
  const second: NormalizedMessage = { id: "a-3000", role: "assistant", timestamp: 3_000, blocks: [{ type: "tool-call", toolName: "grep", toolCallId: "g1", arguments: { pattern: "needle" } }] };
  const answer: NormalizedMessage = { id: "a-4000", role: "assistant", timestamp: 4_000, blocks: [{ type: "text", text: "All done." }] };

  it("folds a run across messages into one collapsed row that expands to the calls", () => {
    const view = render(<Transcript messages={[user, first, result("r1")]} running={false} />);
    // A single call keeps its own row.
    expect(screen.getByRole("button", { name: /Read.*one\.ts/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Explored/ })).not.toBeInTheDocument();

    // A later message extends the run, so the earlier message has to re-render with the group.
    view.rerender(<Transcript messages={[user, first, result("r1"), second, result("g1"), answer]} running={false} />);
    const head = screen.getByRole("button", { name: "Explored 1 file, 1 search" });
    expect(head).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("button", { name: /one\.ts/ })).not.toBeInTheDocument();
    expect(screen.getByText("Reading first.")).toBeInTheDocument();
    expect(screen.getByText("All done.")).toBeInTheDocument();
    // The message that only held the grep renders nothing of its own.
    expect(view.container.querySelectorAll(".msg.assistant")).toHaveLength(2);

    fireEvent.click(head);
    expect(head).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("button", { name: /Read.*one\.ts/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Searched.*needle/ }));
    expect(screen.getByText("contents")).toBeInTheDocument();
  });

  it("names the call in flight while exploring, keeps its open state once saved, then shows counts", () => {
    const streaming: NormalizedMessage = { ...second, id: "streaming-3000" };
    const view = render(<Transcript messages={[user, first, result("r1")]} running partial={streaming} />);
    const head = screen.getByRole("button", { name: /^Exploring Searching needle/ });
    expect(screen.getByLabelText("Running")).toBeInTheDocument();
    fireEvent.click(head);

    // Pi saves the streamed message and starts the tool: the group stays open and live.
    view.rerender(<Transcript messages={[user, first, result("r1"), second]} running />);
    expect(screen.getByRole("button", { name: /^Exploring Searching needle/ })).toHaveAttribute("aria-expanded", "true");

    view.rerender(<Transcript messages={[user, first, result("r1"), second, { ...result("g1"), blocks: [{ type: "tool-result", toolCallId: "g1", isError: true, text: "bad" }] }]} running={false} />);
    expect(screen.getByRole("button", { name: "Explored 1 file, 1 search 1 failed" })).toHaveAttribute("aria-expanded", "true");
    expect(screen.queryByLabelText("Running")).not.toBeInTheDocument();
  });

  it("keeps every call on its own row when switched off", () => {
    render(
      <ExploreGroupingEnabled.Provider value={false}>
        <Transcript messages={[user, first, result("r1"), second, result("g1")]} running={false} />
      </ExploreGroupingEnabled.Provider>
    );
    expect(screen.queryByRole("button", { name: /Explored/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Read.*one\.ts/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Searched.*needle/ })).toBeInTheDocument();
  });
});

describe("Transcript assistant prose", () => {
  it("wraps a saved reply in .assistant-text but leaves tool rows outside it", () => {
    const assistant: NormalizedMessage = {
      id: "assistant-1",
      role: "assistant",
      blocks: [
        { type: "tool-call", toolName: "read", toolCallId: "call-1", arguments: { path: "x.ts" } },
        { type: "text", text: "Here is the answer." }
      ]
    };
    const { container } = render(<Transcript messages={[assistant]} running={false} />);
    expect(container.querySelectorAll(".assistant-text")).toHaveLength(1);
    expect(container.querySelector(".assistant-text")).toHaveTextContent("Here is the answer.");
    expect(container.querySelector(".tool-row")!.closest(".assistant-text")).toBeNull();
  });

  it("wraps streamed text the same way", () => {
    const partial: NormalizedMessage = { id: "assistant-2", role: "assistant", timestamp: 2_000, blocks: [{ type: "text", text: "Streaming" }] };
    const { container } = render(<Transcript messages={[]} running partial={partial} />);
    expect(container.querySelector(".stream-text.assistant-text")).toHaveTextContent("Streaming");
  });
});

describe("Transcript file mentions", () => {
  it("highlights path-like mentions in sent messages", () => {
    const user: NormalizedMessage = { id: "user-1", role: "user", blocks: [{ type: "text", text: "Fix @src/App.tsx for @someone." }] };
    const { container } = render(<Transcript messages={[user]} running={false} />);
    const mentions = [...container.querySelectorAll(".bubble .mention")].map((node) => node.textContent);
    expect(mentions).toEqual(["@src/App.tsx"]);
    expect(container.querySelector(".bubble")).toHaveTextContent("Fix @src/App.tsx for @someone.");
  });
});

describe("Transcript scroll rail", () => {
  const user = (id: string, text: string, timestamp?: number): NormalizedMessage =>
    ({ id, role: "user", timestamp, blocks: [{ type: "text", text }] });

  // jsdom reports 0 for every layout metric; pretend the transcript overflows.
  const overflow = async (container: HTMLElement, scrollHeight = 2_000, clientHeight = 500) => {
    const scroller = container.querySelector<HTMLElement>(".conversation-scroll")!;
    Object.defineProperty(scroller, "scrollHeight", { configurable: true, value: scrollHeight });
    Object.defineProperty(scroller, "clientHeight", { configurable: true, value: clientHeight });
    fireEvent.scroll(scroller);
    await waitFor(() => expect(container.querySelector(".scroll-rail-track")).not.toBeNull());
    return scroller;
  };

  // The rail measures on an rAF after scroll; flush one frame to settle it.
  const flushFrame = () => act(async () => { await new Promise((resolve) => requestAnimationFrame(() => resolve(undefined))); });

  it("shows the timeline and one labelled tick per user turn once the transcript overflows", async () => {
    const { container } = render(<Transcript messages={[user("u1", "First task"), user("u2", "Second task")]} running={false} />);
    expect(container.querySelector(".scroll-rail-track")).toBeNull();

    await overflow(container);
    const ticks = container.querySelectorAll<HTMLElement>(".scroll-rail-tick");
    expect(ticks).toHaveLength(2);
    expect(ticks[0]).toHaveAttribute("aria-label", "Turn 1: First task");
    expect(ticks[1]).toHaveAttribute("aria-label", "Turn 2: Second task");
    expect(screen.getByRole("navigation", { name: "Conversation timeline" })).toBeInTheDocument();
  });

  it("stops following on a gentle upward wheel while the assistant is working", async () => {
    const messages = [user("u1", "Keep working")];
    const view = render(<Transcript messages={messages} running />);
    const scroller = await overflow(view.container);
    scroller.scrollTop = 1_500;
    fireEvent.scroll(scroller);

    fireEvent.wheel(scroller, { deltaY: -4 });
    expect(screen.getByRole("button", { name: "Jump to latest" })).toHaveClass("detached");
    scroller.scrollTop = 1_496;
    fireEvent.scroll(scroller);
    Object.defineProperty(scroller, "scrollHeight", { configurable: true, value: 2_030 });
    view.rerender(<Transcript messages={messages} running activity="tool_execution_update" />);
    expect(scroller.scrollTop).toBe(1_496);
    expect(screen.getByRole("button", { name: "Jump to latest" })).toHaveClass("detached");
  });

  it("releases follow mode before a smooth timeline jump starts", async () => {
    const messages = [user("u1", "First task"), user("u2", "Second task")];
    const view = render(<Transcript messages={messages} running />);
    const scroller = await overflow(view.container);
    scroller.scrollTop = 1_500;
    fireEvent.scroll(scroller);
    await flushFrame();
    // Native smooth scrolling starts later, after the click and possibly another chunk.
    const scrollTo = vi.fn();
    Object.defineProperty(scroller, "scrollTo", { configurable: true, value: scrollTo });
    fireEvent.click(screen.getByRole("button", { name: "Turn 1: First task" }));
    expect(scrollTo).toHaveBeenCalledWith({ top: 0, behavior: "smooth" });
    expect(screen.getByRole("button", { name: "Jump to latest" })).toHaveClass("detached");
    Object.defineProperty(scroller, "scrollHeight", { configurable: true, value: 2_100 });
    view.rerender(<Transcript messages={messages} running activity="tool_execution_update" />);
    expect(scroller.scrollTop).toBe(1_500);
  });

  it("does not snap to the bottom when a scrub reverses direction nearby", async () => {
    const messages = [user("u1", "Keep working")];
    const view = render(<Transcript messages={messages} running />);
    const scroller = await overflow(view.container);
    scroller.scrollTop = 1_500;
    fireEvent.scroll(scroller);
    const rail = view.container.querySelector<HTMLElement>(".scroll-rail")!;
    Object.defineProperty(rail, "clientHeight", { configurable: true, value: 400 });
    vi.spyOn(rail, "getBoundingClientRect").mockReturnValue({ top: 0, height: 400 } as DOMRect);
    fireEvent.scroll(scroller);
    await flushFrame();

    fireEvent.pointerDown(rail, { clientY: 350 });
    fireEvent.pointerMove(rail, { clientY: 330 });
    fireEvent.scroll(scroller);
    expect(scroller.scrollTop).toBe(1_400);
    fireEvent.pointerMove(rail, { clientY: 340 });
    fireEvent.scroll(scroller);
    expect(scroller.scrollTop).toBe(1_450);
    view.rerender(<Transcript messages={messages} running activity="tool_execution_update" />);
    expect(scroller.scrollTop).toBe(1_450);
    expect(screen.getByRole("button", { name: "Jump to latest" })).toHaveClass("detached");
    fireEvent.pointerUp(rail, { clientY: 340 });
  });

  it("marks the running turn's tick live", async () => {
    const startedAt = 1_000;
    const { container } = render(<Transcript
      messages={[user("u1", "Earlier", 500), user("u2", "Now running", 2_000)]}
      running activeRun={{ startedAt }} />);
    await overflow(container);
    const ticks = container.querySelectorAll<HTMLElement>(".scroll-rail-tick");
    expect(ticks[0].className).not.toContain("live");
    expect(ticks[1].className).toContain("live");
  });

  it("jumps to a turn when its tick is clicked", async () => {
    const { container } = render(<Transcript messages={[user("u1", "First task"), user("u2", "Second task")]} running={false} />);
    const scroller = await overflow(container);
    const second = scroller.querySelectorAll<HTMLElement>("[data-turn]")[1];
    Object.defineProperty(second, "offsetTop", { configurable: true, value: 800 });
    // Turn offsets are only re-read when content size changed — bump scrollHeight to signal it.
    Object.defineProperty(scroller, "scrollHeight", { configurable: true, value: 2_100 });
    const scrollTo = vi.fn();
    Object.defineProperty(scroller, "scrollTo", { configurable: true, value: scrollTo });
    fireEvent.scroll(scroller);
    await flushFrame();
    fireEvent.click(screen.getByRole("button", { name: "Turn 2: Second task" }));
    expect(scrollTo).toHaveBeenCalledWith({ top: 780, behavior: "smooth" });
  });

  it("scrubs proportionally when the rail is pressed", async () => {
    const { container } = render(<Transcript messages={[user("u1", "First task"), user("u2", "Second task")]} running={false} />);
    const scroller = await overflow(container);
    const rail = container.querySelector<HTMLElement>(".scroll-rail")!;
    Object.defineProperty(rail, "clientHeight", { configurable: true, value: 400 });
    fireEvent.scroll(scroller);
    await flushFrame();
    // A press that never moves is a click: smooth jump to halfway down the rail.
    const scrollTo = vi.fn();
    Object.defineProperty(scroller, "scrollTo", { configurable: true, value: scrollTo });
    vi.spyOn(rail, "getBoundingClientRect").mockReturnValue({ top: 0, height: 400 } as DOMRect);
    fireEvent.pointerDown(rail, { clientY: 200 });
    fireEvent.pointerUp(rail, { clientY: 200 });
    expect(scrollTo).toHaveBeenCalledWith({ top: 750, behavior: "smooth" });
  });
});

describe("Transcript attached images", () => {
  const attached = (): NormalizedMessage => ({
    id: "u1", entryId: "entry-1", role: "user",
    blocks: [
      { type: "image", imageId: "image-1", mimeType: "image/png", thumbnail: "data:image/png;base64,dGh1bWIx" },
      { type: "image", imageId: "image-2", mimeType: "image/png", thumbnail: "data:image/png;base64,dGh1bWIy" },
      { type: "text", text: "Look at these" }
    ]
  });

  it("opens a sent image full size in the lightbox, and Escape closes it", async () => {
    const loadImage = vi.fn(async () => "data:image/png;base64,b3JpZ2luYWw=");
    render(<Transcript messages={[attached()]} running={false} loadImage={loadImage} />);
    fireEvent.click(screen.getByRole("button", { name: "Open attached image 2" }));
    const dialog = screen.getByRole("dialog", { name: "Attached image 2" });
    await waitFor(() => expect(dialog.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,b3JpZ2luYWw="));
    // The original is fetched for the exact message and position that were clicked.
    expect(loadImage).toHaveBeenCalledWith("entry-1", 1);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("opens the preview alone when no original can come back", () => {
    render(<Transcript messages={[attached()]} running={false} />);
    fireEvent.click(screen.getByRole("button", { name: "Open attached image 1" }));
    expect(screen.getByRole("dialog", { name: "Attached image 1" }).querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,dGh1bWIx");
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("opens a kept image from the edit box too", async () => {
    const loadImage = vi.fn(async () => "data:image/png;base64,b3JpZ2luYWw=");
    render(<Transcript messages={[attached()]} running={false} actionsEnabled onMessageAction={vi.fn().mockResolvedValue(false)} loadImage={loadImage} />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.click(screen.getByRole("button", { name: "Open attached image 1" }));
    const dialog = screen.getByRole("dialog", { name: "Attached image 1" });
    await waitFor(() => expect(dialog.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,b3JpZ2luYWw="));
    expect(loadImage).toHaveBeenCalledWith("entry-1", 0);
  });
});
