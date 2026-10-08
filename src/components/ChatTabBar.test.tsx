import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ChatTabBar, type ChatTabItem } from "./ChatTabBar";
import { ContextMenuProvider } from "./ui/ContextMenu";

vi.mock("motion/react", async (original) => ({ ...await original<typeof import("motion/react")>(), useReducedMotion: () => true }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

const tabs: ChatTabItem[] = [
  { id: "a", title: "Alpha", project: "Project one", model: "Model one", status: "idle" },
  { id: "b", title: "Beta", project: "Project two", model: "Model two", status: "working" },
  { id: "c", title: "Gamma", project: "No project", model: "Model one", status: "completed" }
];

function setup(items = tabs) {
  const handlers = { onSelect: vi.fn(), onClose: vi.fn(), onNew: vi.fn(), onReopen: vi.fn(), onReorder: vi.fn() };
  const surface = (activeId: string, ordered = items) => <ContextMenuProvider items={[]} scope={activeId}
    copyText={vi.fn()} readText={vi.fn()} openLink={vi.fn()} onError={vi.fn()}>
    <ChatTabBar tabs={ordered} activeId={activeId} canReopen {...handlers} />
  </ContextMenuProvider>;
  return { ...render(surface(items[0].id)), ...handlers, surface };
}

describe("ChatTabBar", () => {
  it("offers accessible tabs, roving keyboard focus and separate close buttons", () => {
    const { onSelect, onClose, onNew } = setup();
    const first = screen.getByRole("tab", { name: "Alpha" });
    expect(first).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Beta — Working" })).toHaveAttribute("tabindex", "-1");
    fireEvent.keyDown(first, { key: "ArrowRight" });
    expect(onSelect).toHaveBeenLastCalledWith("b");
    expect(screen.getByRole("tab", { name: "Beta — Working" })).toHaveFocus();
    fireEvent.keyDown(first, { key: "End" });
    expect(onSelect).toHaveBeenLastCalledWith("c");
    fireEvent.click(screen.getByRole("button", { name: "Close tab Alpha" }));
    expect(onClose).toHaveBeenCalledWith("a");
    fireEvent.click(screen.getByRole("button", { name: "New chat tab" }));
    expect(onNew).toHaveBeenCalledOnce();
    expect(screen.getByRole("tablist")).toHaveAttribute("data-tauri-drag-region");
  });

  it("lists every overflowed tab and exposes closed-tab recovery", () => {
    const items = Array.from({ length: 12 }, (_, index) => ({ ...tabs[0], id: String(index), title: `Chat ${index + 1}` }));
    const { onSelect, onReopen } = setup(items);
    fireEvent.click(screen.getByRole("button", { name: "Open tabs" }));
    expect(screen.getAllByRole("menuitem")).toHaveLength(13);
    const last = screen.getByRole("menuitem", { name: /^Chat 12/ });
    expect(last).toHaveTextContent("⌘9");
    fireEvent.click(last);
    expect(onSelect).toHaveBeenCalledWith("11");
    fireEvent.click(screen.getByRole("button", { name: "Open tabs" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Reopen closed tab" }));
    expect(onReopen).toHaveBeenCalledOnce();
  });

  it("reorders through drag and the keyboard-accessible context menu", () => {
    const { onReorder, onSelect } = setup();
    vi.stubGlobal("PointerEvent", MouseEvent);
    vi.spyOn(HTMLElement.prototype, "offsetLeft", "get").mockImplementation(function (this: HTMLElement) {
      return Array.from(this.parentElement?.children ?? []).indexOf(this) * 100;
    });
    vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(100);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 300, 40));
    const beta = screen.getByRole("tab", { name: "Beta — Working" });
    fireEvent.pointerDown(beta, { button: 0, clientX: 150 });
    fireEvent.pointerMove(beta, { clientX: 152 });
    expect(onReorder).not.toHaveBeenCalled();
    fireEvent.pointerMove(beta, { clientX: 250 });
    expect(onReorder).toHaveBeenLastCalledWith("b", 2);
    fireEvent.pointerUp(beta);
    fireEvent.click(beta);
    expect(onSelect).not.toHaveBeenCalled();
    beta.focus();
    fireEvent.keyDown(beta, { key: "F10", shiftKey: true });
    fireEvent.click(screen.getByRole("menuitem", { name: "Move tab left" }));
    expect(onReorder).toHaveBeenLastCalledWith("b", 0);
  });

  it("keeps selection visible after reordering without animation under reduced motion", () => {
    const calls: { node: Element; options: ScrollIntoViewOptions }[] = [];
    HTMLElement.prototype.scrollIntoView = vi.fn(function (this: HTMLElement, options: ScrollIntoViewOptions) { calls.push({ node: this, options }); });
    const { rerender, surface } = setup();
    rerender(surface("a", [tabs[1], tabs[2], tabs[0]]));
    expect(calls.at(-1)).toEqual({ node: screen.getByRole("tab", { name: "Alpha" }), options: { block: "nearest", inline: "nearest", behavior: "instant" } });
    delete (HTMLElement.prototype as { scrollIntoView?: unknown }).scrollIntoView;
  });
});
