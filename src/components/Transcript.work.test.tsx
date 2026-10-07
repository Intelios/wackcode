import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NormalizedMessage } from "../types";
import { ExploreGroupingEnabled } from "./ExploreGroup";
import { SubagentPanelLink } from "./SubagentChip";
import { Transcript } from "./Transcript";

const user: NormalizedMessage = { id: "u", entryId: "u", role: "user", timestamp: 100, blocks: [{ type: "text", text: "Fix it" }] };
const progress: NormalizedMessage = { id: "p", role: "assistant", timestamp: 200, stopReason: "toolUse", blocks: [
  { type: "text", text: "Checking the change" },
  { type: "tool-call", toolName: "edit", toolCallId: "edit-1", arguments: { path: "src/a.ts", oldText: "old", newText: "new" } }
] };
const result: NormalizedMessage = { id: "r", role: "tool", blocks: [
  { type: "tool-result", toolCallId: "edit-1", text: "Edited", details: { diff: "-old\n+new" } }
] };
const answer: NormalizedMessage = { id: "a", entryId: "a", role: "assistant", timestamp: 300, stopReason: "stop", blocks: [
  { type: "thinking", text: "Checked the tests", durationMs: 1_000 }, { type: "text", text: "Done. Tests pass." }
], turn: { userEntryId: "u", endEntryId: "a" } };
const messages = [user, progress, result, answer];
const timings = [{ userMessageId: "u", durationMs: 62_000 }];
const reveal = () => screen.getByRole("button", { name: /Show work transcript/ });
const hide = () => screen.getByRole("button", { name: /Hide work transcript/ });

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("Transcript completed work", () => {
  it("defaults to a timed disclosure with only the user and final answer visible, then reveals original details", () => {
    const { container } = render(<Transcript messages={messages} running={false} runTimings={timings} />);
    const button = reveal();
    expect(button).toHaveAccessibleName("Worked for 1m 2s. Show work transcript");
    expect(button).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByText("Fix it")).toBeInTheDocument();
    expect(screen.getByText("Done. Tests pass.")).toBeInTheDocument();
    expect(screen.queryByText("Checking the change")).not.toBeInTheDocument();
    expect(container.querySelector(".tool-row")).toBeNull();
    expect(container.querySelector(".thinking-row")).toBeNull();
    expect(document.getElementById(button.getAttribute("aria-controls")!)).toHaveAttribute("aria-hidden", "true");

    fireEvent.click(button);
    expect(hide()).toHaveAttribute("aria-expanded", "true");
    const region = screen.getByRole("region", { name: "Work transcript" });
    expect(within(region).getByText("Checking the change")).toBeInTheDocument();
    fireEvent.click(within(region).getByRole("button", { name: /Edited.*a\.ts/ }));
    expect(region.querySelector("pre.tool-diff")).toHaveTextContent("+new");
    fireEvent.click(within(region).getByRole("button", { name: "Thought for 1s" }));
    expect(within(region).getByText("Checked the tests")).toBeInTheDocument();
    expect(region.compareDocumentPosition(screen.getByText("Done. Tests pass.")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    fireEvent.click(hide());
    expect(reveal()).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("+new")).not.toBeInTheDocument();
    expect(container.querySelector(".tool-row")).toBeNull();
  });

  it("stays fully live through final text and stopping, then always collapses despite inspection", () => {
    const view = render(<Transcript messages={messages} running activeRun={{ startedAt: 90 }} status="running" />);
    expect(screen.queryByRole("button", { name: /work transcript/ })).not.toBeInTheDocument();
    expect(screen.getByText(/Working for/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Edited.*a\.ts/ }));
    const thought = screen.getByRole("button", { name: "Thought for 1s" });
    fireEvent.click(thought);
    act(() => thought.focus());
    view.rerender(<Transcript messages={messages} running activeRun={{ startedAt: 90 }} status="stopping" />);
    expect(view.container.querySelector("pre.tool-diff")).toHaveTextContent("+new");
    view.rerender(<Transcript messages={messages} running={false} status="idle" runTimings={timings} />);
    expect(reveal()).toHaveAttribute("aria-expanded", "false");
    expect(reveal()).toHaveFocus();
    expect(screen.queryByText("+new")).not.toBeInTheDocument();
    expect(screen.queryByText("Checked the tests")).not.toBeInTheDocument();
    fireEvent.click(reveal());
    // The existing reasoning expansion context survives the outer fold.
    expect(screen.getByRole("button", { name: "Thought for 1s" })).toHaveAttribute("aria-expanded", "true");
  });

  it("completion overrides even a previously revealed turn without changing other manual choices", () => {
    const view = render(<Transcript messages={messages} running={false} />);
    fireEvent.click(reveal());
    expect(hide()).toHaveAttribute("aria-expanded", "true");
    view.rerender(<Transcript messages={messages} running activeRun={{ startedAt: 90 }} />);
    expect(screen.queryByRole("button", { name: /work transcript/ })).not.toBeInTheDocument();
    view.rerender(<Transcript messages={messages} running={false} />);
    expect(reveal()).toHaveAttribute("aria-expanded", "false");
  });

  it("closes a work-owned screenshot portal and recovers its focus on automatic collapse", () => {
    const call: NormalizedMessage = { id: "shot", role: "assistant", stopReason: "toolUse", blocks: [
      { type: "tool-call", toolName: "browser_screenshot", toolCallId: "shot", arguments: {} }
    ] };
    const shot: NormalizedMessage = { id: "shot-result", role: "tool", blocks: [
      { type: "tool-result", toolName: "browser_screenshot", toolCallId: "shot", text: "Screenshot", images: [{ imageId: "img", thumbnail: "data:image/png;base64,aA==" }] }
    ] };
    const view = render(<Transcript messages={[user, call, shot]} running activeRun={{ startedAt: 90 }} />);
    fireEvent.click(screen.getByRole("button", { name: /Open screenshot/ }));
    expect(screen.getByRole("button", { name: "Close" })).toHaveFocus();
    view.rerender(<Transcript messages={[user, call, shot, answer]} running={false} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(reveal()).toHaveFocus();
  });

  it("does not steal focus from the composer/outside the work", () => {
    const view = render(<><input aria-label="Composer" /><Transcript messages={messages} running activeRun={{ startedAt: 90 }} /></>);
    const composer = screen.getByRole("textbox", { name: "Composer" });
    act(() => composer.focus());
    view.rerender(<><input aria-label="Composer" /><Transcript messages={messages} running={false} /></>);
    expect(composer).toHaveFocus();
    expect(reveal()).toHaveAttribute("aria-expanded", "false");
  });

  it("supports legacy View/Hide work labels, settings opt-out, and restoring manual choices", () => {
    const view = render(<Transcript messages={messages} running={false} />);
    expect(reveal()).toHaveAccessibleName("View work. Show work transcript");
    fireEvent.click(reveal());
    expect(hide()).toHaveAccessibleName("Hide work. Hide work transcript");
    view.rerender(<Transcript messages={messages} running={false} collapseCompletedWork={false} />);
    expect(screen.queryByRole("button", { name: /work transcript/ })).not.toBeInTheDocument();
    expect(screen.getByText("Checking the change")).toBeInTheDocument();
    view.rerender(<Transcript messages={messages} running={false} />);
    expect(hide()).toHaveAttribute("aria-expanded", "true");
  });

  it("isolates expansion between chats and answer versions while retaining choices on return", () => {
    const view = render(<Transcript messages={messages} running={false} scopeKey="one" />);
    fireEvent.click(reveal());
    view.rerender(<Transcript messages={messages} running={false} scopeKey="two" />);
    expect(reveal()).toHaveAttribute("aria-expanded", "false");
    view.rerender(<Transcript messages={messages} running={false} scopeKey="one" />);
    expect(hide()).toHaveAttribute("aria-expanded", "true");
    view.rerender(<Transcript messages={[user, progress, result, { ...answer, id: "a2", entryId: "a2" }]} running={false} scopeKey="one" />);
    expect(reveal()).toHaveAttribute("aria-expanded", "false");
  });

  it("keeps only one terminal action footer and original action targets when work is split", () => {
    const onMessageAction = vi.fn();
    render(<Transcript messages={messages} running={false} actionsEnabled onMessageAction={onMessageAction} />);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onMessageAction).toHaveBeenLastCalledWith({ type: "retry", message: answer });
    fireEvent.click(screen.getByRole("button", { name: "Fork from here" }));
    expect(onMessageAction).toHaveBeenLastCalledWith({ type: "fork", message: answer });
    fireEvent.click(reveal());
    expect(screen.getAllByRole("button", { name: "Retry" })).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: "Fork from here" })).toHaveLength(1);
  });

  it("keeps plan-only outcomes and their review actions visible while folding exploration", () => {
    const plan = "# Ready plan\n\nImplement it.";
    const proposal: NormalizedMessage = { ...answer, blocks: [
      { type: "thinking", text: "Preparing the proposal" },
      { type: "tool-call", toolName: "plan_mode_complete", toolCallId: "plan", arguments: { plan } }
    ] };
    const planResult: NormalizedMessage = { id: "plan-result", role: "tool", blocks: [{ type: "tool-result", toolCallId: "plan", details: { plan }, text: plan }] };
    const onPlanAction = vi.fn();
    render(<Transcript messages={[user, progress, result, proposal, planResult]} running={false}
      planState={{ mode: "plan", phase: "ready", plan }} onPlanAction={onPlanAction} />);
    expect(reveal()).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByRole("heading", { name: "Ready plan" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Approve & implement" }));
    expect(onPlanAction).toHaveBeenCalled();
    expect(screen.queryByText("Preparing the proposal")).not.toBeInTheDocument();
  });

  it("leaves stopped, interrupted, delayed/partial and no-outcome work inspectable", () => {
    const view = render(<Transcript messages={[user, progress, result, { ...answer, stopReason: "aborted" }]} running={false} />);
    expect(screen.getByText("Stopped")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /work transcript/ })).not.toBeInTheDocument();
    view.rerender(<Transcript messages={messages} running={false} status="interrupted" />);
    expect(screen.getByText("Checking the change")).toBeInTheDocument();
    view.rerender(<Transcript messages={messages} running={false} partial={{ ...answer, id: "partial" }} />);
    expect(screen.queryByRole("button", { name: /work transcript/ })).not.toBeInTheDocument();
    view.rerender(<Transcript messages={[user, progress, result]} running={false} />);
    expect(screen.getByRole("button", { name: /Edited.*a\.ts/ })).toBeInTheDocument();
  });

  it("preserves exploration, orphan/switch ordering and trailing model switches on reveal", () => {
    const reads: NormalizedMessage = { id: "reads", role: "assistant", blocks: [
      { type: "tool-call", toolName: "read", toolCallId: "r1", arguments: { path: "one.ts" } },
      { type: "tool-call", toolName: "grep", toolCallId: "r2", arguments: { pattern: "needle" } }
    ] };
    const results: NormalizedMessage = { id: "read-results", role: "tool", blocks: [
      { type: "tool-result", toolCallId: "r1", text: "Contents" }, { type: "tool-result", toolCallId: "r2", text: "Matches" },
      { type: "tool-result", toolName: "mystery", toolCallId: "orphan", text: "Orphan output" }
    ] };
    render(<Transcript messages={[user, reads, results, answer]} running={false} modelSwitches={[
      { id: "inside", at: 2, from: "First", to: "Second" }, { id: "after", at: 4, from: "Second", to: "Third" }
    ]} />);
    expect(screen.queryByText(/First/)).not.toBeInTheDocument();
    expect(screen.getByText("Third")).toBeInTheDocument();
    fireEvent.click(reveal());
    const region = screen.getByRole("region", { name: "Work transcript" });
    expect(within(region).getByRole("button", { name: "Explored 1 file, 1 search" })).toBeInTheDocument();
    expect(within(region).getByText("First")).toBeInTheDocument();
    fireEvent.click(within(region).getByRole("button", { name: /mystery result/ }));
    expect(within(region).getByText("Orphan output")).toBeInTheDocument();
  });

  it("does not fold earlier turns from the same run after a steering prompt arrives", () => {
    const steer: NormalizedMessage = { ...user, id: "steer", entryId: "steer", timestamp: 250, blocks: [{ type: "text", text: "Use another approach" }] };
    const final: NormalizedMessage = { ...answer, id: "final", entryId: "final", turn: { userEntryId: "steer", endEntryId: "final" } };
    const view = render(<Transcript messages={[...messages, steer, { ...progress, id: "more" }, final]} running activeRun={{ startedAt: 90 }} />);
    expect(screen.queryByRole("button", { name: /work transcript/ })).not.toBeInTheDocument();
    expect(screen.getAllByText("Checking the change")).toHaveLength(2);
    view.rerender(<Transcript messages={[...messages, steer, { ...progress, id: "more" }, final]} running={false} />);
    expect(screen.getAllByRole("button", { name: /Show work transcript/ })).toHaveLength(2);
  });

  it("can still open sub-agent chips after revealing completed work", () => {
    const onOpen = vi.fn();
    const subCall: NormalizedMessage = { id: "sub", role: "assistant", blocks: [{ type: "tool-call", toolName: "subagent", toolCallId: "sub", arguments: { agent: "scout", task: "Look" } }] };
    const subResult: NormalizedMessage = { id: "sub-result", role: "tool", blocks: [{ type: "tool-result", toolCallId: "sub", details: { v: 1, mode: "single", results: [{ agent: "scout", task: "Look", status: "done", readOnly: true, activity: [], output: "Mapped", usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 1 } }] } }] };
    render(<SubagentPanelLink.Provider value={{ onOpen }}><Transcript messages={[user, subCall, subResult, answer]} running={false} /></SubagentPanelLink.Provider>);
    fireEvent.click(reveal());
    fireEvent.click(screen.getByRole("button", { name: /^SubAgent Scout/ }));
    expect(onOpen).toHaveBeenCalledWith("sub", 0);
  });

  it("does not add a disclosure to a direct answer, and honors independent exploration opt-out", () => {
    const view = render(<Transcript messages={[user, { ...answer, blocks: [answer.blocks[1]] }]} running={false} runTimings={timings} />);
    expect(screen.queryByRole("button", { name: /work transcript/ })).not.toBeInTheDocument();
    expect(screen.getByText(/Worked for/)).toBeInTheDocument();
    view.rerender(<ExploreGroupingEnabled.Provider value={false}><Transcript messages={messages} running={false} /></ExploreGroupingEnabled.Provider>);
    fireEvent.click(reveal());
    expect(screen.getByRole("button", { name: /Edited.*a\.ts/ })).toBeInTheDocument();
  });

  it("supports keyboard activation and an immediate reduced-motion end state", () => {
    vi.stubGlobal("matchMedia", vi.fn((query: string) => ({ matches: query.includes("reduced-motion"), addListener: vi.fn(), removeListener: vi.fn() })));
    render(<Transcript messages={messages} running={false} />);
    const button = reveal();
    act(() => button.focus());
    // Native buttons dispatch click for Enter/Space; verify the button is not a custom role.
    expect(button.tagName).toBe("BUTTON");
    fireEvent.click(button);
    expect(screen.getByRole("region", { name: "Work transcript" })).toBeInTheDocument();
    expect(button).toHaveFocus();
    fireEvent.click(button);
    expect(screen.queryByText("Checking the change")).not.toBeInTheDocument();
  });

  it("keeps a background child accessible after parent settlement with reduced motion", () => {
    vi.stubGlobal("matchMedia", vi.fn((query: string) => ({ matches: query.includes("reduced-motion"), addListener: vi.fn(), removeListener: vi.fn() })));
    const onOpen = vi.fn();
    const launch: NormalizedMessage = { id: "launch", role: "assistant", blocks: [{ type: "tool-call", toolCallId: "bg", toolName: "subagent", arguments: { agent: "scout", task: "Inspect sessions", background: true } }] };
    const started: NormalizedMessage = { id: "started", role: "tool", blocks: [{ type: "tool-result", toolCallId: "bg", toolName: "subagent", details: { v: 1, mode: "single", background: true, results: [{ jobId: "job", agent: "scout", task: "Inspect sessions", readOnly: true, status: "running", activity: [], usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 0 } }] } }] };
    render(<SubagentPanelLink.Provider value={{ onOpen }}><Transcript messages={[user, launch, started, answer]} running={false} /></SubagentPanelLink.Provider>);
    expect(screen.queryByRole("button", { name: /Show work transcript/ })).not.toBeInTheDocument();
    const chip = screen.getByRole("button", { name: /SubAgent Scout, working/ });
    act(() => chip.focus());
    expect(chip).toHaveFocus();
    expect(chip.tagName).toBe("BUTTON");
    fireEvent.click(chip);
    expect(onOpen).toHaveBeenCalledWith("bg", 0);
    expect(screen.getByText("Done. Tests pass.")).toBeInTheDocument();
  });
});

