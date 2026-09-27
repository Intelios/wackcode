import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CHANGES_VIEW, TERMINAL_VIEW, type SidePanelView } from "../side-panel";
import { SidePanel } from "./SidePanel";

afterEach(cleanup);

const scout: SidePanelView = { kind: "subagent", taskId: "chat-1", toolCallId: "call-1", index: 0 };
const content = (view: SidePanelView) => (
  <p>{view.kind === "changes" ? "Changes view" : view.kind === "terminal" ? "Terminal view" : `SubAgent view ${view.index}`}</p>
);

function panel(view: SidePanelView | null, label: string, onWidthChange = vi.fn()) {
  return <SidePanel view={view} width={430} onWidthChange={onWidthChange} label={label}>{content}</SidePanel>;
}

describe("SidePanel", () => {
  it("shows one view at a time, named for what it shows", async () => {
    const view = render(panel(CHANGES_VIEW, "Changes"));
    expect(screen.getByRole("complementary", { name: "Changes" })).toHaveAttribute("id", "side-panel");
    expect(screen.getByText("Changes view")).toBeInTheDocument();

    view.rerender(panel(scout, "SubAgent Scout"));
    expect(screen.getByRole("complementary", { name: "SubAgent Scout" })).toBeInTheDocument();
    expect(screen.getByText("SubAgent view 0")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("Changes view")).not.toBeInTheDocument());
    expect(screen.getAllByRole("complementary")).toHaveLength(1);
  });

  it("swaps between Changes and Terminal as durable views", async () => {
    const view = render(panel(CHANGES_VIEW, "Changes"));
    view.rerender(panel(TERMINAL_VIEW, "Terminal"));
    expect(screen.getByRole("complementary", { name: "Terminal" })).toBeInTheDocument();
    expect(screen.getByText("Terminal view")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("Changes view")).not.toBeInTheDocument());

    view.rerender(panel(scout, "SubAgent Scout"));
    expect(screen.getByText("SubAgent view 0")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("Terminal view")).not.toBeInTheDocument());
  });

  it("keeps a call's children on one page, and slides away when closed", async () => {
    const view = render(panel(scout, "SubAgent Scout"));
    // Siblings share a page: the view's own tabs animate, so nothing is left behind to fade out.
    view.rerender(panel({ ...scout, index: 1 }, "SubAgent Scout"));
    expect(screen.getByText("SubAgent view 1")).toBeInTheDocument();
    expect(screen.queryByText("SubAgent view 0")).not.toBeInTheDocument();

    view.rerender(panel(null, "SubAgent Scout"));
    await waitFor(() => expect(screen.queryByRole("complementary")).not.toBeInTheDocument());
  });

  it("resizes from its left edge, within its limits", () => {
    const onWidthChange = vi.fn();
    const { container } = render(panel(CHANGES_VIEW, "Changes", onWidthChange));
    fireEvent.pointerDown(container.querySelector(".panel-resizer") as Element, { clientX: 500 });
    fireEvent.pointerMove(window, { clientX: 400 });
    expect(onWidthChange).toHaveBeenLastCalledWith(530);
    fireEvent.pointerMove(window, { clientX: -1_000 });
    expect(onWidthChange).toHaveBeenLastCalledWith(720);
    fireEvent.pointerUp(window);
    fireEvent.pointerMove(window, { clientX: 300 });
    expect(onWidthChange).toHaveBeenCalledTimes(2);
  });
});
