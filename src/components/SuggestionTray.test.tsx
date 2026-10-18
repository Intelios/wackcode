import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SuggestionCard } from "../suggestions";
import { SuggestionNote, SuggestionScopeContext, SuggestionTray, type SuggestionScope } from "./SuggestionTray";

afterEach(cleanup);

const brief = "**Where:** `src/components/Composer.tsx:88`\n\nThe stop button disables itself after a failed save, so the chat can never be stopped again.";
const bug: SuggestionCard = { id: "call-1", kind: "bug", title: "Dismiss stays disabled after a failed save", description: brief };
const polish: SuggestionCard = { id: "call-2", kind: "polish", title: "The archived list has no empty state", description: "A short one." };

const scope = (patch: Partial<SuggestionScope> = {}): SuggestionScope => ({
  links: new Map(),
  started: new Set(),
  dismissed: new Set(),
  busy: false,
  canStartNewChat: true,
  ...patch
});

function renderTray(cards: SuggestionCard[], patch: Partial<SuggestionScope> = {}, onAction = vi.fn()) {
  render(
    <SuggestionScopeContext.Provider value={scope({ ...patch, onAction })}>
      <SuggestionTray suggestions={cards} />
    </SuggestionScopeContext.Provider>
  );
  return onAction;
}

describe("SuggestionTray", () => {
  it("draws each suggestion with its kind, title and brief", () => {
    renderTray([bug, polish]);
    expect(screen.getByRole("heading", { name: "Spotted along the way" })).toBeInTheDocument();
    expect(screen.getByText("2 follow-ups the agent left for later.")).toBeInTheDocument();
    expect(screen.getByText("Dismiss stays disabled after a failed save")).toBeInTheDocument();
    expect(screen.getByText("Bug")).toBeInTheDocument();
    expect(screen.getByText("Polish")).toBeInTheDocument();
    expect(document.querySelectorAll(".suggestion-item.kind-bug")).toHaveLength(1);
    expect(screen.getByText(/The stop button disables itself/)).toBeInTheDocument();
  });

  it("says one when there is one", () => {
    renderTray([bug]);
    expect(screen.getByText("One follow-up the agent left for later.")).toBeInTheDocument();
  });

  it("starts in a new chat or here in Build", () => {
    const onAction = renderTray([bug]);
    fireEvent.click(screen.getByRole("button", { name: /Start in new chat/ }));
    expect(onAction).toHaveBeenCalledWith({ type: "start", card: bug, target: "new", mode: "build" });

    cleanup();
    const here = renderTray([bug]);
    fireEvent.click(screen.getByRole("button", { name: /Start here/ }));
    expect(here).toHaveBeenCalledWith({ type: "start", card: bug, target: "here", mode: "build" });
  });

  it("offers Plan and Ultra Plan in its menu", () => {
    const onAction = renderTray([bug]);
    fireEvent.click(screen.getByRole("button", { name: /More ways to start “Dismiss stays disabled after a failed save”/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Plan it in a new chat" }));
    expect(onAction).toHaveBeenCalledWith({ type: "start", card: bug, target: "new", mode: "plan" });

    cleanup();
    const here = renderTray([bug]);
    fireEvent.click(screen.getByRole("button", { name: /More ways to start/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Ultra Plan it here" }));
    expect(here).toHaveBeenCalledWith({ type: "start", card: bug, target: "here", mode: "ultraplan" });
  });

  it("presses a Start once, so a double click cannot open two chats", () => {
    const onAction = renderTray([bug]);
    const start = screen.getByRole("button", { name: /Start in new chat/ });
    fireEvent.click(start);
    expect(start).toBeDisabled();
    fireEvent.click(start);
    expect(onAction).toHaveBeenCalledTimes(1);
  });

  it("disables starting here while a run is in flight, and copies the brief on request", () => {
    const onAction = renderTray([bug], { busy: true });
    expect(screen.getByRole("button", { name: /Start here/ })).toBeDisabled();
    // A new chat is independent of this run, so it stays offered.
    expect(screen.getByRole("button", { name: /Start in new chat/ })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: /More ways/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: /Copy as prompt/ }));
    expect(onAction).toHaveBeenCalledWith({ type: "copy", card: bug });
  });

  it("does not offer a new chat where there is no project to put it in", () => {
    renderTray([bug], { canStartNewChat: false });
    expect(screen.queryByRole("button", { name: /Start in new chat/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Start here/ })).toHaveClass("primary-button");
  });

  it("refuses starting here when the chat's model is gone", () => {
    renderTray([bug], { modelMissing: true });
    expect(screen.getByRole("button", { name: /Start here/ })).toBeDisabled();
  });

  it("dismisses and restores a card, and remembers it as dismissed", () => {
    const onAction = renderTray([bug]);
    fireEvent.click(screen.getByRole("button", { name: "Dismiss “Dismiss stays disabled after a failed save”" }));
    expect(onAction).toHaveBeenCalledWith({ type: "dismiss", card: bug, dismissed: true });

    cleanup();
    const restore = vi.fn();
    renderTray([bug], { dismissed: new Set([bug.id]) }, restore);
    expect(screen.queryByRole("button", { name: /Start here/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Restore" }));
    expect(restore).toHaveBeenCalledWith({ type: "dismiss", card: bug, dismissed: false });
  });

  it("says when it was started here, and links to the chat it opened elsewhere", () => {
    renderTray([bug], { started: new Set([bug.id]) });
    expect(screen.getByText("Started in this chat")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Start here/ })).not.toBeInTheDocument();

    cleanup();
    const onOpenChat = vi.fn();
    render(
      <SuggestionScopeContext.Provider value={scope({ links: new Map([[bug.id, { taskId: "new-1", name: "Dismiss button" }]]), onOpenChat })}>
        <SuggestionTray suggestions={[bug]} />
      </SuggestionScopeContext.Provider>
    );
    expect(screen.getByText("Opened in")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "“Dismiss button”" }));
    expect(onOpenChat).toHaveBeenCalledWith("new-1");
  });

  it("clamps a long brief behind Show more", () => {
    renderTray([{ ...bug, description: brief.repeat(3) }]);
    expect(document.querySelector(".suggestion-brief")).toHaveClass("clamped");
    fireEvent.click(screen.getByRole("button", { name: "Show more" }));
    expect(document.querySelector(".suggestion-brief")).not.toHaveClass("clamped");
    fireEvent.click(screen.getByRole("button", { name: "Show less" }));
  });

  it("draws nothing for an empty turn", () => {
    const { container } = render(<SuggestionTray suggestions={[]} />);
    expect(container).toBeEmptyDOMElement();
  });
});

describe("SuggestionNote", () => {
  it("says the words started a suggested task, naming the chat it came from", () => {
    const { rerender } = render(<SuggestionNote />);
    expect(screen.getByText("Suggested task")).toBeInTheDocument();
    expect(screen.queryByText(/from/)).not.toBeInTheDocument();
    rerender(<SuggestionNote origin={{ chatId: "t1", chatName: "Settings cleanup" }} />);
    expect(screen.getByText("from “Settings cleanup”")).toBeInTheDocument();
  });
});
