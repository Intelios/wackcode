import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CHANGES_VIEW, GAMES_VIEW, TERMINAL_VIEW, RUN_VIEW, type SidePanelView } from "../side-panel";
import { CHAT_MIN } from "../shell-layout";
import type { PanelTabItem } from "./PanelTabStrip";
import { SidePanel } from "./SidePanel";

afterEach(cleanup);

const scout: SidePanelView = { kind: "subagent", taskId: "chat-1", toolCallId: "call-1", index: 0 };
const browser: SidePanelView = { kind: "browser", taskId: "chat-1" };
const content = (view: SidePanelView) => (
  <p>{view.kind === "changes" ? "Changes view"
    : view.kind === "browser" ? "Browser view"
    : view.kind === "terminal" ? "Terminal view"
    : view.kind === "run" ? "Run view"
    : view.kind === "build" ? "Build view"
    : view.kind === "games" ? "Games view"
    : view.kind === "canvas" ? `Canvas view ${view.canvasId}`
    : `SubAgent view ${view.index}`}</p>
);

const item = (view: SidePanelView): PanelTabItem => ({ view, label: view.kind === "subagent" ? "Scout" : view.kind[0].toUpperCase() + view.kind.slice(1), icon: "panel" });

function panel(view: SidePanelView | null, label: string, onWidthChange = vi.fn(), handlers: Partial<Parameters<typeof SidePanel>[0]> = {}, tabs = view ? [item(view)] : []) {
  return <SidePanel view={view} width={430} onWidthChange={onWidthChange} onResetWidth={vi.fn()} label={label}
    tabs={tabs} addable={[]} expanded={false} onSelectTab={vi.fn()} onCloseTab={vi.fn()} onAddTab={vi.fn()} onToggleExpand={vi.fn()} onHide={vi.fn()} {...handlers}>{content}</SidePanel>;
}

describe("SidePanel", () => {
  it("retires a resize gesture when switching away unmounts its tab panel", () => {
    const onWidthChange = vi.fn();
    const view = render(panel(TERMINAL_VIEW, "Terminal", onWidthChange));
    fireEvent.pointerDown(view.container.querySelector(".panel-resizer")!, { clientX: 500 });
    fireEvent.pointerMove(window, { clientX: 450 });
    expect(onWidthChange).toHaveBeenCalledOnce();
    view.unmount();
    fireEvent.pointerMove(window, { clientX: 300 });
    expect(onWidthChange).toHaveBeenCalledOnce();
  });
  it("shows one view at a time, named for what it shows", async () => {
    const view = render(panel(CHANGES_VIEW, "Changes"));
    expect(screen.getByRole("complementary", { name: "Changes" })).toHaveAttribute("id", "side-panel");
    expect(screen.getByText("Changes view")).toBeInTheDocument();

    view.rerender(panel(browser, "Browser"));
    expect(screen.getByRole("complementary", { name: "Browser" })).toBeInTheDocument();
    expect(screen.getByText("Browser view")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("Changes view")).not.toBeInTheDocument());
    expect(screen.getAllByRole("complementary")).toHaveLength(1);
  });

  it("swaps between durable views and a sub-agent", async () => {
    const view = render(panel(CHANGES_VIEW, "Changes"));
    view.rerender(panel(TERMINAL_VIEW, "Terminal"));
    expect(screen.getByRole("complementary", { name: "Terminal" })).toBeInTheDocument();
    expect(screen.getByText("Terminal view")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("Changes view")).not.toBeInTheDocument());

    view.rerender(panel(RUN_VIEW, "Run"));
    expect(screen.getByText("Run view")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("Terminal view")).not.toBeInTheDocument());

    view.rerender(panel(scout, "SubAgent Scout"));
    expect(screen.getByText("SubAgent view 0")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("Terminal view")).not.toBeInTheDocument());
  });

  it("keeps a call's children on one page, and slides away when closed", async () => {
    const view = render(panel(scout, "SubAgent Scout"));
    view.rerender(panel({ ...scout, index: 1 }, "SubAgent Scout"));
    expect(screen.getByText("SubAgent view 1")).toBeInTheDocument();
    expect(screen.queryByText("SubAgent view 0")).not.toBeInTheDocument();

    view.rerender(panel(null, "SubAgent Scout"));
    await waitFor(() => expect(screen.queryByRole("complementary")).not.toBeInTheDocument());
  });

  it("shows its tabs, the front one selected, and closes or walks them from the keyboard", () => {
    const onCloseTab = vi.fn();
    const onSelectTab = vi.fn();
    const onHide = vi.fn();
    render(panel(TERMINAL_VIEW, "Terminal", vi.fn(), { onCloseTab, onSelectTab, onHide }, [item(CHANGES_VIEW), item(TERMINAL_VIEW), item(GAMES_VIEW)]));
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual(["Changes", "Terminal", "Games"]);
    expect(screen.getByRole("tab", { name: "Terminal" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tabpanel")).toHaveTextContent("Terminal view");
    fireEvent.click(screen.getByRole("button", { name: "Close Games tab" }));
    expect(onCloseTab).toHaveBeenCalledWith(GAMES_VIEW);
    fireEvent.keyDown(screen.getByRole("tab", { name: "Terminal" }), { key: "ArrowLeft" });
    expect(onSelectTab).toHaveBeenCalledWith(CHANGES_VIEW);
    fireEvent.keyDown(screen.getByRole("tab", { name: "Terminal" }), { key: "End" });
    expect(onSelectTab).toHaveBeenLastCalledWith(GAMES_VIEW);
    fireEvent.click(screen.getByRole("button", { name: "Hide panel" }));
    expect(onHide).toHaveBeenCalledOnce();
  });

  it("offers the views not open yet behind its + menu", () => {
    const onAddTab = vi.fn();
    render(panel(CHANGES_VIEW, "Changes", vi.fn(), { onAddTab, addable: [{ view: TERMINAL_VIEW, label: "Terminal", icon: "terminal", keys: "⌘⇧T" }] }));
    fireEvent.click(screen.getByRole("button", { name: "Open a tab" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /Terminal/ }));
    expect(onAddTab).toHaveBeenCalledWith(TERMINAL_VIEW);
  });

  it("resets to a balanced split on a double-click of its edge", () => {
    const onResetWidth = vi.fn();
    const { container } = render(panel(CHANGES_VIEW, "Changes", vi.fn(), { onResetWidth }));
    fireEvent.doubleClick(container.querySelector(".panel-resizer")!);
    expect(onResetWidth).toHaveBeenCalledOnce();
  });

  it("resizes from its left edge, within its limits", () => {
    const onWidthChange = vi.fn();
    const { container } = render(panel(CHANGES_VIEW, "Changes", onWidthChange));
    fireEvent.pointerDown(container.querySelector(".panel-resizer") as Element, { clientX: 500 });
    fireEvent.pointerMove(window, { clientX: 400 });
    expect(onWidthChange).toHaveBeenLastCalledWith(530);
    fireEvent.pointerMove(window, { clientX: -1_000 });
    // As wide as the window allows once the sidebar steps aside: the chat keeps its minimum.
    expect(onWidthChange).toHaveBeenLastCalledWith(window.innerWidth - CHAT_MIN);
    fireEvent.pointerUp(window);
    fireEvent.pointerMove(window, { clientX: 300 });
    expect(onWidthChange).toHaveBeenCalledTimes(2);
  });
});
