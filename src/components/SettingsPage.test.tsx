import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { AppearanceConfig, BuiltinModelSuggestion, PromptConfig, ProviderRecord, SubagentConfig, ToolCatalogEntry } from "../types";
import { DEFAULT_APPEARANCE } from "../theme";
import { SettingsPage } from "./SettingsPage";

vi.mock("../api", () => ({ api: {
  revealPath: vi.fn().mockResolvedValue(undefined),
  listBuiltinModels: vi.fn().mockResolvedValue([]),
  listSubscriptionProviders: vi.fn().mockResolvedValue([])
} }));

const noSubagents: SubagentConfig = { enabled: false, trigger: "on_request", maxConcurrency: 4, agents: [] };
const defaultAppearance: AppearanceConfig = DEFAULT_APPEARANCE;
const defaultPrompts: PromptConfig = {};

afterEach(() => {
  cleanup();
  vi.mocked(api.listBuiltinModels).mockReset().mockResolvedValue([]);
  vi.mocked(api.listSubscriptionProviders).mockReset().mockResolvedValue([]);
});

it("shows subscription guidance before sign-in and reconnect", async () => {
  vi.mocked(api.listSubscriptionProviders).mockResolvedValue([{ id: "anthropic", name: "Anthropic", guidance: "Claude usage may be billed separately." }]);
  const onConnectSubscription = vi.fn().mockResolvedValue(undefined);
  const props = {
    packages: [], toolCatalog: [], disabledTools: [], appDataPath: "/tmp/wackcode",
    onClose: vi.fn(), onSave: vi.fn(), onDelete: vi.fn(), onConnectSubscription,
    onSignOutSubscription: vi.fn(), onSetDisabledTools: vi.fn(), subagents: noSubagents, onSetSubagents: vi.fn(), autoTitle: { enabled: false, providerId: null, modelId: null }, onSetAutoTitle: vi.fn(), appearance: defaultAppearance, glassSupported: true, onSetAppearance: vi.fn(), onPreviewAppearance: vi.fn(), prompts: defaultPrompts, onSetPrompts: vi.fn(),
    onRefresh: vi.fn().mockResolvedValue(undefined), onInstall: vi.fn(), onTrust: vi.fn(),
    onSearch: vi.fn().mockResolvedValue([]), onRemove: vi.fn(), onUpdate: vi.fn(), onSetResources: vi.fn()
  };
  const { rerender } = render(<SettingsPage {...props} providers={[]} />);
  fireEvent.click(screen.getByRole("button", { name: "Sign in with a subscription" }));
  expect(await screen.findByText("Claude usage may be billed separately.")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
  expect(onConnectSubscription).toHaveBeenCalledWith("anthropic");

  const provider: ProviderRecord = {
    id: "anthropic", name: "Anthropic", kind: "subscription", baseUrl: "", apiFormat: "",
    models: [], createdAt: "now", updatedAt: "now", hasApiKey: false, connected: true
  };
  rerender(<SettingsPage {...props} providers={[provider]} connectedSubscriptionId="anthropic" />);
  expect(screen.getByText("Claude usage may be billed separately.")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
  expect(onConnectSubscription).toHaveBeenLastCalledWith("anthropic");
});

const catalog: ToolCatalogEntry[] = [
  { name: "read", description: "Read a file", source: { kind: "builtin" }, available: true },
  { name: "bash", description: "Run a command", source: { kind: "builtin" }, available: true },
  { name: "find", description: "Find files", source: { kind: "builtin" }, available: false, unavailableReason: "Requires fd, which is not installed" },
  { name: "web_search", description: "Search the web", source: { kind: "package", packageId: "npm:pi-web-access" }, available: true }
];

function renderTools(overrides: { disabled?: string[]; onSetDisabledTools?: (next: string[]) => Promise<void> } = {}) {
  const onSetDisabledTools = overrides.onSetDisabledTools ?? vi.fn().mockResolvedValue(undefined);
  render(
    <SettingsPage
      providers={[]}
      packages={[]}
      toolCatalog={catalog}
      disabledTools={overrides.disabled ?? []}
      appDataPath="/tmp/wackcode"
      onClose={vi.fn()}
      onSave={vi.fn()}
      onDelete={vi.fn()}
      onConnectSubscription={vi.fn()}
      onSignOutSubscription={vi.fn()}
      onSetDisabledTools={onSetDisabledTools} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()}
      onRefresh={vi.fn().mockResolvedValue(undefined)}
      onInstall={vi.fn()}
      onTrust={vi.fn()}
      onSearch={vi.fn().mockResolvedValue([])}
      onRemove={vi.fn()}
      onUpdate={vi.fn()}
      onSetResources={vi.fn()}
    />
  );
  fireEvent.click(screen.getByRole("button", { name: "Tools" }));
  return { onSetDisabledTools };
}

describe("SettingsPage tools section", () => {
  it("groups built-ins separately from package tools", () => {
    renderTools();
    expect(screen.getByRole("heading", { name: "Built-in" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "npm:pi-web-access" })).toBeInTheDocument();
  });

  it("switching a tool off sends it as a denylist entry", async () => {
    const { onSetDisabledTools } = renderTools();
    fireEvent.click(screen.getByRole("switch", { name: "bash" }));
    await waitFor(() => expect(onSetDisabledTools).toHaveBeenCalledWith(["bash"]));
  });

  it("switching an already-disabled tool back on removes it from the denylist", async () => {
    const { onSetDisabledTools } = renderTools({ disabled: ["bash"] });
    expect(screen.getByRole("switch", { name: "bash" })).toHaveAttribute("aria-checked", "false");
    fireEvent.click(screen.getByRole("switch", { name: "bash" }));
    await waitFor(() => expect(onSetDisabledTools).toHaveBeenCalledWith([]));
  });

  it("a tool whose binary is missing is off, locked, and explains why", () => {
    renderTools();
    const find = screen.getByRole("switch", { name: "find" });
    expect(find).toBeDisabled();
    expect(find).toHaveAttribute("aria-checked", "false");
    expect(screen.getByText("Requires fd, which is not installed")).toBeInTheDocument();
  });

  it("surfaces a failed save instead of silently reverting", async () => {
    const onSetDisabledTools = vi.fn().mockRejectedValue(new Error("Metadata lock was poisoned"));
    renderTools({ onSetDisabledTools });
    fireEvent.click(screen.getByRole("switch", { name: "read" }));
    expect(await screen.findByText(/Metadata lock was poisoned/)).toBeInTheDocument();
  });
});

const testProviders: ProviderRecord[] = [
  {
    id: "p1",
    name: "Entrim AI",
    baseUrl: "https://api.entrim.ai/v1",
    apiFormat: "openai-completions",
    models: [],
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    kind: "custom",
    connected: true,
    hasApiKey: true
  },
  {
    id: "p2",
    name: "Test",
    baseUrl: "https://api.test.com/v1",
    apiFormat: "openai-completions",
    models: [],
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    kind: "custom",
    connected: false,
    hasApiKey: false
  }
];

describe("SettingsPage sidebar navigation", () => {
  it("renders providers subnav directly under the Providers button and before Packages/Tools", () => {
    render(
      <SettingsPage
        providers={testProviders}
        packages={[]}
        toolCatalog={catalog}
        disabledTools={[]}
        appDataPath="/tmp/wackcode"
        onClose={vi.fn()}
        onSave={vi.fn()}
        onDelete={vi.fn()}
        onConnectSubscription={vi.fn()}
        onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()}
        onRefresh={vi.fn().mockResolvedValue(undefined)}
        onInstall={vi.fn()}
        onTrust={vi.fn()}
        onSearch={vi.fn().mockResolvedValue([])}
        onRemove={vi.fn()}
        onUpdate={vi.fn()}
        onSetResources={vi.fn()}
      />
    );

    const nav = screen.getByRole("navigation", { name: "Settings sections" });
    const buttons = Array.from(nav.querySelectorAll("button")).map((btn) => btn.textContent?.trim());

    // Expected order: Providers -> Entrim AI -> Test -> New connection -> Packages -> Tools -> Appearance -> Prompts
    expect(buttons).toEqual([
      "Providers",
      "Entrim AI",
      "Test",
      "New connection",
      "Packages",
      "Tools",
      "Auto titles",
      "Appearance",
      "Prompts"
    ]);
  });

  it("toggles provider subnav animation state and aria attributes when switching sections", () => {
    const { container } = render(
      <SettingsPage
        providers={testProviders}
        packages={[]}
        toolCatalog={catalog}
        disabledTools={[]}
        appDataPath="/tmp/wackcode"
        onClose={vi.fn()}
        onSave={vi.fn()}
        onDelete={vi.fn()}
        onConnectSubscription={vi.fn()}
        onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()}
        onRefresh={vi.fn().mockResolvedValue(undefined)}
        onInstall={vi.fn()}
        onTrust={vi.fn()}
        onSearch={vi.fn().mockResolvedValue([])}
        onRemove={vi.fn()}
        onUpdate={vi.fn()}
        onSetResources={vi.fn()}
      />
    );

    const providersBtn = screen.getByRole("button", { name: "Providers" });
    const subnavWrapper = container.querySelector(".settings-subnav-wrapper");

    expect(providersBtn).toHaveAttribute("aria-expanded", "true");
    expect(subnavWrapper).toHaveClass("expanded");
    expect(subnavWrapper).toHaveAttribute("aria-hidden", "false");
    expect(screen.getByRole("button", { name: /Entrim AI/ })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Packages" }));
    expect(providersBtn).toHaveAttribute("aria-expanded", "false");
    expect(subnavWrapper).not.toHaveClass("expanded");
    expect(subnavWrapper).toHaveAttribute("aria-hidden", "true");
    expect(screen.queryByRole("button", { name: /Entrim AI/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /New connection/ })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Providers" }));
    expect(providersBtn).toHaveAttribute("aria-expanded", "true");
    expect(subnavWrapper).toHaveClass("expanded");
    expect(subnavWrapper).toHaveAttribute("aria-hidden", "false");
    expect(screen.getByRole("button", { name: /Entrim AI/ })).toBeInTheDocument();
  });

  it("calls onClose when clicking the settings back button containing icon and Settings label", () => {
    const onClose = vi.fn();
    render(
      <SettingsPage
        providers={testProviders}
        packages={[]}
        toolCatalog={catalog}
        disabledTools={[]}
        appDataPath="/tmp/wackcode"
        onClose={onClose}
        onSave={vi.fn()}
        onDelete={vi.fn()}
        onConnectSubscription={vi.fn()}
        onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()}
        onRefresh={vi.fn().mockResolvedValue(undefined)}
        onInstall={vi.fn()}
        onTrust={vi.fn()}
        onSearch={vi.fn().mockResolvedValue([])}
        onRemove={vi.fn()}
        onUpdate={vi.fn()}
        onSetResources={vi.fn()}
      />
    );

    const backBtn = screen.getByRole("button", { name: "Back to chats" });
    expect(backBtn).toHaveTextContent("Settings");
    fireEvent.click(backBtn);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});



describe("SettingsPage model capabilities", () => {
  it("starts Vision off and saves it once the user turns it on", async () => {
    const provider: ProviderRecord = {
      ...testProviders[0],
      models: [{
        id: "vision-model", name: "Vision model", contextWindow: 8000, maxTokens: 1000,
        reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null }, vision: false
      }]
    };
    const onSave = vi.fn().mockImplementation(async (input) => ({ ...provider, models: input.models }));
    render(
      <SettingsPage
        providers={[provider]}
        packages={[]}
        toolCatalog={catalog}
        disabledTools={[]}
        appDataPath="/tmp/wackcode"
        onClose={vi.fn()}
        onSave={onSave}
        onDelete={vi.fn()}
        onConnectSubscription={vi.fn()}
        onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()}
        onRefresh={vi.fn().mockResolvedValue(undefined)}
        onInstall={vi.fn()}
        onTrust={vi.fn()}
        onSearch={vi.fn().mockResolvedValue([])}
        onRemove={vi.fn()}
        onUpdate={vi.fn()}
        onSetResources={vi.fn()}
      />
    );
    const toggle = screen.getByRole("switch", { name: "Vision for Vision model" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("button", { name: "Save connection" }));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0][0].models[0].vision).toBe(true);
  });
});

