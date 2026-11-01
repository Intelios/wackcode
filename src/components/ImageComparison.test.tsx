import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ImageComparison } from "./ImageComparison";
import { ReplyImageScope } from "./ReplyImage";
import { NavigationScope } from "./ui/NavigationScope";
import { replyImages } from "../reply-images";
import type { NormalizedMessage } from "../types";

vi.mock("motion/react", async (importOriginal) => ({ ...await importOriginal<typeof import("motion/react")>(), useReducedMotion: () => true }));

const comparison = { v: 1 as const, before: "wackcode-image://entry/upload/0", after: "wackcode-image://entry/result/0", title: "New layout", beforeLabel: "Original", afterLabel: "Edited" };
const left = "data:image/png;base64,bGVmdA==";
const right = "data:image/png;base64,cmlnaHQ=";
const messages = (): NormalizedMessage[] => [
  { id: "u", entryId: "upload", role: "user", blocks: [{ type: "image", imageId: "before", thumbnail: left }] },
  { id: "r", entryId: "result", role: "tool", blocks: [{ type: "tool-result", toolCallId: "shot", images: [{ imageId: "after", thumbnail: right }] }] },
];

function surface(input: { messages?: NormalizedMessage[]; loadMessage?: (id: string, index: number) => Promise<string | undefined>; loadTool?: (id: string, index: number) => Promise<string | undefined>; navigation?: string } = {}) {
  return <NavigationScope.Provider value={input.navigation ?? "chat"}>
    <ReplyImageScope.Provider value={{ images: replyImages(input.messages ?? messages()), loadMessage: input.loadMessage, loadTool: input.loadTool }}>
      <ImageComparison comparison={comparison} />
    </ReplyImageScope.Provider>
  </NavigationScope.Provider>;
}

