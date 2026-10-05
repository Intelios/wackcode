import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { AppearanceConfig, BuiltinModelSuggestion, PromptConfig, ProviderRecord, SubagentConfig, ToolCatalogEntry } from "../types";
import { DEFAULT_APPEARANCE } from "../theme";
import { SettingsPage } from "./SettingsPage";

vi.mock("../api", () => ({ api: {
  revealPath: vi.fn().mockResolvedValue(undefined),
  appInfo: vi.fn().mockResolvedValue({
    appVersion: "1.0.5", build: "development", appPath: "/Applications/WackCode.app/Contents/MacOS/wackcode",
    piVersion: "0.99.2", nodeVersion: "24.18.0", osVersion: "macOS 15.3", chip: "Apple Silicon (arm64)",
    projectCount: 1, chatCount: 2, archivedCount: 0, activeWorkers: 0
  }),
  listBuiltinModels: vi.fn().mockResolvedValue([]),
  listSubscriptionProviders: vi.fn().mockResolvedValue([]),
  computerUseStatus: vi.fn().mockResolvedValue({ supported: true, accessibility: true, screenRecording: false, hotkeyAvailable: true, devBuild: false }),
  computerUseRequestPermission: vi.fn().mockResolvedValue(undefined),
  computerUseOpenSettings: vi.fn().mockResolvedValue(undefined),
  computerUseResetPermissions: vi.fn().mockResolvedValue(undefined),
  computerUseRelaunch: vi.fn().mockResolvedValue(undefined),
  computerUseListApps: vi.fn().mockResolvedValue([{ name: "Notes", bundleId: "com.apple.Notes" }])
} }));

vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: vi.fn().mockResolvedValue(undefined) }));

/** Opens a connection from the Settings sidebar, where each one is listed under Providers. */
function openConnection(name: string) {
  fireEvent.click(within(screen.getByRole("navigation", { name: "Settings sections" })).getByRole("button", { name }));
}

const noSubagents: SubagentConfig = { enabled: false, trigger: "on_request", maxConcurrency: 4, agents: [] };
const defaultAppearance: AppearanceConfig = DEFAULT_APPEARANCE;
const defaultPrompts: PromptConfig = {};