const flashSuggestion: BuiltinModelSuggestion = {
  sourceProvider: "deepseek", sourceApi: "openai-completions", id: "deepseek-flash", name: "DeepSeek V4.1 Flash",
  contextWindow: 1_000_000, maxTokens: 384_000, reasoning: true,
  thinkingLevels: ["off", "low", "high", "max"],
  thinkingLevelMap: { off: null, low: "low", high: "high", max: "max" }, vision: true
};

function renderModelSettings(provider: ProviderRecord = testProviders[0]) {
  const onSave = vi.fn().mockImplementation(async (input) => ({ ...provider, models: input.models }));
  const view = render(
    <SettingsPage
      providers={[provider]} packages={[]} toolCatalog={catalog} disabledTools={[]}
      appDataPath="/tmp/wackcode" onClose={vi.fn()} onSave={onSave} onDelete={vi.fn()}
      onConnectSubscription={vi.fn()} onSignOutSubscription={vi.fn()}
      onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()} onRefresh={vi.fn()} onInstall={vi.fn()} onTrust={vi.fn()}
      onSearch={vi.fn()} onRemove={vi.fn()} onUpdate={vi.fn()} onSetResources={vi.fn()}
    />
  );
  return { ...view, onSave };
}

describe("SettingsPage Pi catalogue suggestions", () => {
  it("shows sourced matches and fills a blank manual model only after selection", async () => {
    vi.mocked(api.listBuiltinModels).mockResolvedValue([
      flashSuggestion,
      { ...flashSuggestion, sourceProvider: "openrouter", id: "deepseek/deepseek-flash" }
    ]);
    const { onSave } = renderModelSettings();
    fireEvent.click(screen.getByRole("button", { name: "Add manually" }));
    const search = screen.getByRole("combobox", { name: /Find in Pi catalogue/ });
    fireEvent.change(search, { target: { value: "Deepseek V4 Flash" } });
    const options = await screen.findAllByRole("option");
    expect(options).toHaveLength(2);
    expect(options[0]).toHaveTextContent("deepseek · openai-completions · deepseek-flash");
    expect(options[1]).toHaveTextContent("openrouter");
    expect(screen.getByRole("textbox", { name: "Model ID" })).toHaveValue("");
    expect(screen.getByRole("spinbutton", { name: "Context tokens" })).toHaveValue(null);
    expect(onSave).not.toHaveBeenCalled();

    fireEvent.click(options[0]);
    expect(screen.getByRole("textbox", { name: "Model ID" })).toHaveValue("deepseek-flash");
    expect(screen.getByRole("textbox", { name: "Display name" })).toHaveValue("DeepSeek V4.1 Flash");
    expect(screen.getByRole("spinbutton", { name: "Context tokens" })).toHaveValue(1_000_000);
    expect(screen.getByRole("spinbutton", { name: "Max output tokens" })).toHaveValue(384_000);
    expect(screen.getByRole("switch", { name: /Vision for DeepSeek/ })).toHaveAttribute("aria-checked", "true");
    expect(onSave).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Save connection" }));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0][0].models[0].thinkingLevels).toEqual(["off", "low", "high", "max"]);
  });

  it("preserves a discovered provider ID and supports keyboard selection", async () => {
    vi.mocked(api.listBuiltinModels).mockResolvedValue([flashSuggestion]);
    const provider: ProviderRecord = {
      ...testProviders[0], models: [{ id: "gateway/deepseek-flash", name: "gateway/deepseek-flash",
        contextWindow: null, maxTokens: null, reasoning: false, thinkingLevels: ["off"],
        thinkingLevelMap: { off: null }, vision: false }]
    };
    renderModelSettings(provider);
    const search = screen.getByRole("combobox", { name: /Find in Pi catalogue/ });
    fireEvent.change(search, { target: { value: "Deepseek V4 Flash" } });
    await screen.findByRole("option");
    fireEvent.keyDown(search, { key: "Escape" });
    expect(screen.queryByRole("option")).not.toBeInTheDocument();
    expect(screen.getByRole("spinbutton", { name: "Context tokens" })).toHaveValue(null);
    fireEvent.keyDown(search, { key: "ArrowDown" });
    expect(screen.getByRole("combobox", { name: /Find in Pi catalogue/ })).toHaveAttribute("aria-expanded", "true");
    fireEvent.keyDown(search, { key: "Enter" });
    expect(screen.getByRole("textbox", { name: "Model ID" })).toHaveValue("gateway/deepseek-flash");
    expect(screen.getByRole("spinbutton", { name: "Context tokens" })).toHaveValue(1_000_000);
  });

  it("keeps manual entry available when there is no match or the catalogue fails", async () => {
    const { unmount } = renderModelSettings();
    fireEvent.click(screen.getByRole("button", { name: "Add manually" }));
    fireEvent.change(screen.getByRole("combobox", { name: /Find in Pi catalogue/ }), { target: { value: "unknown" } });
    expect(await screen.findByText(/No Pi catalogue matches/)).toBeInTheDocument();
    unmount();

    vi.mocked(api.listBuiltinModels).mockRejectedValue(new Error("Catalogue unavailable"));
    renderModelSettings();
    fireEvent.click(screen.getByRole("button", { name: "Add manually" }));
    expect(await screen.findByText(/Could not load Pi catalogue: Error: Catalogue unavailable/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "Model ID" }), { target: { value: "my-model" } });
    expect(screen.getByRole("textbox", { name: "Model ID" })).toHaveValue("my-model");
  });
});