function scrollerGeometry(container: HTMLElement, readingAnswer = false) {
  const el = container.querySelector<HTMLElement>(".conversation-scroll")!;
  let top = 0;
  const height = () => container.querySelector(".work-disclosure") ? 1_200 : 2_400;
  Object.defineProperties(el, {
    clientHeight: { configurable: true, value: 500 },
    scrollHeight: { configurable: true, get: height },
    scrollTop: { configurable: true, get: () => Math.min(top, height() - 500), set: (value: number) => { top = Math.max(0, Math.min(value, height() - 500)); } }
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    if (this === el) return { top: 0, bottom: 500, height: 500 } as DOMRect;
    const key = this.dataset.transcriptAnchor;
    const y = readingAnswer && key === "message:a:outcome" ? (container.querySelector(".work-disclosure") ? 600 : 1_800)
      : readingAnswer && key === "message:a:work" ? 1_700
      : key?.startsWith("work:") ? 100 : key === "message:u" ? 0 : key?.startsWith("message:p") ? 100 : 1_800;
    const h = readingAnswer && key === "message:a:outcome" ? 300 : key?.startsWith("message:p") ? 1_600 : 100;
    return { top: y - el.scrollTop, bottom: y - el.scrollTop + h, height: h } as DOMRect;
  });
  return el;
}