const noFavorites = { favoriteModels: [], onSetFavorite: vi.fn().mockResolvedValue(undefined) };

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
    onClose: vi.fn(), onSave: vi.fn(), onDelete: vi.fn(), onSetProviderEnabled: vi.fn(), onConnectSubscription,
    onSignOutSubscription: vi.fn(), onSetDisabledTools: vi.fn(), subagents: noSubagents, onSetSubagents: vi.fn(), autoTitle: { enabled: false, providerId: null, modelId: null }, onSetAutoTitle: vi.fn(), appearance: defaultAppearance, glassSupported: true, onSetAppearance: vi.fn(), onPreviewAppearance: vi.fn(), onChooseBackgroundImage: vi.fn(), onRemoveBackgroundImage: vi.fn(), prompts: defaultPrompts, onSetPrompts: vi.fn(), onCommandsChanged: vi.fn(), memory: { enabled: true, disabledProjects: [] }, onSetMemory: vi.fn(),
    onRefresh: vi.fn().mockResolvedValue(undefined), onInstall: vi.fn(), onTrust: vi.fn(),
    onSearch: vi.fn().mockResolvedValue([]), onRemove: vi.fn(), onUpdate: vi.fn(), onSetResources: vi.fn()
  };
  const { rerender } = render(<SettingsPage {...noFavorites} {...props} providers={[]} />);
  fireEvent.click(screen.getByRole("button", { name: /Sign in with a subscription/ }));
  expect(await screen.findByText("Claude usage may be billed separately.")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
  expect(onConnectSubscription).toHaveBeenCalledWith("anthropic");

  const provider: ProviderRecord = {
    id: "anthropic", name: "Anthropic", kind: "subscription", baseUrl: "", apiFormat: "",
    models: [], createdAt: "now", updatedAt: "now", hasApiKey: false, connected: true
  };
  rerender(<SettingsPage {...noFavorites} {...props} providers={[provider]} connectedSubscriptionId="anthropic" />);
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
    <SettingsPage {...noFavorites}
      providers={[]}
      packages={[]}
      toolCatalog={catalog}
      disabledTools={overrides.disabled ?? []}
      appDataPath="/tmp/wackcode"
      onClose={vi.fn()}
      onSave={vi.fn()}
      onDelete={vi.fn()}
      onSetProviderEnabled={vi.fn()}
      onConnectSubscription={vi.fn()}
      onSignOutSubscription={vi.fn()}
      onSetDisabledTools={onSetDisabledTools} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} onChooseBackgroundImage={vi.fn()} onRemoveBackgroundImage={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()} onCommandsChanged={vi.fn()} memory={{ enabled: true, disabledProjects: [] }} onSetMemory={vi.fn()}
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
  it("groups Pi's tools by what they do, apart from package tools", () => {
    renderTools();
    const looking = screen.getByRole("region", { name: "Looking around" });
    expect(within(looking).getByText("Read files")).toBeInTheDocument();
    expect(within(looking).getByText("Find files by name")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Making changes" })).getByText("Run commands")).toBeInTheDocument();
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
    expect(screen.getByText(/Needs fd, which isn't installed/)).toBeInTheDocument();
    expect(screen.getByText("brew install fd")).toBeInTheDocument();
  });

  it("opens the sections where the app's own tools are set up", () => {
    renderTools();
    fireEvent.click(screen.getByRole("button", { name: /^Built-ins/ }));
    expect(screen.getByRole("heading", { level: 2, name: "Packages" })).toBeInTheDocument();
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
      <SettingsPage {...noFavorites}
        providers={testProviders}
        packages={[]}
        toolCatalog={catalog}
        disabledTools={[]}
        appDataPath="/tmp/wackcode"
        onClose={vi.fn()}
        onSave={vi.fn()}
        onDelete={vi.fn()}
        onSetProviderEnabled={vi.fn()}
        onConnectSubscription={vi.fn()}
        onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} onChooseBackgroundImage={vi.fn()} onRemoveBackgroundImage={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()} onCommandsChanged={vi.fn()} memory={{ enabled: true, disabledProjects: [] }} onSetMemory={vi.fn()}
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

    // Expected order: Providers -> Entrim AI -> Test -> New connection -> Packages -> Skills -> Commands -> Tools -> Appearance -> Prompts
    expect(buttons).toEqual([
      "Providers",
      "Entrim AI",
      "Test",
      "New connection",
      "Integrations",
      "Packages",
      "Skills",
      "Commands",
      "Memory",
      "Tools",
      "Appearance",
      "Prompts"
    ]);
  });

  it("toggles provider subnav animation state and aria attributes when switching sections", () => {
    const { container } = render(
      <SettingsPage {...noFavorites}
        providers={testProviders}
        packages={[]}
        toolCatalog={catalog}
        disabledTools={[]}
        appDataPath="/tmp/wackcode"
        onClose={vi.fn()}
        onSave={vi.fn()}
        onDelete={vi.fn()}
        onSetProviderEnabled={vi.fn()}
        onConnectSubscription={vi.fn()}
        onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} onChooseBackgroundImage={vi.fn()} onRemoveBackgroundImage={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()} onCommandsChanged={vi.fn()} memory={{ enabled: true, disabledProjects: [] }} onSetMemory={vi.fn()}
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
    const nav = screen.getByRole("navigation", { name: "Settings sections" });
    expect(within(nav).getByRole("button", { name: /Entrim AI/ })).toBeInTheDocument();

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
    expect(within(nav).getByRole("button", { name: /Entrim AI/ })).toBeInTheDocument();
  });

  it("calls onClose when clicking the settings back button containing icon and Settings label", () => {
    const onClose = vi.fn();
    render(
      <SettingsPage {...noFavorites}
        providers={testProviders}
        packages={[]}
        toolCatalog={catalog}
        disabledTools={[]}
        appDataPath="/tmp/wackcode"
        onClose={onClose}
        onSave={vi.fn()}
        onDelete={vi.fn()}
        onSetProviderEnabled={vi.fn()}
        onConnectSubscription={vi.fn()}
        onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} onChooseBackgroundImage={vi.fn()} onRemoveBackgroundImage={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()} onCommandsChanged={vi.fn()} memory={{ enabled: true, disabledProjects: [] }} onSetMemory={vi.fn()}
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


describe("SettingsPage about", () => {
  function renderPage() {
    render(
      <SettingsPage {...noFavorites}
        providers={[]}
        packages={[]}
        toolCatalog={catalog}
        disabledTools={[]}
        appDataPath="/tmp/wackcode"
        onClose={vi.fn()}
        onSave={vi.fn()}
        onDelete={vi.fn()}
        onSetProviderEnabled={vi.fn()}
        onConnectSubscription={vi.fn()}
        onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} onChooseBackgroundImage={vi.fn()} onRemoveBackgroundImage={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()} onCommandsChanged={vi.fn()} memory={{ enabled: true, disabledProjects: [] }} onSetMemory={vi.fn()}
        onRefresh={vi.fn().mockResolvedValue(undefined)}
        onInstall={vi.fn()}
        onTrust={vi.fn()}
        onSearch={vi.fn().mockResolvedValue([])}
        onRemove={vi.fn()}
        onUpdate={vi.fn()}
        onSetResources={vi.fn()}
      />
    );
  }

  it("opens About from the sidebar footer, not the nav, and a nav click switches away", async () => {
    renderPage();
    const nav = screen.getByRole("navigation", { name: "Settings sections" });
    // About is the app's own page, so it stays out of the agent-feature nav.
    expect(within(nav).queryByRole("button", { name: "About" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "About" }));
    expect(screen.getByRole("heading", { name: "About" })).toBeInTheDocument();
    expect(await screen.findByRole("region", { name: "About overview" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "About" })).toHaveClass("active");
    fireEvent.click(within(nav).getByRole("button", { name: "Appearance" }));
    expect(screen.getByRole("heading", { name: "Appearance" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "About" })).not.toHaveClass("active");
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
      <SettingsPage {...noFavorites}
        providers={[provider]}
        packages={[]}
        toolCatalog={catalog}
        disabledTools={[]}
        appDataPath="/tmp/wackcode"
        onClose={vi.fn()}
        onSave={onSave}
        onDelete={vi.fn()}
        onSetProviderEnabled={vi.fn()}
        onConnectSubscription={vi.fn()}
        onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} onChooseBackgroundImage={vi.fn()} onRemoveBackgroundImage={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()} onCommandsChanged={vi.fn()} memory={{ enabled: true, disabledProjects: [] }} onSetMemory={vi.fn()}
        onRefresh={vi.fn().mockResolvedValue(undefined)}
        onInstall={vi.fn()}
        onTrust={vi.fn()}
        onSearch={vi.fn().mockResolvedValue([])}
        onRemove={vi.fn()}
        onUpdate={vi.fn()}
        onSetResources={vi.fn()}
      />
    );
    openConnection("Entrim AI");
    fireEvent.click(screen.getByRole("button", { name: /^Vision model/ }));
    const toggle = screen.getByRole("switch", { name: "Vision for Vision model" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("button", { name: "Save connection" }));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0][0].models[0].vision).toBe(true);
  });
});

