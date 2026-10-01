import { useRef } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Popover } from "./Popover";

// jsdom has no layout: the anchor's rect and the panel's content height are faked per test.
let anchorRect = { top: 0, bottom: 0 };
let panelHeight = 0;

function rect(top: number, bottom: number, width: number): DOMRect {
  return { top, bottom, left: 20, right: 20 + width, width, height: bottom - top, x: 20, y: top, toJSON: () => ({}) };
}

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return this.classList.contains("popover") ? rect(0, panelHeight, 240) : rect(anchorRect.top, anchorRect.bottom, 120);
  });
  Object.defineProperty(HTMLElement.prototype, "scrollHeight", {
    configurable: true,
    get(this: HTMLElement) { return this.classList.contains("popover") ? panelHeight : 0; }
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  delete (HTMLElement.prototype as { scrollHeight?: number }).scrollHeight;
});

function Harness({ side }: { side: "top" | "bottom" }) {
  const anchor = useRef<HTMLButtonElement>(null);
  return (
    <>
      <button ref={anchor} type="button">Anchor</button>
      <Popover anchor={anchor} open onClose={() => undefined} side={side}>
        <div>Panel</div>
      </Popover>
    </>
  );
}

function panelStyle() {
  return (screen.getByText("Panel").parentElement as HTMLElement).style;
}

describe("Popover", () => {
  // window.innerHeight is 768 in jsdom; margin 8, offset 6.
  it("stays on its preferred side when the panel fits there", () => {
    anchorRect = { top: 100, bottom: 130 };
    panelHeight = 180;
    render(<Harness side="bottom" />);
    expect(panelStyle().top).toBe("136px");
  });

  it("flips above when the panel would not fit below", () => {
    anchorRect = { top: 570, bottom: 600 };
    panelHeight = 180;
    render(<Harness side="bottom" />);
    expect(panelStyle().top).toBe("384px");
    expect(panelStyle().maxHeight).toBe("556px");
  });

  it("flips below when a top popover would not fit above", () => {
    anchorRect = { top: 90, bottom: 120 };
    panelHeight = 180;
    render(<Harness side="top" />);
    expect(panelStyle().top).toBe("126px");
  });

  it("takes the roomier side and caps the height when neither side fits", () => {
    anchorRect = { top: 570, bottom: 600 };
    panelHeight = 1000;
    render(<Harness side="bottom" />);
    expect(panelStyle().top).toBe("8px");
    expect(panelStyle().maxHeight).toBe("556px");
  });
});