describe("SettingsPage sub-agents", () => {
  function renderPage(enabled: boolean) {
    const props = {
      providers: [], packages: [], toolCatalog: [], disabledTools: [], appDataPath: "/tmp/wackcode",
      onClose: vi.fn(), onSave: vi.fn(), onDelete: vi.fn(), onConnectSubscription: vi.fn(),
      onSignOutSubscription: vi.fn(), onSetDisabledTools: vi.fn(), onSetSubagents: vi.fn().mockResolvedValue(undefined),
      autoTitle: { enabled: false, providerId: null, modelId: null }, onSetAutoTitle: vi.fn(),
      appearance: defaultAppearance, glassSupported: true, onSetAppearance: vi.fn(), onPreviewAppearance: vi.fn(), prompts: defaultPrompts, onSetPrompts: vi.fn(),
      onRefresh: vi.fn().mockResolvedValue(undefined), onInstall: vi.fn(), onTrust: vi.fn(),
      onSearch: vi.fn().mockResolvedValue([]), onRemove: vi.fn(), onUpdate: vi.fn(), onSetResources: vi.fn()
    };
    const subagents: SubagentConfig = { ...noSubagents, enabled };
    const view = render(<SettingsPage {...props} subagents={subagents} />);
    return { ...view, props, subagents };
  }

  it("lists the Sub-agents page only while the built-in is on, and switches it on from Packages", async () => {
    const { rerender, props, subagents } = renderPage(false);
    const nav = screen.getByRole("navigation", { name: "Settings sections" });
    expect(within(nav).queryByRole("button", { name: /Sub-agents/ })).not.toBeInTheDocument();
    fireEvent.click(within(nav).getByRole("button", { name: /Packages/ }));
    fireEvent.click(await screen.findByRole("switch", { name: "Sub-agents" }));
    await waitFor(() => expect(props.onSetSubagents).toHaveBeenCalledWith({ ...subagents, enabled: true }));

    rerender(<SettingsPage {...props} subagents={{ ...subagents, enabled: true }} />);
    fireEvent.click(within(screen.getByRole("switch", { name: "Sub-agents" }).closest("article")!).getByRole("button", { name: /Configure/ }));
    expect(screen.getByRole("heading", { name: "How the agent uses sub-agents" })).toBeInTheDocument();
    expect(within(nav).getByRole("button", { name: /Sub-agents/ })).toHaveClass("active");

    // Switched off elsewhere while its page is open: back to Packages, where the switch lives.
    rerender(<SettingsPage {...props} subagents={subagents} />);
    expect(screen.queryByRole("heading", { name: "How the agent uses sub-agents" })).not.toBeInTheDocument();
    expect(within(nav).getByRole("button", { name: /Packages/ })).toHaveClass("active");
  });
});