describe("completed work scroll transitions", () => {
  it("always folds while detached and anchors to the disclosure when the reading row disappears", async () => {
    const view = render(<Transcript messages={messages} running activeRun={{ startedAt: 90 }} />);
    const el = scrollerGeometry(view.container);
    el.scrollTop = 1_900;
    fireEvent.scroll(el);
    fireEvent.wheel(el, { deltaY: -5 });
    el.scrollTop = 500;
    fireEvent.scroll(el);
    view.rerender(<Transcript messages={messages} running={false} />);
    expect(reveal()).toHaveAttribute("aria-expanded", "false");
    expect(el.scrollTop).toBe(100);
    await waitFor(() => expect(screen.getByRole("button", { name: "Jump to latest" })).toHaveClass("detached"));
  });

  it.each([false, true])("anchors the inspected disclosure across steering folds (staggered: %s)", (staggered) => {
    vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, addListener: vi.fn(), removeListener: vi.fn() })));
    const steer: NormalizedMessage = { ...user, id: "steer", entryId: "steer", timestamp: 400 };
    const nextProgress: NormalizedMessage = { ...progress, id: "p2", timestamp: 500, blocks: [
      progress.blocks[0], { ...progress.blocks[1], toolCallId: "edit-2" }
    ] };
    const nextResult: NormalizedMessage = { ...result, id: "r2", blocks: [{ ...result.blocks[0], toolCallId: "edit-2" }] };
    const nextAnswer: NormalizedMessage = { ...answer, id: "a2", entryId: "a2", timestamp: 600, turn: { userEntryId: "steer", endEntryId: "a2" } };
    const history = [...messages, steer, nextProgress, nextResult, nextAnswer];
    const view = render(<Transcript messages={history} running activeRun={{ startedAt: 90 }} />);
    const el = scrollerGeometry(view.container);
    el.scrollTop = 1_900;
    fireEvent.scroll(el);
    fireEvent.wheel(el, { deltaY: -4 });
    el.scrollTop = 500;
    fireEvent.scroll(el);
    if (staggered) {
      view.rerender(<Transcript messages={history.slice(0, -2)} running={false} />);
      expect(screen.getAllByRole("button", { name: /Show work transcript/ })).toHaveLength(1);
      expect(el.scrollTop).toBe(100);
    }
    view.rerender(<Transcript messages={history} running={false} />);
    expect(screen.getAllByRole("button", { name: /Show work transcript/ })).toHaveLength(2);
    // The earlier work is still mounted during its exit; the later turn must not anchor it.
    expect(view.container.querySelectorAll('.work-transcript-region[aria-hidden="true"] .msg').length).toBeGreaterThan(0);
    expect(el.scrollTop).toBe(100);
  });

  it("gives work and outcome slices distinct anchors before a detached completion", () => {
    const view = render(<Transcript messages={messages} running activeRun={{ startedAt: 90 }} />);
    const el = scrollerGeometry(view.container, true);
    const all = [...el.querySelectorAll<HTMLElement>("[data-transcript-anchor]")];
    expect(new Set(all.map((node) => node.dataset.transcriptAnchor)).size).toBe(all.length);
    const outcome = el.querySelector<HTMLElement>('[data-transcript-anchor="message:a:outcome"]')!;
    // The reasoning slice is above the viewport; the answer, not reasoning, is being read.
    el.scrollTop = 1_900;
    fireEvent.scroll(el);
    fireEvent.wheel(el, { deltaY: -4 });
    el.scrollTop = 1_850;
    fireEvent.scroll(el);
    view.rerender(<Transcript messages={messages} running={false} />);
    expect(outcome).toBeInTheDocument();
    expect(el.scrollTop).toBe(650);
    expect(outcome.getBoundingClientRect().top).toBe(-50);
  });

  it("holds the answer through animation even when completion resets an earlier manual reveal", () => {
    const view = render(<Transcript messages={messages} running={false} />);
    fireEvent.click(reveal());
    view.rerender(<Transcript messages={messages} running activeRun={{ startedAt: 90 }} />);
    const el = scrollerGeometry(view.container, true);
    const outcome = el.querySelector<HTMLElement>('[data-transcript-anchor="message:a:outcome"]')!;
    const originalRect = outcome.getBoundingClientRect.bind(outcome);
    let animationDrift = 0;
    Object.defineProperty(outcome, "getBoundingClientRect", { configurable: true, value: () => {
      const rect = originalRect();
      return { ...rect, top: rect.top + animationDrift, bottom: rect.bottom + animationDrift };
    } });
    el.scrollTop = 1_900;
    fireEvent.scroll(el);
    fireEvent.wheel(el, { deltaY: -4 });
    el.scrollTop = 1_850;
    fireEvent.scroll(el);
    view.rerender(<Transcript messages={messages} running={false} />);
    expect(reveal()).toHaveAttribute("aria-expanded", "false");
    expect(outcome.getBoundingClientRect().top).toBe(-50);
    animationDrift = 40;
    view.rerender(<Transcript messages={messages} running={false} />);
    expect(outcome.getBoundingClientRect().top).toBe(-50);
  });

  it("keeps bottom-following through automatic shrink and does not snap on manual reveal", async () => {
    const view = render(<Transcript messages={messages} running activeRun={{ startedAt: 90 }} />);
    const el = scrollerGeometry(view.container);
    view.rerender(<Transcript messages={messages} running activeRun={{ startedAt: 90 }} />);
    expect(el.scrollTop).toBe(1_900);
    view.rerender(<Transcript messages={messages} running={false} />);
    fireEvent.scroll(el);
    expect(el.scrollTop).toBe(700);
    await waitFor(() => expect(screen.getByRole("button", { name: "Jump to latest" })).not.toHaveClass("detached"));
    fireEvent.click(reveal());
    expect(el.scrollTop).toBe(700);
    expect(screen.getByRole("button", { name: "Jump to latest" })).toHaveClass("detached");
  });
});