describe("SettingsPage provider enable switch", () => {
  function renderProviderSettings(provider: ProviderRecord, onSetProviderEnabled = vi.fn().mockResolvedValue(undefined)) {
    const view = render(
      <SettingsPage {...noFavorites}
        providers={[provider]}
        packages={[]}
        toolCatalog={catalog}
        disabledTools={[]}
        appDataPath="/tmp/wackcode"
        onClose={vi.fn()}
        onSave={vi.fn()}
        onDelete={vi.fn()}
        onSetProviderEnabled={onSetProviderEnabled}
        onConnectSubscription={vi.fn()}
        onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} onChooseBackgroundImage={vi.fn()} onRemoveBackgroundImage={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()} onCommandsChanged={vi.fn()} memory={{ enabled: true, disabledProjects: [] }} onSetMemory={vi.fn()}
        onRefresh={vi.fn().mockResolvedValue(undefined)}
        onInstall={vi.fn()}
        onTrust={vi.fn()}
        onSearch={vi.fn().mockResolvedValue([])}
        onRemove={vi.fn()}
        onUpdate={vi.fn()}
        onSetResources={vi.fn()}
      />
    );
    return { ...view, onSetProviderEnabled };
  }

  it("switches the saved connection off immediately and keeps unsaved form edits", async () => {
    const provider = { ...testProviders[0] };
    const { rerender, onSetProviderEnabled } = renderProviderSettings(provider);
    openConnection("Entrim AI");
    const toggle = screen.getByRole("switch", { name: "Use Entrim AI" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.change(screen.getByLabelText("Name", { selector: "input" }), { target: { value: "Renamed" } });
    fireEvent.click(toggle);
    await waitFor(() => expect(onSetProviderEnabled).toHaveBeenCalledWith(provider.id, false));
    // The switch rewrites `providers` live; only the saved record's flag changed, so the
    // in-progress rename must survive it.
    expect(screen.getByLabelText("Name", { selector: "input" })).toHaveValue("Renamed");
    rerender(<SettingsPage {...noFavorites} providers={[{ ...provider, enabled: false }]} packages={[]} toolCatalog={catalog} disabledTools={[]} appDataPath="/tmp/wackcode" onClose={vi.fn()} onSave={vi.fn()} onDelete={vi.fn()} onSetProviderEnabled={onSetProviderEnabled} onConnectSubscription={vi.fn()} onSignOutSubscription={vi.fn()} onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} onChooseBackgroundImage={vi.fn()} onRemoveBackgroundImage={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()} onCommandsChanged={vi.fn()} memory={{ enabled: true, disabledProjects: [] }} onSetMemory={vi.fn()} onRefresh={vi.fn().mockResolvedValue(undefined)} onInstall={vi.fn()} onTrust={vi.fn()} onSearch={vi.fn().mockResolvedValue([])} onRemove={vi.fn()} onUpdate={vi.fn()} onSetResources={vi.fn()} />);
    expect(screen.getByLabelText("Name", { selector: "input" })).toHaveValue("Renamed");
    expect(screen.getByRole("switch", { name: "Use Entrim AI" })).toHaveAttribute("aria-checked", "false");
  });

  it("hides the switch while a new connection is being created", () => {
    renderProviderSettings(testProviders[0]);
    openConnection("New connection");
    // The blank editor has no saved record yet, so there is nothing to switch off.
    expect(screen.queryByRole("switch", { name: /Use / })).not.toBeInTheDocument();
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
    <SettingsPage {...noFavorites}
      providers={[provider]} packages={[]} toolCatalog={catalog} disabledTools={[]}
      appDataPath="/tmp/wackcode" onClose={vi.fn()} onSave={onSave} onDelete={vi.fn()} onSetProviderEnabled={vi.fn()}
      onConnectSubscription={vi.fn()} onSignOutSubscription={vi.fn()}
      onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} onChooseBackgroundImage={vi.fn()} onRemoveBackgroundImage={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()} onCommandsChanged={vi.fn()} memory={{ enabled: true, disabledProjects: [] }} onSetMemory={vi.fn()} onRefresh={vi.fn()} onInstall={vi.fn()} onTrust={vi.fn()}
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
    openConnection("Entrim AI");
    fireEvent.click(screen.getByRole("button", { name: /Add model/ }));
    const search = screen.getByRole("combobox", { name: /Find in Pi catalogue/ });
    fireEvent.change(search, { target: { value: "Deepseek V4 Flash" } });
    const options = await screen.findAllByRole("option");
    expect(options).toHaveLength(2);
    expect(options[0]).toHaveTextContent("deepseek · openai-completions · deepseek-flash");
    expect(options[1]).toHaveTextContent("openrouter");
    // Nothing is added until a suggestion is picked.
    expect(screen.queryByRole("textbox", { name: "Model ID" })).not.toBeInTheDocument();
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
    openConnection("Entrim AI");
    expect(screen.getByText("Needs limits")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^gateway\/deepseek-flash/ }));
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
    openConnection("Entrim AI");
    fireEvent.click(screen.getByRole("button", { name: /Add model/ }));
    fireEvent.change(screen.getByRole("combobox", { name: /Find in Pi catalogue/ }), { target: { value: "unknown" } });
    expect(await screen.findByText(/No Pi catalogue matches/)).toBeInTheDocument();
    unmount();

    vi.mocked(api.listBuiltinModels).mockRejectedValue(new Error("Catalogue unavailable"));
    renderModelSettings();
    openConnection("Entrim AI");
    fireEvent.click(screen.getByRole("button", { name: /Add model/ }));
    expect(await screen.findByText(/Could not load Pi catalogue: Error: Catalogue unavailable/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Enter it manually" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Model ID" }), { target: { value: "my-model" } });
    expect(screen.getByRole("textbox", { name: "Model ID" })).toHaveValue("my-model");
  });
});

