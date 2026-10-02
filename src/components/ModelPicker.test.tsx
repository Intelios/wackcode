import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProviderRecord } from "../types";
import { ModelPicker, ReasoningToggle } from "./ModelPicker";

const noFavorites = { favoriteModels: [], onSetFavorite: vi.fn().mockResolvedValue(undefined) };

afterEach(cleanup);

const testProviders: ProviderRecord[] = [
  {
    id: "openrouter",
    name: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    apiFormat: "openai-completions",
    createdAt: "2024-01-01",
    updatedAt: "2024-01-01",
    kind: "custom",
    connected: true,
    hasApiKey: true,
    models: [
      {
        id: "anthropic/claude-3.5-sonnet",
        name: "Claude 3.5 Sonnet",
        contextWindow: 200000,
        maxTokens: 8192,
        reasoning: true,
        thinkingLevels: ["off", "low", "medium", "high", "max"],
        thinkingLevelMap: { off: null, low: "low", medium: "medium", high: "high", max: "max" },
        vision: false
      },
      {
        id: "openai/gpt-4o",
        name: "GPT-4o",
        contextWindow: 128000,
        maxTokens: 4096,
        reasoning: false,
        thinkingLevels: ["off"],
        thinkingLevelMap: { off: null },
        vision: false
      }
    ]
  },
  {
    id: "anthropic",
    name: "Anthropic",
    baseUrl: "https://api.anthropic.com/v1",
    apiFormat: "openai-responses",
    createdAt: "2024-01-01",
    updatedAt: "2024-01-01",
    kind: "custom",
    connected: true,
    hasApiKey: true,
    models: [
      {
        id: "claude-3-opus",
        name: "Claude 3 Opus",
        contextWindow: 200000,
        maxTokens: 4096,
        reasoning: false,
        thinkingLevels: ["off"],
        thinkingLevelMap: { off: null },
        vision: false
      }
    ]
  }
];

describe("ModelPicker", () => {
  it("renders only the model name in the pill button", () => {
    render(
      <ModelPicker {...noFavorites}
        providers={testProviders}
        providerId="openrouter"
        modelId="anthropic/claude-3.5-sonnet"
        onConfigure={() => undefined}
      />
    );

    const button = screen.getByRole("button", { name: /Claude 3.5 Sonnet/i });
    expect(button).toBeInTheDocument();
    expect(screen.queryByText(/reasoning/i)).toBeNull();
  });

  it("auto-navigates to the active provider's models on open", () => {
    render(
      <ModelPicker {...noFavorites}
        providers={testProviders}
        providerId="openrouter"
        modelId="anthropic/claude-3.5-sonnet"
        onConfigure={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /Claude 3.5 Sonnet/i }));

    // The other sliding panel is hidden from keyboard and accessibility navigation.
    expect(screen.getByRole("button", { name: "Back to providers" })).toHaveTextContent("OpenRouter");
    expect(screen.queryByRole("button", { name: /Favourites/ })).not.toBeInTheDocument();
    // Models for OpenRouter should be visible
    expect(screen.getByRole("button", { name: "GPT-4o" })).toBeInTheDocument();
  });

  it("allows navigating back to provider list and selecting another provider", () => {
    const onConfigure = vi.fn();
    render(
      <ModelPicker {...noFavorites}
        providers={testProviders}
        providerId="openrouter"
        modelId="anthropic/claude-3.5-sonnet"
        onConfigure={onConfigure}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /Claude 3.5 Sonnet/i }));

    // Click back to providers
    fireEvent.click(screen.getByLabelText("Back to providers"));

    // Providers panel should now show Anthropic and OpenRouter
    expect(screen.getByText("Providers")).toBeInTheDocument();
    const anthropicBtn = screen.getByRole("button", { name: /Anthropic/i });
    expect(anthropicBtn).toBeInTheDocument();

    // Click Anthropic
    fireEvent.click(anthropicBtn);

    // Now Anthropic's model should be listed
    const opusBtn = screen.getByRole("button", { name: "Claude 3 Opus" });
    expect(opusBtn).toBeInTheDocument();

    // Select Claude 3 Opus
    fireEvent.click(opusBtn);
    expect(onConfigure).toHaveBeenCalledWith({
      providerId: "anthropic",
      modelId: "claude-3-opus"
    });
  });
});

