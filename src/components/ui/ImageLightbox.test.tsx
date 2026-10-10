import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ImageLightbox } from "./ImageLightbox";
import { NavigationScope } from "./NavigationScope";

// jsdom has no layout, so the image's box is faked: 400×300 at (100, 100). Reduced motion is
// faked too, so every zoom animation lands at its end state immediately.
const IMG_RECT = { left: 100, top: 100, width: 400, height: 300 };
const ZOOM = 2.5;
const onClose = vi.fn();

function renderLightbox() {
  render(<ImageLightbox preview="data:image/png;base64,thumb" alt="Attached image 1" onClose={onClose} />);
  return screen.getByAltText("Attached image 1") as HTMLElement;
}

beforeEach(() => {
  window.matchMedia = ((query: string) => ({
    matches: query.includes("prefers-reduced-motion"), media: query, onchange: null,
    addListener: vi.fn(), removeListener: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn(), dispatchEvent: vi.fn(),
  })) as unknown as typeof window.matchMedia;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    if (this.tagName !== "IMG") return { left: 0, top: 0, width: 0, height: 0, x: 0, y: 0, right: 0, bottom: 0, toJSON: () => ({}) };
    return { ...IMG_RECT, x: IMG_RECT.left, y: IMG_RECT.top, right: IMG_RECT.left + IMG_RECT.width, bottom: IMG_RECT.top + IMG_RECT.height, toJSON: () => ({}) };
  });
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", { configurable: true, get(this: HTMLElement) { return this.tagName === "IMG" ? IMG_RECT.width : 0; } });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, get(this: HTMLElement) { return this.tagName === "IMG" ? IMG_RECT.height : 0; } });
});

afterEach(() => {
  cleanup();
  onClose.mockClear();
  vi.restoreAllMocks();
  delete (window as { matchMedia?: typeof window.matchMedia }).matchMedia;
  delete (HTMLElement.prototype as { offsetWidth?: number }).offsetWidth;
  delete (HTMLElement.prototype as { offsetHeight?: number }).offsetHeight;
});

describe("ImageLightbox", () => {
  it("dismisses on navigation so a late original cannot appear over another tab", async () => {
    let resolve!: (url: string) => void;
    const load = () => new Promise<string>((done) => { resolve = done; });
    const surface = (scope: string) => <NavigationScope.Provider value={scope}>
      <ImageLightbox preview="data:image/png;base64,thumb" load={load} alt="Preview" onClose={onClose} />
    </NavigationScope.Provider>;
    const view = render(surface("chat"));
    view.rerender(surface("git"));
    expect(onClose).toHaveBeenCalledOnce();
    view.unmount();
    resolve("data:image/png;base64,original");
    await Promise.resolve();
    expect(screen.queryByRole("dialog")).toBeNull();
  });
  it("shows the preview sharp at once when it is the only image (no load)", () => {
    const img = renderLightbox();
    // The composer passes its already-full-size data URL with no `load`: it must never wear
    // the blurred placeholder class.
    expect(img.className).not.toContain("preview");
    expect(img.className).toContain("loaded");
  });
  it("keeps the blurred preview when load resolves undefined (original unavailable)", async () => {
    const img = render(<ImageLightbox preview="data:image/png;base64,thumb" load={() => Promise.resolve(undefined)} alt="Attached image 1" onClose={onClose} />)
      .getByAltText("Attached image 1");
    expect(img.className).toContain("preview"); // the placeholder while loading
    await act(async () => {}); // let the undefined-resolving promise settle
    expect(img.className).toContain("preview"); // and it stays blurred: no original to sharpen into
    expect(img.className).not.toContain("loaded");
  });
  it("zooms toward the clicked point, and a second click zooms back out", async () => {
    const img = renderLightbox();
    // A click a quarter of the way across: that point becomes the centre.
    fireEvent.click(img, { clientX: 200, clientY: 250 });
    await waitFor(() => expect(img.style.transform).toContain(`scale(${ZOOM})`));
    expect(img.style.transform).toContain(`translateX(${ZOOM * 0.25 * IMG_RECT.width}px)`);
    expect(img.style.transform).not.toContain("translateY"); // clicked mid-height, so no vertical offset
    expect(screen.getByRole("dialog").className).toContain("zoomed");

    fireEvent.click(img, { clientX: 300, clientY: 250 });
    await waitFor(() => expect(img.style.transform).not.toContain(`scale(${ZOOM})`));
    expect(screen.getByRole("dialog").className).not.toContain("zoomed");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("clamps a click near the edge so the zoom never pans past the image", async () => {
    const img = renderLightbox();
    fireEvent.click(img, { clientX: 101, clientY: 300 });
    const limit = ((ZOOM - 1) * IMG_RECT.width) / 2;
    await waitFor(() => expect(img.style.transform).toContain(`translateX(${limit}px)`));
  });

  it("pans while zoomed, and the click that ends a pan doesn't toggle the zoom", async () => {
    const img = renderLightbox();
    fireEvent.click(img, { clientX: 300, clientY: 250 }); // dead centre: no offset
    await waitFor(() => expect(img.style.transform).toContain(`scale(${ZOOM})`));

    fireEvent.pointerDown(img, { button: 0, clientX: 300, clientY: 250 });
    fireEvent.pointerMove(window, { clientX: 200, clientY: 250 });
    expect(screen.getByRole("dialog").className).toContain("panning");
    fireEvent.pointerUp(window, { clientX: 200, clientY: 250 });
    await waitFor(() => expect(img.style.transform).toContain("translateX(-100px)"));
    expect(screen.getByRole("dialog").className).not.toContain("panning");

    // The browser fires a click after that drag; it must not zoom back out.
    fireEvent.click(img, { clientX: 200, clientY: 250 });
    expect(img.style.transform).toContain(`scale(${ZOOM})`);
  });

  it("treats a press that never leaves the slop as a click", async () => {
    const img = renderLightbox();
    fireEvent.click(img, { clientX: 300, clientY: 250 });
    await waitFor(() => expect(img.style.transform).toContain(`scale(${ZOOM})`));

    fireEvent.pointerDown(img, { button: 0, clientX: 300, clientY: 250 });
    fireEvent.pointerMove(window, { clientX: 302, clientY: 251 });
    fireEvent.pointerUp(window, { clientX: 302, clientY: 251 });
    fireEvent.click(img, { clientX: 302, clientY: 251 });
    await waitFor(() => expect(img.style.transform).not.toContain(`scale(${ZOOM})`));
  });

  it("zooms to the centre from the keyboard, and − zooms back out", async () => {
    renderLightbox();
    fireEvent.keyDown(document, { key: "=" });
    const img = screen.getByAltText("Attached image 1");
    await waitFor(() => expect(img.style.transform).toContain(`scale(${ZOOM})`));
    expect(img.style.transform).not.toContain("translateX");

    fireEvent.keyDown(document, { key: "-" });
    await waitFor(() => expect(img.style.transform).not.toContain(`scale(${ZOOM})`));
  });

  it("closes on Escape, on the close button, and on a click outside the image", () => {
    renderLightbox();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(2);

    fireEvent.mouseDown(document.querySelector(".image-lightbox")!);
    expect(onClose).toHaveBeenCalledTimes(3);
  });
});
