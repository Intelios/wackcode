import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_APPEARANCE, swatchTint } from "../theme";
import type { AppearanceConfig } from "../types";
import { AppearanceSection } from "./AppearanceSection";

afterEach(cleanup);

/** A backdrop-Image config with the named library, the first entry shown. */
function imageLibrary(names: string[], shown = names[0]): Partial<AppearanceConfig> {
  return {
    backdrop: "image",
    backgroundImage: shown,
    backgroundImages: names.map((name) => ({ name, zoom: 100, x: 500, y: 500 }))
  };
}

function renderSection(config: Partial<AppearanceConfig> = {}, { glassSupported = true, urls = {} as Record<string, string> } = {}) {
  const onChange = vi.fn().mockResolvedValue(undefined);
  const onPreview = vi.fn();
  const onChooseImage = vi.fn().mockResolvedValue(undefined);
  const onRemoveImage = vi.fn().mockResolvedValue(undefined);
  const onSelectImage = vi.fn().mockResolvedValue(undefined);
  const onShuffleImage = vi.fn().mockResolvedValue(undefined);
  render(
    <AppearanceSection
      config={{ ...DEFAULT_APPEARANCE, ...config }}
      glassSupported={glassSupported}
      backgroundImageUrlFor={(name) => urls[name]}
      onChange={onChange}
      onPreview={onPreview}
      onChooseImage={onChooseImage}
      onRemoveImage={onRemoveImage}
      onSelectImage={onSelectImage}
      onShuffleImage={onShuffleImage}
    />
  );
  return { onChange, onPreview, onChooseImage, onRemoveImage, onSelectImage, onShuffleImage };
}

describe("AppearanceSection theme", () => {
  it("offers chat tabs off by default and saves through the appearance configuration", async () => {
    const { onChange } = renderSection();
    const toggle = screen.getByRole("switch", { name: "Chat tabs" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    fireEvent.click(toggle);
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ chatTabs: true })));
  });
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

  it("rings the near-black background swatches in their tint, and leaves the accent discs flat", () => {
    renderSection();
    const background = screen.getByRole("radio", { name: "Grape background colour" });
    expect(background).toHaveClass("swatch-tint");
    expect(background.style.getPropertyValue("--swatch-tint")).toBe(swatchTint("#14111b"));
    const accent = screen.getByRole("radio", { name: "Grape accent colour" });
    expect(accent).not.toHaveClass("swatch-tint");
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

describe("AppearanceSection backdrop", () => {
  it("goes straight to the image picker when Image has nothing stored yet", async () => {
    const { onChange, onChooseImage } = renderSection();
    fireEvent.click(screen.getByRole("radio", { name: "Image" }));
    await waitFor(() => expect(onChooseImage).toHaveBeenCalled());
    expect(onChange).not.toHaveBeenCalled();
  });

  it("switches back to a stored image without asking again", async () => {
    const { onChange, onChooseImage } = renderSection({ backgroundImage: "a.png", backgroundImages: [{ name: "a.png", zoom: 100, x: 500, y: 500 }] });
    fireEvent.click(screen.getByRole("radio", { name: "Image" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ backdrop: "image" })));
    expect(onChooseImage).not.toHaveBeenCalled();
  });

  it("shows one card per saved image and removes a named one", async () => {
    const { onRemoveImage } = renderSection(
      { ...imageLibrary(["a.png", "b.png"], "b.png"), backgroundImages: [{ name: "a.png", zoom: 100, x: 500, y: 500 }, { name: "b.png", zoom: 160, x: 250, y: 750 }] },
      { urls: { "a.png": "asset://localhost/a.png", "b.png": "asset://localhost/b.png" } }
    );
    expect(screen.getByRole("button", { name: "Background 2 (showing)" })).toHaveStyle({ backgroundImage: 'url("asset://localhost/b.png")' });
    expect(screen.getByRole("button", { name: "Background 1" })).toHaveAttribute("aria-pressed", "false");
    expect(screen.getByRole("slider", { name: "Dim" })).toBeInTheDocument();
    expect(screen.queryByRole("slider", { name: "Tint" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Remove background 1" }));
    await waitFor(() => expect(onRemoveImage).toHaveBeenCalledWith("a.png"));
  });

  it("shows a clicked card, without re-selecting the one already shown", async () => {
    const { onSelectImage } = renderSection(imageLibrary(["a.png", "b.png"]), { urls: { "a.png": "asset://localhost/a.png", "b.png": "asset://localhost/b.png" } });
    fireEvent.click(screen.getByRole("button", { name: "Background 2" }));
    await waitFor(() => expect(onSelectImage).toHaveBeenCalledWith("b.png"));
    fireEvent.click(screen.getByRole("button", { name: "Background 1 (showing)" }));
    expect(onSelectImage).toHaveBeenCalledTimes(1);
  });

  it("offers shuffle once two images are saved, and saves the choice", async () => {
    renderSection(imageLibrary(["a.png"]));
    expect(screen.queryByRole("switch", { name: "Shuffle" })).not.toBeInTheDocument();
    const { onChange, onShuffleImage } = renderSection(imageLibrary(["a.png", "b.png"]));
    const toggle = screen.getByRole("switch", { name: "Shuffle" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    fireEvent.click(toggle);
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ shuffleBackgrounds: true })));
    fireEvent.click(screen.getByRole("button", { name: "Shuffle now" }));
    await waitFor(() => expect(onShuffleImage).toHaveBeenCalled());
  });

  it("resets the crop of the shown image onto its own entry", async () => {
    const { onChange } = renderSection(
      {
        ...imageLibrary(["a.png", "b.png"], "b.png"),
        backgroundImages: [{ name: "a.png", zoom: 130, x: 400, y: 600 }, { name: "b.png", zoom: 160, x: 250, y: 750 }]
      },
      { urls: { "b.png": "asset://localhost/b.png" } }
    );
    fireEvent.click(screen.getByRole("button", { name: "Reset position" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      backgroundImages: [{ name: "a.png", zoom: 130, x: 400, y: 600 }, { name: "b.png", zoom: 100, x: 500, y: 500 }]
    })));
  });

  it("previews a slider while dragging and saves it on release", async () => {
    const { onChange, onPreview } = renderSection({ backdrop: "image", backgroundImage: "a.png" });
    const dim = screen.getByRole("slider", { name: "Dim" });
    fireEvent.input(dim, { target: { value: "30" } });
    expect(onPreview).toHaveBeenCalledWith(expect.objectContaining({ imageDim: 30 }));
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.change(dim, { target: { value: "30" } });
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ imageDim: 30 })));
  });

  it("offers Liquid Glass styles and tint once chosen", async () => {
    const { onChange } = renderSection({ backdrop: "glass" });
    expect(screen.getByRole("slider", { name: "Tint" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: "Clear" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ backdrop: "glass", glassStyle: "clear" })));
  });

  it("disables Liquid Glass before macOS 26", () => {
    renderSection({}, { glassSupported: false });
    expect(screen.getByRole("radio", { name: "Liquid Glass" })).toBeDisabled();
  });
});

