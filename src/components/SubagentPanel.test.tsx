import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NormalizedMessage, SubagentDetails, SubagentResult, SubagentView } from "../types";
import { SubagentPanel, siblingLabels } from "./SubagentPanel";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function child(overrides: Partial<SubagentResult> = {}): SubagentResult {
  return {
    agent: "scout",
    task: "Find where sessions are restored",
    readOnly: true,
    status: "running",
    activity: [],
    usage: { input: 1_200, output: 300, cacheRead: 0, cacheWrite: 0, cost: 0.0123, turns: 3 },
    ...overrides
  };
}

const details = (results: SubagentResult[]): SubagentDetails => ({ v: 1, mode: results.length > 1 ? "parallel" : "single", results });

function stream(overrides: Partial<SubagentView> = {}): SubagentView {
  return { toolCallId: "call-1", index: 0, rev: 2, messages: [], live: true, truncated: false, missing: false, loading: false, ...overrides };
}

const answer: NormalizedMessage = { id: "assistant-2", role: "assistant", blocks: [{ type: "text", text: "Sessions are restored in session.ts." }] };

function panel(props: Partial<Parameters<typeof SubagentPanel>[0]> = {}) {
  const handlers = { onSelect: vi.fn(), onClose: vi.fn(), onRetry: vi.fn() };
  render(<SubagentPanel toolCallId="call-1" index={0} details={details([child()])} live stream={stream()} {...handlers} {...props} />);
  return handlers;
}

describe("SubagentPanel", () => {
  it("shows who it is, what it was asked, and its transcript as it works", () => {
    const { onClose } = panel({
      details: details([child({ model: "OpenAI · Small", startedAt: Date.now() - 5_000 })]),
      stream: stream({ messages: [answer] })
    });
    expect(screen.getByRole("heading", { name: "Scout" })).toBeInTheDocument();
    expect(screen.getByText("SubAgent")).toBeInTheDocument();
    expect(screen.getByText("Working")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Brief" })).getByText("Find where sessions are restored")).toBeInTheDocument();
    expect(screen.getByText("Brief from WackCode")).toBeInTheDocument();
    expect(screen.getByText("Sessions are restored in session.ts.")).toBeInTheDocument();
    expect(screen.getByText("OpenAI · Small")).toBeInTheDocument();
    expect(screen.getByText("3 turns")).toBeInTheDocument();
    expect(screen.getByText("↑1.2k ↓300")).toBeInTheDocument();
    expect(screen.getByText("$0.01")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Close SubAgent panel" }));
    expect(onClose).toHaveBeenCalled();
  });

  it("ticks live thinking and stops an unfinished timer with the watched stream", () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    const partial: NormalizedMessage = { id: "partial", role: "assistant", blocks: [{ type: "thinking", text: "Unfinished", startedAt: 8_000 }] };
    const props = { toolCallId: "call-1", index: 0, details: details([child()]), onSelect: vi.fn(), onClose: vi.fn(), onRetry: vi.fn() };
    const view = render(<SubagentPanel {...props} live stream={stream({ partial })} />);
    expect(screen.getByRole("button", { name: "Thinking… 2s" })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(3_000));
    expect(screen.getByRole("button", { name: "Thinking… 5s" })).toBeInTheDocument();
    view.rerender(<SubagentPanel {...props} live={false} stream={stream({ partial, live: false })} />);
    expect(screen.getByRole("button", { name: "Reasoning" })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(3_000));
    expect(screen.queryByText("Thinking…")).not.toBeInTheDocument();
  });

  it("waits in the queue, then warms up before its first step", () => {
    panel({ details: details([child({ status: "queued" })]) });
    expect(screen.getByText("Waiting for a free slot")).toBeInTheDocument();
    cleanup();
    panel();
    expect(screen.getByText("Warming up…")).toBeInTheDocument();
    cleanup();
    panel({ stream: stream({ loading: true }) });
    expect(screen.getByRole("status", { name: "Loading transcript" })).toBeInTheDocument();
  });

  it("tells a finished child's ending, and shows what an old call reported without a transcript", () => {
    panel({
      live: false,
      details: details([child({ status: "done", output: "## Found\n\nIn `restore.ts`.", activity: [{ tool: "grep", subject: "restore" }], startedAt: 1_000, endedAt: 13_000 })]),
      stream: stream({ live: false, missing: true })
    });
    expect(screen.getByText("Done")).toBeInTheDocument();
    expect(screen.getByText("Returned to WackCode")).toBeInTheDocument();
    expect(screen.getByText(/No transcript was saved for this SubAgent/)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Found" })).toBeInTheDocument();
    expect(within(screen.getByRole("list", { name: "Recent tool calls" })).getByText("restore")).toBeInTheDocument();
    expect(screen.getAllByText("12s").length).toBeGreaterThan(0);
  });

  it("shows why a child failed, and notes a trimmed transcript", () => {
    panel({
      live: false,
      details: details([child({ status: "failed", error: "Scout's model isn't connected." })]),
      stream: stream({ live: false, truncated: true, messages: [answer] })
    });
    expect(screen.getByText("Failed")).toBeInTheDocument();
    expect(screen.getByText("Scout's model isn't connected.")).toBeInTheDocument();
    expect(screen.getByText(/Older tool output was trimmed/)).toBeInTheDocument();
  });

  it("moves between a parallel call's children with tabs", () => {
    const { onSelect } = panel({ details: details([child(), child({ agent: "code-reviewer", status: "queued" })]) });
    const tabs = within(screen.getByRole("tablist", { name: "SubAgents in this call" })).getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual(["Scout", "Code Reviewer"]);
    expect(tabs[0]).toHaveAttribute("aria-selected", "true");
    fireEvent.click(tabs[1]);
    expect(onSelect).toHaveBeenCalledWith(1);
  });

  it("numbers a role that appears more than once", () => {
    expect(siblingLabels([child(), child({ agent: "worker" }), child()])).toEqual(["Scout 1", "Worker", "Scout 2"]);
  });

  it("offers to try again when it couldn't follow the child", () => {
    const { onRetry } = panel({ stream: stream({ error: "Pi did not answer in time." }) });
    expect(screen.getByText("Pi did not answer in time.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalled();
  });

  it("keeps completed child transcripts detailed instead of folding their work", () => {
    const prompt: NormalizedMessage = { id: "u", role: "user", blocks: [{ type: "text", text: "Look around" }] };
    const reasoning: NormalizedMessage = { id: "work", role: "assistant", blocks: [{ type: "thinking", text: "Inspecting" }, { type: "text", text: "Reading the files" }] };
    panel({ live: false, details: details([child({ status: "done" })]), stream: stream({ live: false, messages: [prompt, reasoning, answer] }) });
    expect(screen.getByText("Reading the files")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reasoning" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /work transcript/ })).not.toBeInTheDocument();
  });

  it("ignores a transcript that belongs to another child", () => {
    panel({ stream: stream({ index: 1, messages: [answer] }) });
    expect(screen.queryByText("Sessions are restored in session.ts.")).not.toBeInTheDocument();
    expect(screen.getByRole("status", { name: "Loading transcript" })).toBeInTheDocument();
  });
});
