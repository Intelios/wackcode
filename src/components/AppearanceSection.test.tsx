import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_APPEARANCE } from "../theme";
import type { AppearanceConfig } from "../types";
import { AppearanceSection } from "./AppearanceSection";

afterEach(cleanup);

function renderSection(config: Partial<AppearanceConfig> = {}) {
  const onChange = vi.fn().mockResolvedValue(undefined);
  const onPreview = vi.fn();
  render(<AppearanceSection config={{ ...DEFAULT_APPEARANCE, ...config }} onChange={onChange} onPreview={onPreview} />);
  return { onChange, onPreview };
}

describe("AppearanceSection theme", () => {
  it("applies a preset's accent and background together", async () => {
    const { onChange } = renderSection();
    expect(screen.getByRole("radio", { name: "WackCode" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("radio", { name: "Midnight" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ accent: "#6cc4ff", background: "#0f1218" })));
  });

  it("stores the default preset as unset colours, so future defaults still apply", async () => {
    const { onChange } = renderSection({ accent: "#6cc4ff", background: "#0f1218" });
    expect(screen.getByRole("radio", { name: "Midnight" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("radio", { name: "WackCode" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ accent: null, background: null })));
  });

  it("picks an accent swatch without touching the background", async () => {
    const { onChange } = renderSection({ background: "#14111b" });
    fireEvent.click(screen.getByRole("radio", { name: "Rosé accent colour" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ accent: "#ff8fb8", background: "#14111b" })));
  });

  it("previews a custom colour while dragging and saves it once committed", async () => {
    const { onChange, onPreview } = renderSection();
    const input = screen.getByLabelText("Custom accent colour");
    fireEvent.input(input, { target: { value: "#ff00aa" } });
    expect(onPreview).toHaveBeenCalledWith(expect.objectContaining({ accent: "#ff00aa" }));
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.change(input, { target: { value: "#ff00aa" } });
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ accent: "#ff00aa" })));
  });

  it("darkens a background too light for the text, and says so", async () => {
    const { onChange } = renderSection();
    fireEvent.change(screen.getByLabelText("Custom background colour"), { target: { value: "#ffffff" } });
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    const saved = onChange.mock.calls[0][0] as AppearanceConfig;
    expect(saved.background).toMatch(/^#[0-9a-f]{6}$/);
    expect(saved.background).not.toBe("#ffffff");
    expect(screen.getByText("Darkened to keep text readable.")).toBeInTheDocument();
  });

  it("notes when an accent is lightened to stay readable", () => {
    renderSection({ accent: "#1d4ed8" });
    expect(screen.getByText(/Lightened slightly/)).toBeInTheDocument();
  });

  it("resets a colour to the default, and has nothing to reset when it already is", async () => {
    const { onChange } = renderSection({ accent: "#ff8fb8" });
    expect(screen.getByRole("button", { name: "Reset background colour" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Reset accent colour" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ accent: null })));
  });
});
