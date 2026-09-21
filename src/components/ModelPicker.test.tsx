import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProviderRecord } from "../types";
import { ModelPicker, ReasoningToggle } from "./ModelPicker";

afterEach(cleanup);

const testProviders: ProviderRecord[] = [
  {
    id: "openrouter",
    name: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    apiFormat: "openai-completions",
    createdAt: "2024-01-01",
    updatedAt: "2024-01-01",
    hasApiKey: true,
    models: [
      {
        id: "anthropic/claude-3.5-sonnet",
        name: "Claude 3.5 Sonnet",
        contextWindow: 200000,
        maxTokens: 8192,
        reasoning: true,
        thinkingLevels: ["off", "low", "medium", "high", "max"],
        thinkingLevelMap: { off: null, low: "low", medium: "medium", high: "high", max: "max" }
      },
      {
        id: "openai/gpt-4o",
        name: "GPT-4o",
        contextWindow: 128000,
        maxTokens: 4096,
        reasoning: false,
        thinkingLevels: ["off"],
        thinkingLevelMap: { off: null }
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
    hasApiKey: true,
    models: [
      {
        id: "claude-3-opus",
        name: "Claude 3 Opus",
        contextWindow: 200000,
        maxTokens: 4096,
        reasoning: false,
        thinkingLevels: ["off"],
        thinkingLevelMap: { off: null }
      }
    ]
  }
];

describe("ModelPicker", () => {
  it("renders only the model name in the pill button", () => {
    render(
      <ModelPicker
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
      <ModelPicker
        providers={testProviders}
        providerId="openrouter"
        modelId="anthropic/claude-3.5-sonnet"
        onConfigure={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /Claude 3.5 Sonnet/i }));

    // Back button with provider name should be visible
    expect(screen.getByRole("button", { name: /OpenRouter/i })).toBeInTheDocument();
    // Models for OpenRouter should be visible
    expect(screen.getByRole("button", { name: "GPT-4o" })).toBeInTheDocument();
  });

  it("allows navigating back to provider list and selecting another provider", () => {
    const onConfigure = vi.fn();
    render(
      <ModelPicker
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