describe("SettingsPage sub-agents", () => {
  function renderPage(enabled: boolean) {
    const props = {
      providers: [], packages: [], toolCatalog: [], disabledTools: [], appDataPath: "/tmp/wackcode",
      onClose: vi.fn(), onSave: vi.fn(), onDelete: vi.fn(), onSetProviderEnabled: vi.fn(), onConnectSubscription: vi.fn(),
      onSignOutSubscription: vi.fn(), onSetDisabledTools: vi.fn(), onSetSubagents: vi.fn().mockResolvedValue(undefined),
      autoTitle: { enabled: false, providerId: null, modelId: null }, onSetAutoTitle: vi.fn(),
      appearance: defaultAppearance, glassSupported: true, onSetAppearance: vi.fn(), onPreviewAppearance: vi.fn(), onChooseBackgroundImage: vi.fn(), onRemoveBackgroundImage: vi.fn(), prompts: defaultPrompts, onSetPrompts: vi.fn(), onCommandsChanged: vi.fn(), memory: { enabled: true, disabledProjects: [] }, onSetMemory: vi.fn(),
      onRefresh: vi.fn().mockResolvedValue(undefined), onInstall: vi.fn(), onTrust: vi.fn(),
      onSearch: vi.fn().mockResolvedValue([]), onRemove: vi.fn(), onUpdate: vi.fn(), onSetResources: vi.fn()
    };
    const subagents: SubagentConfig = { ...noSubagents, enabled };
    const view = render(<SettingsPage {...noFavorites} {...props} subagents={subagents} />);
    return { ...view, props, subagents };
  }

  it("lists the Sub-agents page only while the built-in is on, and switches it on from Packages", async () => {
    const { rerender, props, subagents } = renderPage(false);
    const nav = screen.getByRole("navigation", { name: "Settings sections" });
    expect(within(nav).queryByRole("button", { name: /Sub-agents/ })).not.toBeInTheDocument();
    fireEvent.click(within(nav).getByRole("button", { name: /Packages/ }));
    fireEvent.click(await screen.findByRole("switch", { name: "Sub-agents" }));
    await waitFor(() => expect(props.onSetSubagents).toHaveBeenCalledWith({ ...subagents, enabled: true }));

    rerender(<SettingsPage {...noFavorites} {...props} subagents={{ ...subagents, enabled: true }} />);
    fireEvent.click(within(screen.getByRole("switch", { name: "Sub-agents" }).closest("article")!).getByRole("button", { name: /Configure/ }));
    expect(screen.getByRole("heading", { name: "Helpers WackCode can hand work to" })).toBeInTheDocument();
    expect(within(nav).getByRole("button", { name: /Sub-agents/ })).toHaveClass("active");

    // Switched off elsewhere while its page is open: back to Packages, where the switch lives.
    rerender(<SettingsPage {...noFavorites} {...props} subagents={subagents} />);
    expect(screen.queryByRole("heading", { name: "Helpers WackCode can hand work to" })).not.toBeInTheDocument();
    expect(within(nav).getByRole("button", { name: /Packages/ })).toHaveClass("active");
  });

  it("switches computer use on only through its setup dialog, then lists its page", async () => {
    const { rerender, props, subagents } = renderPage(false);
    const onSetComputerUse = vi.fn().mockResolvedValue(undefined);
    const off = { enabled: false, showAgentCursor: true, neverAllow: [] };
    rerender(<SettingsPage {...noFavorites} {...props} subagents={subagents} computerUse={off} computerUseSupported onSetComputerUse={onSetComputerUse} />);
    const nav = screen.getByRole("navigation", { name: "Settings sections" });
    expect(within(nav).queryByRole("button", { name: /Computer use/ })).not.toBeInTheDocument();
    fireEvent.click(within(nav).getByRole("button", { name: /Packages/ }));
    fireEvent.click(await screen.findByRole("switch", { name: "Computer use" }));

    const dialog = await screen.findByRole("dialog", { name: "Set up computer use" });
    expect(onSetComputerUse).not.toHaveBeenCalled();
    // Accessibility is allowed; Screen Recording still needs granting.
    await within(dialog).findByText("Allowed");
    fireEvent.click(within(dialog).getByRole("button", { name: "Allow…" }));
    expect(api.computerUseRequestPermission).toHaveBeenCalledWith("screenRecording");
    fireEvent.click(within(dialog).getByRole("button", { name: "Turn on anyway" }));
    await waitFor(() => expect(onSetComputerUse).toHaveBeenCalledWith({ enabled: true, showAgentCursor: true, neverAllow: [] }));

    rerender(<SettingsPage {...noFavorites} {...props} subagents={subagents} computerUse={{ enabled: true, showAgentCursor: true, neverAllow: [] }} computerUseSupported onSetComputerUse={onSetComputerUse} />);
    expect(within(nav).getByRole("button", { name: /Computer use/ })).toHaveClass("active");
    expect(screen.getByRole("heading", { name: "Apps it never uses" })).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "Bundle id to never allow" }), { target: { value: "com.example.Secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() => expect(onSetComputerUse).toHaveBeenLastCalledWith({ enabled: true, showAgentCursor: true, neverAllow: ["com.example.Secret"] }));
  });

  it("keeps the computer use switch off on Macs that can't run it", async () => {
    const { rerender, props, subagents } = renderPage(false);
    rerender(<SettingsPage {...noFavorites} {...props} subagents={subagents} computerUse={{ enabled: false, showAgentCursor: true, neverAllow: [] }} computerUseSupported={false} onSetComputerUse={vi.fn()} />);
    fireEvent.click(within(screen.getByRole("navigation", { name: "Settings sections" })).getByRole("button", { name: /Packages/ }));
    const toggle = await screen.findByRole("switch", { name: "Computer use" });
    expect(toggle).toBeDisabled();
    expect(toggle).toHaveAttribute("title", "Requires macOS 14 or later");
  });

  it("switches Web Fetch through the tool denylist, keeping the rest of it", async () => {
    const { rerender, props, subagents } = renderPage(false);
    props.onSetDisabledTools.mockResolvedValue(undefined);
    rerender(<SettingsPage {...noFavorites} {...props} subagents={subagents} disabledTools={["bash"]} />);
    fireEvent.click(within(screen.getByRole("navigation", { name: "Settings sections" })).getByRole("button", { name: /Packages/ }));
    fireEvent.click(await screen.findByRole("switch", { name: "Web Fetch" }));
    await waitFor(() => expect(props.onSetDisabledTools).toHaveBeenCalledWith(["bash", "web_fetch"]));

    rerender(<SettingsPage {...noFavorites} {...props} subagents={subagents} disabledTools={["bash", "web_fetch"]} />);
    expect(screen.getByRole("switch", { name: "Web Fetch" })).toHaveAttribute("aria-checked", "false");
    fireEvent.click(screen.getByRole("switch", { name: "Web Fetch" }));
    await waitFor(() => expect(props.onSetDisabledTools).toHaveBeenLastCalledWith(["bash"]));
  });
});