describe("ModelPicker favourites", () => {
  const opus = { providerId: "anthropic", modelId: "claude-3-opus" };
  const gpt = { providerId: "openrouter", modelId: "openai/gpt-4o" };

  function openFavorites() {
    fireEvent.click(screen.getByRole("button", { name: "Choose model" }));
    fireEvent.click(screen.getByRole("button", { name: /^Favourites/ }));
    return screen.getByRole("dialog", { name: "Choose model" });
  }

  it("shows the available count, current names and providers, and selects across providers", () => {
    const onConfigure = vi.fn();
    render(<ModelPicker {...noFavorites} providers={testProviders} favoriteModels={[gpt, opus]} onConfigure={onConfigure} />);
    const trigger = screen.getByRole("button", { name: "Choose model" });
    fireEvent.click(trigger);
    expect(screen.getByRole("button", { name: "Favourites 2" })).toHaveFocus();
    fireEvent.click(screen.getByRole("button", { name: "Favourites 2" }));
    const dialog = screen.getByRole("dialog");
    const selections = within(dialog).getAllByRole("button", { pressed: false }).filter((button) => !button.hasAttribute("title"));
    expect(selections.map((button) => button.textContent)).toEqual(["Claude 3 OpusAnthropic", "GPT-4oOpenRouter"]);
    fireEvent.click(within(dialog).getByRole("button", { name: "Claude 3 Opus (Anthropic)" }));
    expect(onConfigure).toHaveBeenCalledWith(opus);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });

  it("stars and unstars provider models without selecting them or closing the picker", () => {
    const onConfigure = vi.fn();
    const onSetFavorite = vi.fn().mockResolvedValue(undefined);
    const props = { providers: testProviders, providerId: "openrouter", modelId: gpt.modelId, onConfigure, onSetFavorite };
    const view = render(<ModelPicker {...props} favoriteModels={[]} />);
    fireEvent.click(screen.getByRole("button", { name: "GPT-4o" }));
    const star = screen.getByRole("button", { name: "Add GPT-4o (OpenRouter) to favourites" });
    expect(star).toHaveAttribute("aria-pressed", "false");
    expect(star.parentElement?.querySelector("button button")).toBeNull();
    fireEvent.click(star);
    expect(onSetFavorite).toHaveBeenCalledWith(gpt, true);
    expect(onConfigure).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    view.rerender(<ModelPicker {...props} favoriteModels={[gpt]} />);
    const remove = screen.getByRole("button", { name: "Remove GPT-4o (OpenRouter) from favourites" });
    expect(remove).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(remove);
    expect(onSetFavorite).toHaveBeenLastCalledWith(gpt, false);
    expect(onConfigure).not.toHaveBeenCalled();
  });

  it("keeps the current provider opening behaviour even after visiting Favourites", () => {
    render(<ModelPicker {...noFavorites} providers={testProviders} favoriteModels={[opus]} providerId="openrouter" modelId={gpt.modelId} onConfigure={vi.fn()} />);
    const trigger = screen.getByRole("button", { name: "GPT-4o" });
    fireEvent.click(trigger);
    expect(screen.getByRole("button", { name: "Back to providers" })).toHaveTextContent("OpenRouter");
    fireEvent.click(screen.getByRole("button", { name: "Back to providers" }));
    fireEvent.click(screen.getByRole("button", { name: "Favourites 1" }));
    expect(document.querySelector(".picker-panel-providers")).toHaveAttribute("inert");
    expect(screen.queryByRole("button", { name: "Favourites 1" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Back to providers" }));
    expect(screen.getByRole("button", { name: "Favourites 1" })).toHaveFocus();
    expect(document.querySelector(".picker-panel-models")).toHaveAttribute("inert");
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
    fireEvent.click(trigger);
    expect(screen.getByRole("button", { name: "Back to providers" })).toHaveTextContent("OpenRouter");
  });

  it("moves focus to the next row after removal and keeps the last empty view open", () => {
    const onSetFavorite = vi.fn().mockResolvedValue(undefined);
    const props = { providers: testProviders, onConfigure: vi.fn(), onSetFavorite };
    const view = render(<ModelPicker {...props} favoriteModels={[opus, gpt]} />);
    openFavorites();
    const removeOpus = screen.getByRole("button", { name: "Remove Claude 3 Opus (Anthropic) from favourites" });
    removeOpus.focus();
    fireEvent.click(removeOpus);
    view.rerender(<ModelPicker {...props} favoriteModels={[gpt]} />);
    expect(screen.getByRole("button", { name: "GPT-4o (OpenRouter)" })).toHaveFocus();
    const removeGpt = screen.getByRole("button", { name: "Remove GPT-4o (OpenRouter) from favourites" });
    removeGpt.focus();
    fireEvent.click(removeGpt);
    view.rerender(<ModelPicker {...props} favoriteModels={[]} />);
    expect(screen.getByRole("button", { name: "Back to providers" })).toHaveFocus();
    expect(screen.getByRole("status")).toHaveTextContent("No favourites yet");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(props.onConfigure).not.toHaveBeenCalled();
  });

  it("keeps Favourites available when empty, and restores unavailable references", () => {
    const props = { ...noFavorites, onConfigure: vi.fn() };
    const view = render(<ModelPicker {...props} providers={[]} />);
    openFavorites();
    expect(screen.getByRole("status")).toHaveTextContent("Star a model in any provider");
    view.rerender(<ModelPicker {...props} providers={[]} favoriteModels={[opus]} />);
    expect(screen.getByRole("status")).toHaveTextContent("No favourites available");
    view.rerender(<ModelPicker {...props} providers={testProviders} favoriteModels={[opus]} />);
    expect(screen.getByRole("button", { name: "Claude 3 Opus (Anthropic)" })).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("disables star actions during saves and leaves the list intact after a failure", async () => {
    const onSetFavorite = vi.fn().mockRejectedValue("Could not save favourites.");
    const props = { providers: testProviders, favoriteModels: [opus], onSetFavorite, onConfigure: vi.fn() };
    const view = render(<ModelPicker {...props} favoriteSaving />);
    openFavorites();
    const star = screen.getByRole("button", { name: /Remove Claude 3 Opus/ });
    expect(star).toBeDisabled();
    expect(screen.getByRole("button", { name: "Claude 3 Opus (Anthropic)" })).toBeEnabled();
    fireEvent.click(star);
    expect(onSetFavorite).not.toHaveBeenCalled();
    view.rerender(<ModelPicker {...props} favoriteSaving={false} />);
    await act(async () => { fireEvent.click(star); });
    expect(onSetFavorite).toHaveBeenCalledWith(opus, false);
    expect(star).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("distinguishes identical model IDs and names, including a provider named favourites", () => {
    const providers = testProviders.map((provider, index) => ({
      ...provider, id: index ? "favorites" : provider.id,
      models: [{ ...provider.models[0], id: "same", name: "Same model" }]
    }));
    const references = providers.map((provider) => ({ providerId: provider.id, modelId: "same" }));
    const onConfigure = vi.fn();
    const onSetFavorite = vi.fn().mockResolvedValue(undefined);
    render(<ModelPicker providers={providers} favoriteModels={references} onConfigure={onConfigure} onSetFavorite={onSetFavorite} />);
    openFavorites();
    fireEvent.click(screen.getByRole("button", { name: "Remove Same model (Anthropic) from favourites" }));
    expect(onSetFavorite).toHaveBeenCalledWith(references[1], false);
    fireEvent.click(screen.getByRole("button", { name: "Same model (Anthropic)" }));
    expect(onConfigure).toHaveBeenCalledWith(references[1]);
  });
});

describe("ReasoningToggle", () => {
  it("does not render when the model only supports 'off'", () => {
    const { container } = render(
      <ReasoningToggle
        providers={testProviders}
        providerId="openrouter"
        modelId="openai/gpt-4o"
        thinkingLevel="off"
        onConfigure={() => undefined}
      />
    );

    expect(container.firstChild).toBeNull();
  });

  it("renders when the model supports multiple thinking levels", () => {
    render(
      <ReasoningToggle
        providers={testProviders}
        providerId="openrouter"
        modelId="anthropic/claude-3.5-sonnet"
        thinkingLevel="medium"
        onConfigure={() => undefined}
      />
    );

    const toggle = screen.getByRole("button", { name: /medium/i });
    expect(toggle).toBeInTheDocument();
    expect(toggle).toHaveClass("active");
  });

  it("opens the discrete slider popover and allows picking levels", () => {
    const onConfigure = vi.fn();
    render(
      <ReasoningToggle
        providers={testProviders}
        providerId="openrouter"
        modelId="anthropic/claude-3.5-sonnet"
        thinkingLevel="medium"
        onConfigure={onConfigure}
      />
    );

    // Click toggle to open popover
    fireEvent.click(screen.getByRole("button", { name: /medium/i }));

    // Header shows the active level and the model name
    expect(document.querySelector(".reasoning-level-name")).toHaveTextContent("medium");
    expect(screen.getByText("Claude 3.5 Sonnet")).toBeInTheDocument();

    // Tick buttons should be visible
    expect(screen.getByRole("button", { name: "low" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "max" })).toBeInTheDocument();

    // Click "max" tick button
    fireEvent.click(screen.getByRole("button", { name: "max" }));
    expect(onConfigure).toHaveBeenCalledWith({ thinkingLevel: "max" });

    // Move range slider
    const slider = screen.getByLabelText("Reasoning effort");
    fireEvent.change(slider, { target: { value: "1" } }); // index 1 corresponds to "low"
    expect(onConfigure).toHaveBeenCalledWith({ thinkingLevel: "low" });
  });

  it("shows the max-level effect only at the topmost level", () => {
    render(
      <ReasoningToggle
        providers={testProviders}
        providerId="openrouter"
        modelId="anthropic/claude-3.5-sonnet"
        thinkingLevel="max"
        onConfigure={() => undefined}
      />
    );

    const toggle = screen.getByRole("button", { name: /max/i });
    expect(toggle).toHaveClass("at-max");

    fireEvent.click(toggle);

    expect(document.querySelector(".reasoning-panel")).toHaveClass("at-max");
    expect(document.querySelector(".reasoning-max-fx")).not.toBeNull();
  });

  it("does not show the max-level effect below the topmost level", () => {
    render(
      <ReasoningToggle
        providers={testProviders}
        providerId="openrouter"
        modelId="anthropic/claude-3.5-sonnet"
        thinkingLevel="high"
        onConfigure={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /high/i }));

    expect(document.querySelector(".reasoning-panel")).not.toHaveClass("at-max");
    expect(document.querySelector(".reasoning-max-fx")).toBeNull();
  });
});