describe("SettingsPage appearance section", () => {
  function renderAppearance(appearance: AppearanceConfig, onSetAppearance = vi.fn().mockResolvedValue(undefined)) {
    render(
      <SettingsPage
        providers={[]} packages={[]} toolCatalog={[]} disabledTools={[]} appDataPath="/tmp/wackcode"
        onClose={vi.fn()} onSave={vi.fn()} onDelete={vi.fn()} onConnectSubscription={vi.fn()} onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()}
        appearance={appearance} glassSupported onSetAppearance={onSetAppearance} onPreviewAppearance={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()}
        onRefresh={vi.fn().mockResolvedValue(undefined)} onInstall={vi.fn()} onTrust={vi.fn()}
        onSearch={vi.fn().mockResolvedValue([])} onRemove={vi.fn()} onUpdate={vi.fn()} onSetResources={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));
    return { onSetAppearance };
  }

  it("switches the thinking preview off and back on", async () => {
    const { onSetAppearance } = renderAppearance({ ...DEFAULT_APPEARANCE, thinkingPreview: true });
    const toggle = screen.getByRole("switch", { name: "Thinking preview" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(toggle);
    await waitFor(() => expect(onSetAppearance).toHaveBeenCalledWith({ ...DEFAULT_APPEARANCE, thinkingPreview: false }));
    cleanup();

    const again = renderAppearance({ ...DEFAULT_APPEARANCE, thinkingPreview: false });
    fireEvent.click(screen.getByRole("switch", { name: "Thinking preview" }));
    await waitFor(() => expect(again.onSetAppearance).toHaveBeenCalledWith({ ...DEFAULT_APPEARANCE, thinkingPreview: true }));
  });

  it("surfaces a failed save", async () => {
    renderAppearance({ ...DEFAULT_APPEARANCE, thinkingPreview: true }, vi.fn().mockRejectedValue("Could not save settings."));
    fireEvent.click(screen.getByRole("switch", { name: "Thinking preview" }));
    expect(await screen.findByText("Could not save settings.")).toBeInTheDocument();
  });
});