describe("AppearanceSection composer styles", () => {
  it("defaults to WackCode Classic and saves a picked layout", async () => {
    const { onChange } = renderSection();
    expect(screen.getByRole("radio", { name: "WackCode Classic" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("radio", { name: "Capsule" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ composerStyle: "capsule" })));
  });

  it("offers Classic Liquid Glass only where Liquid Glass exists", () => {
    renderSection({}, { glassSupported: false });
    expect(screen.getByRole("radio", { name: "Classic Liquid Glass" })).toBeDisabled();
    cleanup();
    renderSection({ composerStyle: "classicGlass" });
    const glass = screen.getByRole("radio", { name: "Classic Liquid Glass" });
    expect(glass).toBeEnabled();
    expect(glass).toHaveAttribute("aria-checked", "true");
  });

  it("saves Split Bar and Terminal without touching the rest of the appearance", async () => {
    const config = { ...DEFAULT_APPEARANCE, backdrop: "dynamic" as const, agentName: "Nova" };
    const { onChange } = renderSection(config);
    fireEvent.click(screen.getByRole("radio", { name: "Split Bar" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith({ ...config, composerStyle: "split" }));
    fireEvent.click(screen.getByRole("radio", { name: "Terminal" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith({ ...config, composerStyle: "terminal" }));
  });
});

describe("AppearanceSection completed work", () => {
  it("defaults to folding completed steps and explains that live work stays open", () => {
    renderSection();
    const toggle = screen.getByRole("switch", { name: "Collapse completed work" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    const card = within(toggle.closest(".chat-preview-card")!);
    expect(card.getByText("Worked for 12s")).toBeInTheDocument();
    expect(card.queryByText("Thought for 2s")).not.toBeInTheDocument();
    expect(card.getByText("Done — the tests pass.")).toBeInTheDocument();
    expect(card.getByText("Keep the final reply visible and fold its steps behind “Worked for…”. Live work stays open.")).toBeInTheDocument();
  });

  it.each([false, true])("shows the %s miniature and saves the opposite choice without changing other preferences", async (collapseCompletedWork) => {
    const config: AppearanceConfig = {
      ...DEFAULT_APPEARANCE, collapseCompletedWork, thinkingPreview: false, groupExploration: false,
      thinkingTimerPrecision: "tenth", messageBubbles: true, accent: "#b69cff", agentName: "Nova"
    };
    const { onChange, onPreview } = renderSection(config);
    const toggle = screen.getByRole("switch", { name: "Collapse completed work" });
    expect(toggle).toHaveAttribute("aria-checked", String(collapseCompletedWork));
    const card = within(toggle.closest(".chat-preview-card")!);
    if (!collapseCompletedWork) {
      expect(card.queryByText("Worked for 12s")).not.toBeInTheDocument();
      expect(card.getByText("Thought for 2s")).toBeInTheDocument();
      expect(card.getByText("Edited")).toBeInTheDocument();
      expect(card.getByText("Ran")).toBeInTheDocument();
    }
    expect(card.getByText("Done — the tests pass.")).toBeInTheDocument();
    fireEvent.click(toggle);
    await waitFor(() => expect(onChange).toHaveBeenCalledWith({ ...config, collapseCompletedWork: !collapseCompletedWork }));
    expect(onPreview).not.toHaveBeenCalled();
  });
});

describe("AppearanceSection agent name", () => {
  it("shows the stored name", () => {
    renderSection({ agentName: "Nova" });
    expect((screen.getByLabelText("Agent name") as HTMLInputElement).value).toBe("Nova");
  });

  it("an unset name shows the default as the placeholder", () => {
    renderSection();
    expect((screen.getByLabelText("Agent name") as HTMLInputElement).placeholder).toBe("WackCode");
  });

  it("saves a typed name on blur, trimming it", async () => {
    const { onChange } = renderSection();
    const input = screen.getByLabelText("Agent name");
    fireEvent.input(input, { target: { value: "  Nova  " } });
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.blur(input);
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ agentName: "Nova" })));
  });

  it("clearing the field goes back to the default", async () => {
    const { onChange } = renderSection({ agentName: "Nova" });
    const input = screen.getByLabelText("Agent name");
    fireEvent.input(input, { target: { value: "  " } });
    fireEvent.blur(input);
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ agentName: null })));
  });

  it("an unchanged name writes nothing", () => {
    const { onChange } = renderSection({ agentName: "Nova" });
    fireEvent.blur(screen.getByLabelText("Agent name"));
    expect(onChange).not.toHaveBeenCalled();
  });

  it("the chat previews speak with the chosen name", () => {
    renderSection({ agentName: "Nova" });
    expect(screen.getByText(/^Give Nova's replies a bubble/)).toBeInTheDocument();
  });

  it("the miniature draws the draft live, before anything is saved", () => {
    const { onChange } = renderSection();
    fireEvent.input(screen.getByLabelText("Agent name"), { target: { value: "  Nova  " } });
    expect(screen.getByText("Describe the change, bug, or question — Nova can read this project, run commands, and edit files.")).toBeInTheDocument();
    expect(screen.getByText("Ask Nova to inspect, change, or run something…")).toBeInTheDocument();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("an emptied field's miniature falls back to the default name", () => {
    renderSection({ agentName: "Nova" });
    fireEvent.input(screen.getByLabelText("Agent name"), { target: { value: "  " } });
    expect(screen.getByText("Ask WackCode to inspect, change, or run something…")).toBeInTheDocument();
  });
});

describe("dynamic backgrounds", () => {
  it("selects Dynamic without changing the saved image or dynamic preferences", async () => {
    const { onChange } = renderSection({ ...imageLibrary(["kept.png"]), dynamicStyle: "glyphFlow", dynamicPalette: "sunset", dynamicIntensity: 72 });
    fireEvent.click(screen.getByRole("radio", { name: "Dynamic Backgrounds" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ backdrop: "dynamic", backgroundImage: "kept.png", dynamicStyle: "glyphFlow", dynamicPalette: "sunset", dynamicIntensity: 72 })));
  });
  it("labels only Glyph Flow reactive and saves styles and palettes", async () => {
    const { onChange } = renderSection({ backdrop: "dynamic" });
    expect(screen.getByRole("radio", { name: "Colour Drift" })).toHaveAttribute("aria-checked", "true");
    expect(within(screen.getByRole("radio", { name: "Glyph Flow" })).getByText("Reactive")).toBeInTheDocument();
    expect(within(screen.getByRole("radio", { name: "Colour Drift" })).queryByText("Reactive")).toBeNull();
    fireEvent.click(screen.getByRole("radio", { name: "Glyph Flow" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ dynamicStyle: "glyphFlow" })));
    await waitFor(() => expect(screen.getByRole("radio", { name: "Aurora" })).not.toBeDisabled());
    fireEvent.click(screen.getByRole("radio", { name: "Aurora" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ dynamicPalette: "aurora" })));
  });
  it("previews intensity while dragging and commits on release", async () => {
    const { onChange, onPreview } = renderSection({ backdrop: "dynamic" });
    const slider = screen.getByRole("slider", { name: "Intensity" });
    fireEvent.input(slider, { target: { value: "83" } });
    expect(onPreview).toHaveBeenCalledWith(expect.objectContaining({ dynamicIntensity: 83 }));
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.change(slider, { target: { value: "83" } });
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ dynamicIntensity: 83 })));
  });
});