beforeEach(() => {
  vi.stubGlobal("PointerEvent", class extends MouseEvent {
    pointerId: number;
    constructor(type: string, options: PointerEventInit = {}) { super(type, options); this.pointerId = options.pointerId ?? 1; }
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ left: 100, top: 100, width: 400, height: 200, right: 500, bottom: 300, x: 100, y: 100, toJSON: () => ({}) });
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(400);
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(200);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("image comparison", () => {
  it("fits both images without cropping and uses the after image's proportions", () => {
    const { container } = render(surface());
    const slider = screen.getByRole("slider", { name: "New layout: comparison slider" });
    expect(slider).toHaveAttribute("aria-valuenow", "50");
    expect(slider).toHaveAttribute("aria-valuetext", "Original 50%, Edited 50%");
    const after = screen.getByAltText("Edited");
    Object.defineProperties(after, { naturalWidth: { value: 300 }, naturalHeight: { value: 600 } });
    fireEvent.load(after);
    expect(container.querySelector("figure")?.style.getPropertyValue("--comparison-ratio")).toBe("0.5");
    expect(screen.getByAltText("Original")).toHaveAttribute("src", left);
    expect(after).toHaveAttribute("src", right);
    expect(container.querySelectorAll(".image-comparison-layer")[0]).toHaveStyle({ clipPath: "inset(0 0 0 50%)" });
  });

  it("supports keyboard endpoints and captured pointer dragging beyond the frame", () => {
    render(surface());
    const slider = screen.getByRole("slider");
    const capture = vi.fn();
    Object.assign(slider, { setPointerCapture: capture });
    fireEvent.keyDown(slider, { key: "ArrowLeft" });
    expect(slider).toHaveAttribute("aria-valuenow", "49");
    fireEvent.keyDown(slider, { key: "Home" });
    fireEvent.keyDown(slider, { key: "ArrowLeft" });
    expect(slider).toHaveAttribute("aria-valuenow", "0");
    fireEvent.keyDown(slider, { key: "End" });
    fireEvent.keyDown(slider, { key: "ArrowRight" });
    expect(slider).toHaveAttribute("aria-valuenow", "100");
    fireEvent.pointerDown(slider, { button: 0, pointerId: 4, clientX: 200 });
    expect(capture).toHaveBeenCalledWith(4);
    expect(slider).toHaveAttribute("aria-valuenow", "25");
    fireEvent.pointerMove(slider, { pointerId: 5, clientX: 600 });
    expect(slider).toHaveAttribute("aria-valuenow", "25");
    fireEvent.pointerMove(slider, { pointerId: 4, clientX: 600 });
    expect(slider).toHaveAttribute("aria-valuenow", "100");
    fireEvent.pointerMove(slider, { pointerId: 4, clientX: 0 });
    expect(slider).toHaveAttribute("aria-valuenow", "0");
    fireEvent.pointerCancel(slider, { pointerId: 4 });
    fireEvent.pointerMove(slider, { pointerId: 4, clientX: 300 });
    expect(slider).toHaveAttribute("aria-valuenow", "0");
  });

  it("preserves the divider across expansion, traps focus, and restores the trigger on close", async () => {
    render(surface());
    fireEvent.keyDown(screen.getByRole("slider"), { key: "ArrowRight" });
    const expand = screen.getByRole("button", { name: "Expand image comparison" });
    expand.focus();
    fireEvent.click(expand);
    const dialog = screen.getByRole("dialog", { name: "Image comparison: New layout" });
    expect(within(dialog).getByRole("slider")).toHaveAttribute("aria-valuenow", "51");
    expect(within(dialog).getByRole("button", { name: "Close image comparison" })).toHaveFocus();
    const zoomIn = within(dialog).getByRole("button", { name: "Zoom in" });
    zoomIn.focus();
    fireEvent.keyDown(document, { key: "Tab" });
    expect(within(dialog).getByRole("slider")).toHaveFocus();
    fireEvent.keyDown(document, { key: "Tab", shiftKey: true });
    expect(zoomIn).toHaveFocus();
    fireEvent.keyDown(within(dialog).getByRole("slider"), { key: "Home" });
    fireEvent.keyDown(document, { key: "Escape" });
    await act(async () => {});
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("slider")).toHaveAttribute("aria-valuenow", "0");
    expect(expand).toHaveFocus();
  });

  it("zooms and pans both images together while the divider stays in viewport coordinates", async () => {
    render(surface());
    fireEvent.click(screen.getByRole("button", { name: "Expand image comparison" }));
    const dialog = screen.getByRole("dialog");
    const images = within(dialog).getAllByRole("img") as HTMLImageElement[];
    const stage = images[0].closest<HTMLElement>(".image-comparison-stage")!;
    fireEvent.click(within(dialog).getByRole("button", { name: "Zoom in" }));
    await waitFor(() => expect(images[0].style.transform).toContain("scale(2.5)"));
    expect(images[1].style.transform).toBe(images[0].style.transform);
    fireEvent.pointerDown(stage, { button: 0, pointerId: 1, clientX: 300, clientY: 200 });
    fireEvent.pointerMove(stage, { pointerId: 1, clientX: 2000, clientY: 2000 });
    fireEvent.pointerUp(stage, { pointerId: 1 });
    fireEvent.click(stage);
    await waitFor(() => expect(images[0].style.transform).toContain("translateX(300px)"));
    expect(images[0].style.transform).toContain("translateY(150px)");
    expect(images[1].style.transform).toBe(images[0].style.transform);
    const slider = within(dialog).getByRole("slider");
    fireEvent.pointerDown(slider, { button: 0, pointerId: 2, clientX: 200 });
    fireEvent.pointerMove(slider, { pointerId: 2, clientX: 400 });
    fireEvent.pointerUp(slider, { pointerId: 2 });
    expect(slider).toHaveAttribute("aria-valuenow", "75");
    expect(images[0].parentElement).toHaveStyle({ clipPath: "inset(0 0 0 75%)" });
    expect(images[1].style.transform).toBe(images[0].style.transform);
    fireEvent.click(within(dialog).getByRole("button", { name: "Fit" }));
    await waitFor(() => expect(images[0].style.transform).toBe("none"));
    expect(images[1].style.transform).toBe("none");
  });

  it("waits until visible, then upgrades both originals together without reloading on a snapshot", async () => {
    let observe!: (entries: Array<{ isIntersecting: boolean }>) => void;
    vi.stubGlobal("IntersectionObserver", class {
      constructor(callback: typeof observe) { observe = callback; }
      observe() {} disconnect() {}
    });
    const decodes: Array<{ src: string; naturalWidth: number; naturalHeight: number; onload: () => void; onerror: () => void }> = [];
    vi.stubGlobal("Image", class {
      src = ""; naturalWidth = 800; naturalHeight = 400; onload = () => {}; onerror = () => {};
      constructor() { decodes.push(this); }
    });
    const loadMessage = vi.fn().mockResolvedValue("data:image/png;base64,b3JpZ2luYWwtbGVmdA==");
    const loadTool = vi.fn().mockResolvedValue("data:image/png;base64,b3JpZ2luYWwtcmlnaHQ=");
    const view = render(surface({ loadMessage, loadTool }));
    expect(loadMessage).not.toHaveBeenCalled();
    act(() => observe([{ isIntersecting: true }]));
    await act(async () => {});
    expect(loadMessage).toHaveBeenCalledWith("upload", 0);
    expect(loadTool).toHaveBeenCalledWith("shot", 0);
    expect(decodes).toHaveLength(2);
    act(() => decodes[0].onload());
    await act(async () => {});
    expect(screen.getByAltText("Original")).toHaveAttribute("src", left);
    act(() => decodes[1].onload());
    await act(async () => {});
    expect(screen.getByAltText("Original")).toHaveAttribute("src", decodes[0].src);
    expect(screen.getByAltText("Edited")).toHaveAttribute("src", decodes[1].src);
    fireEvent.keyDown(screen.getByRole("slider"), { key: "ArrowRight" });
    view.rerender(surface({ loadMessage, loadTool }));
    await act(async () => {});
    expect(loadMessage).toHaveBeenCalledOnce();
    expect(loadTool).toHaveBeenCalledOnce();
    expect(screen.getByRole("slider")).toHaveAttribute("aria-valuenow", "51");
  });

  it("retains both previews when an original fails, and gives pending and unavailable sources clear states", async () => {
    const loadMessage = vi.fn().mockRejectedValue(new Error("Worker unavailable"));
    const loadTool = vi.fn().mockResolvedValue(right);
    const view = render(surface({ loadMessage, loadTool }));
    expect(await screen.findByText("Preview quality")).toBeInTheDocument();
    expect(screen.getByAltText("Original")).toHaveAttribute("src", left);
    expect(screen.getByAltText("Edited")).toHaveAttribute("src", right);
    fireEvent.click(screen.getByRole("button", { name: "Expand image comparison" }));
    expect(within(screen.getByRole("dialog")).getByText("Preview quality")).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "Escape" });
    const pending = messages();
    pending[0] = { ...pending[0], blocks: [{ type: "image" }] };
    view.rerender(surface({ messages: pending }));
    expect(screen.getByText("Preparing comparison…")).toBeInTheDocument();
    expect(screen.queryByRole("slider")).toBeNull();
    view.rerender(surface({ messages: messages().slice(0, 1) }));
    expect(screen.getByRole("img", { name: "Image comparison unavailable: New layout" })).toBeInTheDocument();
    view.rerender(surface());
    fireEvent.error(screen.getByAltText("Original"));
    expect(screen.getByRole("img", { name: "Image comparison unavailable: New layout" })).toBeInTheDocument();
  });

  it("retires the expanded viewer on navigation or a source replacement", () => {
    const view = render(surface());
    fireEvent.click(screen.getByRole("button", { name: "Expand image comparison" }));
    view.rerender(surface({ navigation: "another-chat" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Expand image comparison" }));
    const edited = messages();
    edited[0] = { ...edited[0], blocks: [{ type: "image", imageId: "replacement", thumbnail: right }] };
    view.rerender(surface({ messages: edited, navigation: "another-chat" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByAltText("Original")).toHaveAttribute("src", right);
    fireEvent.click(screen.getByRole("button", { name: "Expand image comparison" }));
    fireEvent.mouseDown(screen.getByRole("dialog").parentElement!);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
