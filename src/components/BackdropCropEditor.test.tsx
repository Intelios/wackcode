import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CENTRED, type ImageCrop } from "../backdrop-crop";
import { BackdropCropEditor } from "./BackdropCropEditor";

afterEach(cleanup);

function renderEditor(crop: ImageCrop = CENTRED, disabled = false) {
  const onPreview = vi.fn();
  const onCommit = vi.fn();
  render(<BackdropCropEditor imageUrl="asset://a.png" crop={crop} disabled={disabled} onPreview={onPreview} onCommit={onCommit} />);
  return { onPreview, onCommit };
}

describe("BackdropCropEditor", () => {
  it("resets to the centred, uncropped picture, and only when it has moved", () => {
    const { onCommit } = renderEditor({ zoom: 250, x: 100, y: 900 });
    fireEvent.click(screen.getByRole("button", { name: "Reset position" }));
    expect(onCommit).toHaveBeenCalledWith(CENTRED);
    cleanup();
    renderEditor();
    expect(screen.getByRole("button", { name: "Reset position" })).toBeDisabled();
  });

  it("previews the zoom while sliding and saves it on release", () => {
    const onCommit = vi.fn();
    const onPreview = vi.fn();
    const { rerender } = render(<BackdropCropEditor imageUrl="asset://a.png" crop={{ zoom: 150, x: 500, y: 500 }} disabled={false} onPreview={onPreview} onCommit={onCommit} />);
    const slider = screen.getByRole("slider", { name: "Zoom" });
    expect(slider).toHaveValue("150");
    // A drag sends `input` events, and one native `change` when let go.
    fireEvent.input(slider, { target: { value: "220" } });
    expect(onPreview).toHaveBeenCalledWith(expect.objectContaining({ zoom: 220 }));
    expect(onCommit).not.toHaveBeenCalled();
    // The app feeds the preview back in as the crop.
    rerender(<BackdropCropEditor imageUrl="asset://a.png" crop={onPreview.mock.calls[0][0]} disabled={false} onPreview={onPreview} onCommit={onCommit} />);
    fireEvent(slider, new Event("change"));
    expect(onCommit).toHaveBeenCalledWith(expect.objectContaining({ zoom: 220 }));
  });

  it("moves with the arrow keys and saves when the key comes up", () => {
    const { onPreview, onCommit } = renderEditor({ zoom: 200, x: 500, y: 500 });
    const frame = screen.getByRole("group", { name: "Image position" });
    fireEvent.keyDown(frame, { key: "ArrowLeft" });
    expect(onPreview).toHaveBeenCalledTimes(1);
    expect(onCommit).not.toHaveBeenCalled();
    fireEvent.keyUp(frame, { key: "ArrowLeft" });
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it("toggles the layout guides", () => {
    renderEditor();
    expect(screen.getByRole("button", { name: "Hide guides" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "Hide guides" }));
    expect(screen.getByRole("button", { name: "Show guides" })).toHaveAttribute("aria-pressed", "false");
  });

  it("does nothing while a save is in flight", () => {
    const { onPreview } = renderEditor(CENTRED, true);
    fireEvent.keyDown(screen.getByRole("group", { name: "Image position" }), { key: "ArrowLeft" });
    expect(onPreview).not.toHaveBeenCalled();
    expect(screen.getByRole("slider", { name: "Zoom" })).toBeDisabled();
  });
});
