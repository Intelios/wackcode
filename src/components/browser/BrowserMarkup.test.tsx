import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BrowserMarkup } from "./BrowserMarkup";
import { BROWSER_VIEWPORTS } from "../../browser-context";
import type { BrowserContextAttachment } from "../../types";

const capture: BrowserContextAttachment = { id: "capture", kind: "markup", capturedAt: 1, url: "http://localhost/", title: "Example", viewport: BROWSER_VIEWPORTS.desktop,
  image: { type: "image", mimeType: "image/png", data: "aW1hZ2U=" }, targets: [{ selector: "button", path: [], label: "Save", dom: "<button>Save</button>", bounds: { x: 10, y: 10, width: 80, height: 30 } }] };
beforeEach(() => {
  vi.stubGlobal("PointerEvent", MouseEvent);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ clearRect: vi.fn(), beginPath: vi.fn(), strokeText: vi.fn(), fillText: vi.fn(), stroke: vi.fn() } as unknown as CanvasRenderingContext2D);
  vi.spyOn(HTMLCanvasElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 1440, 900));
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it("places text using event-time geometry and supports undo/redo without losing its target", () => {
  render(<BrowserMarkup capture={capture} onAdd={vi.fn()} onClose={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Text" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Markup text" }), { target: { value: "Change this" } });
  fireEvent.pointerDown(screen.getByLabelText("Draw on screenshot"), { button: 0, clientX: 20, clientY: 20 });
  expect(screen.getByText("1 matched element")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Undo markup" }));
  expect(screen.getByText("0 matched elements")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Redo markup" }));
  expect(screen.getByText("1 matched element")).toBeInTheDocument();
});