describe("SettingsPage MCP servers", () => {
  it("lists MCP servers after Tools and opens its page from the nav and from Packages", () => {
    const mcpActions = {
      onSaveMcpServer: vi.fn(), onDeleteMcpServer: vi.fn(), onSetMcpServerEnabled: vi.fn(),
      onSetMcpServerTools: vi.fn(), onTestMcpServer: vi.fn()
    };
    render(
      <SettingsPage {...noFavorites}
        providers={[]} packages={[]} toolCatalog={[]} disabledTools={[]} appDataPath="/tmp/wackcode"
        onClose={vi.fn()} onSave={vi.fn()} onDelete={vi.fn()} onSetProviderEnabled={vi.fn()} onConnectSubscription={vi.fn()} onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()} appearance={defaultAppearance} glassSupported onSetAppearance={vi.fn()} onPreviewAppearance={vi.fn()} onChooseBackgroundImage={vi.fn()} onRemoveBackgroundImage={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()} onCommandsChanged={vi.fn()} memory={{ enabled: true, disabledProjects: [] }} onSetMemory={vi.fn()}
        mcp={{ servers: [] }} mcpActions={mcpActions}
        onRefresh={vi.fn().mockResolvedValue(undefined)} onInstall={vi.fn()} onTrust={vi.fn()} onSearch={vi.fn().mockResolvedValue([])} onRemove={vi.fn()} onUpdate={vi.fn()} onSetResources={vi.fn()}
      />
    );
    const nav = screen.getByRole("navigation", { name: "Settings sections" });
    const labels = Array.from(nav.querySelectorAll("button")).map((button) => button.textContent?.trim());
    expect(labels.slice(labels.indexOf("Tools"), labels.indexOf("Tools") + 2)).toEqual(["Tools", "MCP servers"]);

    fireEvent.click(within(nav).getByRole("button", { name: "MCP servers" }));
    expect(screen.getByRole("heading", { level: 2, name: "MCP servers" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "No servers yet" })).toBeInTheDocument();

    fireEvent.click(within(nav).getByRole("button", { name: "Packages" }));
    fireEvent.click(within(screen.getByRole("article", { name: "MCP servers" })).getByRole("button", { name: /Configure/ }));
    expect(screen.getByRole("heading", { level: 2, name: "MCP servers" })).toBeInTheDocument();
  });
});

