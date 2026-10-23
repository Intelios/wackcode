import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProviderRecord, SubagentModel } from "../types";
import { ModelPicker, type TeamPillProps } from "./ModelPicker";

const noFavorites = { favoriteModels: [], onSetFavorite: vi.fn().mockResolvedValue(undefined) };

afterEach(cleanup);

const testProviders: ProviderRecord[] = [
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
        id: "claude-opus",
        name: "Claude Opus",
        contextWindow: 200000,
        maxTokens: 8192,
        reasoning: true,
        thinkingLevels: ["off", "low", "high"],
        thinkingLevelMap: { off: null, low: "low", high: "high" },
        vision: false
      },
      {
        id: "claude-haiku",
        name: "Claude Haiku",
        contextWindow: 200000,
        maxTokens: 8192,
        reasoning: true,
        thinkingLevels: ["off", "low"],
        thinkingLevelMap: { off: null, low: "low" },
        vision: false
      }
    ]
  },
  {
    id: "deepseek",
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com/v1",
    apiFormat: "openai-completions",
    createdAt: "2024-01-01",
    updatedAt: "2024-01-01",
    kind: "custom",
    connected: true,
    hasApiKey: true,
    models: [
      {
        id: "deepseek-flash",
        name: "DeepSeek Flash",
        contextWindow: 128000,
        maxTokens: 4096,
        reasoning: false,
        thinkingLevels: ["off"],
        thinkingLevelMap: { off: null },
        vision: false
      }
    ]
  }
];

const crew: SubagentModel = { providerId: "deepseek", modelId: "deepseek-flash", thinkingLevel: "off" };

function team(overrides: Partial<TeamPillProps> = {}): TeamPillProps {
  return {
    team: null,
    teamDefault: null,
    onOpenSettings: vi.fn(),
    onTeamChange: vi.fn(),
    ...overrides
  };
}

function picker(props: TeamPillProps) {
  return render(
    <ModelPicker {...noFavorites}
      providers={testProviders}
      providerId="anthropic"
      modelId="claude-opus"
      onConfigure={() => undefined}
      team={props}
    />
  );
}

describe("ModelPicker team", () => {
  it("shows the Team switch and hint in the popover footer", () => {
    picker(team());
    fireEvent.click(screen.getByRole("button", { name: /Claude Opus/i }));
    expect(screen.getByRole("switch", { name: "Team" })).toBeInTheDocument();
    expect(screen.getByText(/the crew does the work/i)).toBeInTheDocument();
    // No crew yet: no second segment on the pill.
    expect(screen.queryByRole("button", { name: /Team crew/i })).toBeNull();
  });

  it("hides everything without a team prop (chat-mode composers)", () => {
    render(
      <ModelPicker {...noFavorites}
        providers={testProviders}
        providerId="anthropic"
        modelId="claude-opus"
        onConfigure={() => undefined}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /Claude Opus/i }));
    expect(screen.queryByRole("switch", { name: "Team" })).toBeNull();
    expect(screen.queryByText(/Team crew/i)).toBeNull();
  });

  it("turns on with the settings default, and off again", () => {
    const onTeamChange = vi.fn();
    picker(team({ onTeamChange, teamDefault: crew }));
    fireEvent.click(screen.getByRole("button", { name: /Claude Opus/i }));
    fireEvent.click(screen.getByRole("switch", { name: "Team" }));
    expect(onTeamChange).toHaveBeenCalledWith(crew);
  });

  it("falls back to the first available model when there is no default", () => {
    const onTeamChange = vi.fn();
    picker(team({ onTeamChange }));
    fireEvent.click(screen.getByRole("button", { name: /Claude Opus/i }));
    fireEvent.click(screen.getByRole("switch", { name: "Team" }));
    expect(onTeamChange).toHaveBeenCalledWith(expect.objectContaining({ providerId: "anthropic", modelId: "claude-opus" }));
  });

  it("disables the switch and links to Settings while the crew can't run", () => {
    const onOpenSettings = vi.fn();
    picker(team({ teamAvailable: "Sub-agents are off in Settings › Sub-agents", onOpenSettings }));
    fireEvent.click(screen.getByRole("button", { name: /Claude Opus/i }));
    expect(screen.getByRole("switch", { name: "Team" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Open Settings" }));
    expect(onOpenSettings).toHaveBeenCalled();
  });

  it("renders the split pill while a chat carries a crew, and the crew segment opens its view", () => {
    picker(team({ team: crew }));
    const segment = screen.getByRole("button", { name: /Team crew: DeepSeek Flash/i });
    expect(segment).toBeInTheDocument();
    fireEvent.click(segment);
    // The crew view lists models across providers, without favourites stars.
    expect(screen.getByRole("button", { name: /^Claude Haiku/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^DeepSeek Flash/i })).toBeInTheDocument();
  });

  it("marks a stale crew segment but still lets the switch turn Team off", () => {
    const onTeamChange = vi.fn();
    picker(team({ team: crew, teamAvailable: "Its connection is turned off.", onTeamChange }));
    expect(screen.getByRole("button", { name: /Team crew: Its connection is turned off/i })).toHaveClass("stale");
    fireEvent.click(screen.getByRole("button", { name: /Claude Opus/i }));
    const toggle = screen.getByRole("switch", { name: "Team" });
    expect(toggle).not.toBeDisabled();
    fireEvent.click(toggle);
    expect(onTeamChange).toHaveBeenCalledWith(null);
  });

  it("changes the crew model and reasoning inside the crew view", () => {
    const onTeamChange = vi.fn();
    picker(team({ team: crew, onTeamChange }));
    fireEvent.click(screen.getByRole("button", { name: /Team crew: DeepSeek Flash/i }));
    fireEvent.click(screen.getByRole("button", { name: /^Claude Haiku/i }));
    expect(onTeamChange).toHaveBeenCalledWith(expect.objectContaining({ providerId: "anthropic", modelId: "claude-haiku" }));
  });
});