describe("SettingsPage appearance section", () => {
  function renderAppearance(appearance: AppearanceConfig, onSetAppearance = vi.fn().mockResolvedValue(undefined)) {
    render(
      <SettingsPage {...noFavorites}
        providers={[]} packages={[]} toolCatalog={[]} disabledTools={[]} appDataPath="/tmp/wackcode"
        onClose={vi.fn()} onSave={vi.fn()} onDelete={vi.fn()} onSetProviderEnabled={vi.fn()} onConnectSubscription={vi.fn()} onSignOutSubscription={vi.fn()}
        onSetDisabledTools={vi.fn()} subagents={noSubagents} onSetSubagents={vi.fn()} autoTitle={{ enabled: false, providerId: null, modelId: null }} onSetAutoTitle={vi.fn()}
        appearance={appearance} glassSupported onSetAppearance={onSetAppearance} onPreviewAppearance={vi.fn()} onChooseBackgroundImage={vi.fn()} onRemoveBackgroundImage={vi.fn()} prompts={defaultPrompts} onSetPrompts={vi.fn()} onCommandsChanged={vi.fn()} memory={{ enabled: true, disabledProjects: [] }} onSetMemory={vi.fn()}
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

  it("switches message bubbles on and back off", async () => {
    const { onSetAppearance } = renderAppearance({ ...DEFAULT_APPEARANCE, messageBubbles: false });
    const toggle = screen.getByRole("switch", { name: "Message bubbles" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    fireEvent.click(toggle);
    await waitFor(() => expect(onSetAppearance).toHaveBeenCalledWith({ ...DEFAULT_APPEARANCE, messageBubbles: true }));
    cleanup();

    const again = renderAppearance({ ...DEFAULT_APPEARANCE, messageBubbles: true });
    fireEvent.click(screen.getByRole("switch", { name: "Message bubbles" }));
    await waitFor(() => expect(again.onSetAppearance).toHaveBeenCalledWith({ ...DEFAULT_APPEARANCE, messageBubbles: false }));
  });

  it("switches exploration grouping off and back on", async () => {
    const { onSetAppearance } = renderAppearance({ ...DEFAULT_APPEARANCE, groupExploration: true });
    const toggle = screen.getByRole("switch", { name: "Group exploration" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(toggle);
    await waitFor(() => expect(onSetAppearance).toHaveBeenCalledWith({ ...DEFAULT_APPEARANCE, groupExploration: false }));
    cleanup();

    const again = renderAppearance({ ...DEFAULT_APPEARANCE, groupExploration: false });
    fireEvent.click(screen.getByRole("switch", { name: "Group exploration" }));
    await waitFor(() => expect(again.onSetAppearance).toHaveBeenCalledWith({ ...DEFAULT_APPEARANCE, groupExploration: true }));
  });

  it("sets the thinking timer to tenths and back to whole seconds", async () => {
    const { onSetAppearance } = renderAppearance({ ...DEFAULT_APPEARANCE, thinkingTimerPrecision: "second" });
    const group = screen.getByRole("radiogroup", { name: "Thinking timer" });
    expect(within(group).getByRole("radio", { name: "1s" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(within(group).getByRole("radio", { name: "0.1s" }));
    await waitFor(() => expect(onSetAppearance).toHaveBeenCalledWith({ ...DEFAULT_APPEARANCE, thinkingTimerPrecision: "tenth" }));
    cleanup();

    const again = renderAppearance({ ...DEFAULT_APPEARANCE, thinkingTimerPrecision: "tenth" });
    const switched = screen.getByRole("radiogroup", { name: "Thinking timer" });
    expect(within(switched).getByRole("radio", { name: "0.1s" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(within(switched).getByRole("radio", { name: "1s" }));
    await waitFor(() => expect(again.onSetAppearance).toHaveBeenCalledWith({ ...DEFAULT_APPEARANCE, thinkingTimerPrecision: "second" }));
  });

  it("surfaces a failed save", async () => {
    renderAppearance({ ...DEFAULT_APPEARANCE, thinkingPreview: true }, vi.fn().mockRejectedValue("Could not save settings."));
    fireEvent.click(screen.getByRole("switch", { name: "Thinking preview" }));
    expect(await screen.findByText("Could not save settings.")).toBeInTheDocument();
  });
});
