import { Fragment, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { api } from "../api";
import { agentName } from "../agentName";
import { applyBuiltinModelSuggestion, mergeDiscoveredModels, modelIsReady, searchBuiltinModels } from "../model-utils";
import { BROWSER_TOOL_NAMES, WEB_FETCH_TOOL_NAME, groupTools } from "../tool-utils";
import type { ApiFormat, AppearanceConfig, AutoTitleConfig, BuiltinModelSuggestion, CommandsConfig, ComputerUseConfig, CustomProviderRecord, McpConfig, MemoryConfig, ModelRecord, PackageRecord, PromptConfig, ProviderRecord, SaveProviderInput, SubagentConfig, SubscriptionProviderInfo, ThinkingLevel, ToolCatalogEntry } from "../types";
import { Icon, type IconName } from "./Icons";
import { CommandsSection, type SlashCommandActions } from "./CommandsSection";
import { MemorySection, type MemoryActions } from "./MemorySection";
import { ComputerUseSection, type ComputerUseActions } from "./ComputerUseSection";
import { ComputerUseSetupDialog } from "./ComputerUseSetupDialog";
import { IntegrationsSection } from "./IntegrationsSection";
import { McpSection, type McpActions } from "./McpSection";
import { PackagesSection, type PackageActions } from "./PackagesSection";
import { PromptsSection } from "./PromptsSection";
import { SkillsSection, type SkillActions } from "./SkillsSection";
import { SubagentsSection } from "./SubagentsSection";
import { AppearanceSection } from "./AppearanceSection";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { Popover } from "./ui/Popover";
import { Select } from "./ui/Select";
import { Tooltip } from "./ui/Tooltip";

const levels: ThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/** Settings › Skills talks to Rust directly, like the rest of this page; `api`'s functions are stable. */
const SKILL_ACTIONS: SkillActions = {
  onList: api.listSkills,
  onRead: api.readSkill,
  onSave: api.saveSkill,
  onDelete: api.deleteSkill,
  onSetEnabled: api.setSkillEnabled,
  onSetFolderEnabled: api.setSkillFolderEnabled,
  onAddFolder: api.addSkillFolder,
  onRemoveFolder: api.removeSkillFolder,
  onImport: api.importSkill,
  onCopyToLibrary: api.copySkillToLibrary,
  onReveal: api.revealPath,
  onSearch: api.searchSkillPackages,
  onDetails: api.packageDetails
};
/** Settings › Commands talks to Rust directly, like Skills; `api`'s functions are stable. */
const COMMAND_ACTIONS: Omit<SlashCommandActions, "onChanged"> = {
  onList: api.listSlashCommands,
  onRead: api.readSlashCommand,
  onSave: api.saveSlashCommand,
  onDelete: api.deleteSlashCommand,
  onSetEnabled: api.setSlashCommandEnabled,
  onReveal: api.revealPath
};
/** Settings › Memory talks to Rust directly, like Commands; the master switch comes from App. */
const MEMORY_ACTIONS: Omit<MemoryActions, "onSetEnabled"> = {
  onList: api.listMemories,
  onRead: api.readMemory,
  onSave: api.saveMemory,
  onDelete: api.deleteMemory,
  onRemoveProject: api.removeMemoryProject,
  onSetProjectEnabled: api.setProjectMemoryEnabled,
  onReveal: api.revealPath,
  onFindFile: api.findMemoryInFinder
};
/** Settings › Computer use and its setup dialog talk to Rust directly, like Skills. */
const COMPUTER_USE_ACTIONS: ComputerUseActions = {
  onStatus: api.computerUseStatus,
  onRequestPermission: api.computerUseRequestPermission,
  onOpenSettings: api.computerUseOpenSettings,
  onResetPermissions: api.computerUseResetPermissions,
  onRelaunch: api.computerUseRelaunch,
  onListApps: api.computerUseListApps
};
const COMPUTER_USE_OFF: ComputerUseConfig = { enabled: false, neverAllow: [] };
let nextModelCardKey = 0;
const newModelCardKeys = (count: number) => Array.from({ length: count }, () => ++nextModelCardKey);

type SectionId = "providers" | "packages" | "skills" | "commands" | "memory" | "tools" | "mcp" | "appearance" | "prompts" | "subagents" | "computer_use" | "integrations";

interface Section {
  id: SectionId;
  label: string;
  icon: IconName;
}

const SECTIONS: Section[] = [
  { id: "providers", label: "Providers", icon: "key" },
  { id: "integrations", label: "Integrations", icon: "plug" },
  { id: "packages", label: "Packages", icon: "spark" },
  { id: "skills", label: "Skills", icon: "book" },
  { id: "commands", label: "Commands", icon: "slash" },
  { id: "memory", label: "Memory", icon: "memory" },
  { id: "tools", label: "Tools", icon: "wrench" },
  { id: "mcp", label: "MCP servers", icon: "plug" },
  // Only listed while the built-in is switched on (Settings → Packages). Auto titles lives
  // on the same page, as an agent WackCode runs itself rather than one the model can call.
  { id: "subagents", label: "Sub-agents", icon: "agents" },
  // Only listed while computer use is switched on (Settings → Packages).
  { id: "computer_use", label: "Computer use", icon: "cursor" },
  { id: "appearance", label: "Appearance", icon: "palette" },
  { id: "prompts", label: "Prompts", icon: "pencil" }
];

interface Props extends PackageActions {
  providers: ProviderRecord[];
  packages: PackageRecord[];
  toolCatalog: ToolCatalogEntry[];
  disabledTools: string[];
  appDataPath: string;
  onClose: () => void;
  onSave: (input: SaveProviderInput) => Promise<ProviderRecord>;
  onDelete: (providerId: string) => Promise<void>;
  /** Switches a saved connection off/on; unlike `onSave` it applies immediately, not on Save. */
  onSetProviderEnabled: (providerId: string, enabled: boolean) => Promise<void>;
  onConnectSubscription: (providerId: string) => Promise<void>;
  onSignOutSubscription: (providerId: string) => Promise<void>;
  connectedSubscriptionId?: string;
  onSetDisabledTools: (disabled: string[]) => Promise<void>;
  subagents: SubagentConfig;
  onSetSubagents: (config: SubagentConfig) => Promise<void>;
  computerUse?: ComputerUseConfig;
  /** macOS 14+: computer use can be switched on. */
  computerUseSupported?: boolean;
  onSetComputerUse?: (config: ComputerUseConfig) => Promise<void>;
  autoTitle: AutoTitleConfig;
  onSetAutoTitle: (config: AutoTitleConfig) => Promise<void>;
  appearance: AppearanceConfig;
  /** macOS 26+: the Liquid Glass backdrop is available. */
  glassSupported: boolean;
  onSetAppearance: (config: AppearanceConfig) => Promise<void>;
  onPreviewAppearance: (config: AppearanceConfig) => void;
  backgroundImageUrl?: string;
  onChooseBackgroundImage: () => Promise<void>;
  onRemoveBackgroundImage: () => Promise<void>;
  prompts: PromptConfig;
  onSetPrompts: (config: PromptConfig) => Promise<void>;
  /** Commands-section saves report the config back so `data.commands` stays current. */
  onCommandsChanged: (config: CommandsConfig) => void;
  memory: MemoryConfig;
  onSetMemory: (config: MemoryConfig) => Promise<void>;
  mcp?: McpConfig;
  /** Settings › MCP servers is listed once these are wired. */
  mcpActions?: McpActions;
}

export function SettingsPage({
  providers, packages, toolCatalog, disabledTools, appDataPath,
  onClose, onSave, onDelete, onSetProviderEnabled, onConnectSubscription, onSignOutSubscription, connectedSubscriptionId, onSetDisabledTools,
  subagents, onSetSubagents, computerUse = COMPUTER_USE_OFF, computerUseSupported = false, onSetComputerUse, autoTitle, onSetAutoTitle, appearance, glassSupported, onSetAppearance, onPreviewAppearance, backgroundImageUrl, onChooseBackgroundImage, onRemoveBackgroundImage, prompts, onSetPrompts, onCommandsChanged, memory, onSetMemory, mcp, mcpActions, onRefresh, onInstall, onTrust, onSearch, onRemove, onUpdate, onSetResources
}: Props) {
  const [chosenSection, setSection] = useState<SectionId>("providers");
  const [computerSetup, setComputerSetup] = useState(false);
  // Switching sub-agents or computer use off while its page is open lands on Packages, where the switch is.
  const section: SectionId =
    (chosenSection === "subagents" && !subagents.enabled) || (chosenSection === "computer_use" && !computerUse.enabled) ? "packages" : chosenSection;
  const sections = SECTIONS.filter((item) =>
    (item.id !== "subagents" || subagents.enabled) && (item.id !== "computer_use" || computerUse.enabled) && (item.id !== "mcp" || mcpActions));
  const [selectedProviderId, setSelectedProviderId] = useState(providers[0]?.id ?? "new");
  const [builtinModels, setBuiltinModels] = useState<BuiltinModelSuggestion[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [catalogError, setCatalogError] = useState<string>();
  const [subscriptionProviders, setSubscriptionProviders] = useState<SubscriptionProviderInfo[]>([]);
  const [subscriptionError, setSubscriptionError] = useState<string>();
  const [newMethod, setNewMethod] = useState<"apiKey" | "subscription">("apiKey");
  const selectedSection = SECTIONS.find((item) => item.id === section);

  useEffect(() => {
    let active = true;
    api.listBuiltinModels().then((models) => {
      if (active) setBuiltinModels(models);
    }).catch((reason) => {
      if (active) setCatalogError(String(reason));
    }).finally(() => {
      if (active) setCatalogLoading(false);
    });
    api.listSubscriptionProviders().then((providers) => {
      if (active) setSubscriptionProviders(providers);
    }).catch((reason) => { if (active) setSubscriptionError(String(reason)); });
    return () => { active = false; };
  }, []);

  useEffect(() => {
    if (connectedSubscriptionId) { setSelectedProviderId(connectedSubscriptionId); setSection("providers"); }
  }, [connectedSubscriptionId]);

  return (
    <>
      <aside className="sidebar">
        <div className="titlebar-drag" data-tauri-drag-region />
        <div className="settings-head">
          <Tooltip label="Back to chats">
            <button type="button" className="settings-back-button" onClick={onClose} aria-label="Back to chats">
              <Icon name="back" />
              <span className="settings-title">Settings</span>
            </button>
          </Tooltip>
        </div>
        <nav className="settings-nav" aria-label="Settings sections">
          {sections.map((item) => (
            <Fragment key={item.id}>
              <button
                type="button"
                className={`settings-nav-item ${item.id === section ? "active" : ""}`}
                onClick={() => setSection(item.id)}
                aria-expanded={item.id === "providers" ? section === "providers" : undefined}
              >
                <Icon name={item.icon} /> {item.label}
              </button>
              {item.id === "providers" && (
                <div
                  className={`settings-subnav-wrapper ${section === "providers" ? "expanded" : ""}`}
                  aria-hidden={section !== "providers"}
                  inert={section !== "providers" ? true : undefined}
                >
                  <div className="settings-subnav-inner">
                    <div className="settings-subnav">
                      {providers.map((provider) => (
                        <button
                          key={provider.id}
                          type="button"
                          className={`settings-subnav-item ${selectedProviderId === provider.id ? "active" : ""} ${provider.enabled === false ? "off" : ""}`}
                          onClick={() => {
                            setSelectedProviderId(provider.id);
                            setSection("providers");
                          }}
                        >
                          <span className={`credential-dot ${provider.connected ? "connected" : ""}`} />
                          <span>{provider.name}</span>
                        </button>
                      ))}
                      <button
                        type="button"
                        className={`settings-subnav-item ${selectedProviderId === "new" ? "active" : ""}`}
                        onClick={() => {
                          setSelectedProviderId("new");
                          setSection("providers");
                        }}
                      >
                        <Icon name="plus" /> New connection
                      </button>
                    </div>
                  </div>
                </div>
              )}
            </Fragment>
          ))}
        </nav>
        <div className="sidebar-footer">
          <button type="button" className="sidebar-action" title={appDataPath} onClick={() => void api.revealPath(appDataPath).catch(() => undefined)}>
            <Icon name="folder" /> Data folder
          </button>
        </div>
      </aside>

      <main className="workspace settings-workspace">
        <header className="settings-page-head">
          <div>
            <span className="eyebrow">{selectedSection?.label}</span>
            <h2>{section === "providers" ? providerHeading(providers, selectedProviderId) : selectedSection?.label}</h2>
          </div>
          <button type="button" className="secondary-button" onClick={onClose}>Done</button>
        </header>
        {section === "providers" && selectedProviderId === "new" && <div className="connection-methods" role="group" aria-label="Connection method">
          <button type="button" className={newMethod === "apiKey" ? "selected" : ""} onClick={() => setNewMethod("apiKey")}>API key</button>
          <button type="button" className={newMethod === "subscription" ? "selected" : ""} onClick={() => setNewMethod("subscription")}>Sign in with a subscription</button>
        </div>}
        {section === "providers" && (providers.find((provider) => provider.id === selectedProviderId)?.kind === "subscription"
          ? <SubscriptionSection
              provider={providers.find((provider) => provider.id === selectedProviderId)!}
              guidance={subscriptionProviders.find((provider) => provider.id === selectedProviderId)?.guidance}
              onConnect={onConnectSubscription}
              onSignOut={onSignOutSubscription}
              onDelete={onDelete}
              onSetProviderEnabled={onSetProviderEnabled}
              onSelect={setSelectedProviderId}
            />
          : selectedProviderId === "new" && newMethod === "subscription"
          ? <SubscriptionCatalog providers={subscriptionProviders} error={subscriptionError} onConnect={onConnectSubscription} />
          : <ProvidersSection
            providers={providers}
            selectedId={selectedProviderId}
            onSelect={setSelectedProviderId}
            onSave={onSave}
            onDelete={onDelete}
            onSetProviderEnabled={onSetProviderEnabled}
            builtinModels={builtinModels}
            catalogLoading={catalogLoading}
            catalogError={catalogError}
          />)}
        {section === "integrations" && <IntegrationsSection />}
        {section === "packages" && (
          <PackagesSection
            packages={packages}
            subagentsEnabled={subagents.enabled}
            memoryEnabled={memory.enabled}
            onConfigureMemory={() => setSection("memory")}
            webFetchEnabled={!disabledTools.includes(WEB_FETCH_TOOL_NAME)}
            browserEnabled={BROWSER_TOOL_NAMES.every((name) => !disabledTools.includes(name))}
            computerUseEnabled={computerUse.enabled}
            computerUseSupported={computerUseSupported}
            // Switching on goes through the setup dialog, which saves the setting itself.
            onToggleComputerUse={onSetComputerUse && (async (enabled) => {
              if (enabled) setComputerSetup(true);
              else await onSetComputerUse({ ...computerUse, enabled: false });
            })}
            onConfigureComputerUse={() => setSection("computer_use")}
            autoTitlesEnabled={autoTitle.enabled}
            autoTitlesConfigured={providers.some((provider) => provider.id === autoTitle.providerId && provider.connected && provider.models.some((model) => model.id === autoTitle.modelId && modelIsReady(model)))}
            onToggleSubagents={(enabled) => onSetSubagents({ ...subagents, enabled })}
            // Web fetch rides the Tools denylist, so switching it applies live with no restart.
            onToggleWebFetch={(enabled) => onSetDisabledTools(
              enabled
                ? disabledTools.filter((name) => name !== WEB_FETCH_TOOL_NAME)
                : [...disabledTools.filter((name) => name !== WEB_FETCH_TOOL_NAME), WEB_FETCH_TOOL_NAME]
            )}
            onToggleBrowser={(enabled) => onSetDisabledTools(
              enabled
                ? disabledTools.filter((name) => !(BROWSER_TOOL_NAMES as readonly string[]).includes(name))
                : [...disabledTools.filter((name) => !(BROWSER_TOOL_NAMES as readonly string[]).includes(name)), ...BROWSER_TOOL_NAMES]
            )}
            onToggleAutoTitles={(enabled) => onSetAutoTitle({ ...autoTitle, enabled })}
            onConfigureAutoTitles={() => setSection("subagents")}
            onConfigureSubagents={() => setSection("subagents")}
            onConfigureMcp={mcpActions ? () => setSection("mcp") : undefined}
            onRefresh={onRefresh}
            onInstall={onInstall}
            onTrust={onTrust}
            onSearch={onSearch}
            onRemove={onRemove}
            onUpdate={onUpdate}
            onSetResources={onSetResources}
          />
        )}
        {section === "skills" && (
          <SkillsSection
            {...SKILL_ACTIONS}
            packages={packages}
            onInstallSkills={(source) => onInstall(source, { skillsOnly: true })}
            onSetPackageSkills={(source, enabled) => onSetResources(source, "skills", enabled)}
            onOpenPackages={() => setSection("packages")}
          />
        )}
        {section === "commands" && (
          <CommandsSection {...COMMAND_ACTIONS} onChanged={onCommandsChanged} />
        )}
        {section === "memory" && (
          <MemorySection
            {...MEMORY_ACTIONS}
            onSetEnabled={(enabled) => onSetMemory({ ...memory, enabled })}
          />
        )}
        {section === "tools" && (
          <ToolsSection catalog={toolCatalog} disabled={disabledTools} onSetDisabled={onSetDisabledTools} />
        )}
        {section === "appearance" && (
          <AppearanceSection
            config={appearance}
            glassSupported={glassSupported}
            backgroundImageUrl={backgroundImageUrl}
            onChange={onSetAppearance}
            onPreview={onPreviewAppearance}
            onChooseImage={onChooseBackgroundImage}
            onRemoveImage={onRemoveBackgroundImage}
          />
        )}
        {section === "prompts" && <PromptsSection config={prompts} agentName={agentName(appearance)} onChange={onSetPrompts} />}
        {section === "mcp" && mcpActions && <McpSection servers={mcp?.servers ?? []} {...mcpActions} />}
        {section === "computer_use" && onSetComputerUse && (
          <ComputerUseSection config={computerUse} actions={COMPUTER_USE_ACTIONS} agentName={agentName(appearance)} onChange={onSetComputerUse} />
        )}
        {computerSetup && onSetComputerUse && (
          <ComputerUseSetupDialog
            actions={COMPUTER_USE_ACTIONS}
            agentName={agentName(appearance)}
            onEnable={async () => {
              await onSetComputerUse({ ...computerUse, enabled: true });
              setComputerSetup(false);
              setSection("computer_use");
            }}
            onCancel={() => setComputerSetup(false)}
          />
        )}
        {section === "subagents" && (
          <SubagentsSection
            config={subagents}
            providers={providers}
            onChange={onSetSubagents}
            webFetchEnabled={!disabledTools.includes(WEB_FETCH_TOOL_NAME)}
            autoTitle={autoTitle}
            onSetAutoTitle={onSetAutoTitle}
            onOpenProviders={() => { setSelectedProviderId("new"); setSection("providers"); }}
          />
        )}
      </main>
    </>
  );
}

interface ToolsSectionProps {
  catalog: ToolCatalogEntry[];
  disabled: string[];
  onSetDisabled: (disabled: string[]) => Promise<void>;
}

function ToolsSection({ catalog, disabled, onSetDisabled }: ToolsSectionProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const groups = useMemo(() => groupTools(catalog), [catalog]);
  const disabledSet = useMemo(() => new Set(disabled), [disabled]);

  async function toggle(name: string, enabled: boolean): Promise<void> {
    const next = enabled ? disabled.filter((item) => item !== name) : [...disabled, name];
    setBusy(true);
    setError(undefined);
    try {
      await onSetDisabled(next);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  if (!catalog.length) {
    return (
      <div className="settings-scroll">
        <div className="model-empty">
          Tools are listed once a chat has started. Open or create a chat, then come back.
        </div>
      </div>
    );
  }

  return (
    <div className="settings-scroll">
      <div className="section-heading-row">
        <div>
          <h3>Available tools</h3>
          <p>Switched-off tools are not offered to the model. Changes apply to running chats on their next turn.</p>
        </div>
      </div>
      {groups.map((group) => (
        <section className="tool-setting-group" key={group.id}>
          <h4>{group.label}</h4>
          {group.tools.map((tool) => {
            const enabled = tool.available && !disabledSet.has(tool.name);
            return (
              <div className={`tool-setting ${tool.available ? "" : "unavailable"}`} key={tool.name}>
                <div className="tool-setting-text">
                  <span className="tool-setting-name">{tool.name}</span>
                  <span className="tool-setting-description">{tool.unavailableReason ?? tool.description}</span>
                </div>
                <button
                  type="button"
                  role="switch"
                  aria-checked={enabled}
                  aria-label={tool.name}
                  className={`toggle ${enabled ? "on" : ""}`}
                  disabled={busy || !tool.available}
                  onClick={() => void toggle(tool.name, disabledSet.has(tool.name))}
                >
                  <span />
                </button>
              </div>
            );
          })}
        </section>
      ))}
      {error && <div className="error-banner">{error}</div>}
    </div>
  );
}

function providerHeading(providers: ProviderRecord[], selectedId: string): ReactNode {
  const provider = providers.find((item) => item.id === selectedId);
  return provider ? provider.name : "New connection";
}

interface Draft extends SaveProviderInput {
  id?: string;
}

function blankDraft(): Draft {
  return { name: "", baseUrl: "", apiFormat: "openai-completions", apiKey: "", models: [] };
}

function fromProvider(provider: CustomProviderRecord): Draft {
  return {
    id: provider.id,
    name: provider.name,
    baseUrl: provider.baseUrl,
    apiFormat: provider.apiFormat,
    apiKey: "",
    models: provider.models.map((model) => ({
      ...model,
      thinkingLevels: [...model.thinkingLevels],
      thinkingLevelMap: { ...model.thinkingLevelMap }
    }))
  };
}

interface ModelSuggestionSearchProps {
  modelLabel: string;
  catalog: BuiltinModelSuggestion[];
  loading: boolean;
  error?: string;
  onSelect: (suggestion: BuiltinModelSuggestion) => void;
}

function ModelSuggestionSearch({ modelLabel, catalog, loading, error, onSelect }: ModelSuggestionSearchProps) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const [applied, setApplied] = useState<BuiltinModelSuggestion>();
  const anchor = useRef<HTMLDivElement>(null);
  const listId = useId();
  const matches = useMemo(() => searchBuiltinModels(catalog, query), [catalog, query]);
  const visible = open && matches.length > 0 && !error;

  function choose(suggestion: BuiltinModelSuggestion) {
    onSelect(suggestion);
    setApplied(suggestion);
    setQuery("");
    setOpen(false);
    setActive(-1);
  }

  return (
    <div className="model-catalog-search" ref={anchor}>
      <label>
        <span>Find in Pi catalogue</span>
        <input
          role="combobox"
          aria-label={`Find in Pi catalogue for ${modelLabel}`}
          aria-autocomplete="list"
          aria-expanded={visible}
          aria-controls={visible ? listId : undefined}
          aria-activedescendant={visible && active >= 0 ? `${listId}-${active}` : undefined}
          value={query}
          onChange={(event) => { setQuery(event.target.value); setOpen(true); setActive(-1); }}
          onFocus={() => { if (query.trim()) setOpen(true); }}
          onKeyDown={(event) => {
            if (event.key === "Escape") { setOpen(false); setActive(-1); }
            else if (event.key === "Tab") setOpen(false);
            else if (event.key === "ArrowDown" && matches.length) {
              event.preventDefault();
              setOpen(true);
              setActive((value) => (value + 1) % matches.length);
            } else if (event.key === "ArrowUp" && matches.length) {
              event.preventDefault();
              setOpen(true);
              setActive((value) => value < 0 ? matches.length - 1 : (value - 1 + matches.length) % matches.length);
            } else if (event.key === "Enter" && visible) {
              event.preventDefault();
              choose(matches[active < 0 ? 0 : active]);
            }
          }}
          placeholder="Search by model name or ID"
          autoComplete="off"
          spellCheck={false}
        />
      </label>
      {loading && <small role="status">Loading bundled Pi catalogue…</small>}
      {error && <small role="status">Could not load Pi catalogue: {error} Manual entry is still available.</small>}
      {!loading && !error && query.trim() && matches.length === 0 && <small role="status">No Pi catalogue matches. You can enter settings manually.</small>}
      {applied && <small>Filled from Pi’s {applied.sourceProvider} catalogue entry. Review these settings before saving.</small>}
      <Popover anchor={anchor} open={visible} onClose={() => { setOpen(false); setActive(-1); }} matchWidth className="model-catalog-popover">
        <div id={listId} role="listbox" aria-label="Pi model suggestions" className="model-catalog-options">
          {matches.map((suggestion, index) => (
            <button
              id={`${listId}-${index}`}
              key={`${suggestion.sourceProvider}:${suggestion.id}`}
              type="button"
              role="option"
              aria-selected={index === active}
              className={index === active ? "active" : ""}
              onMouseDown={(event) => event.preventDefault()}
              onMouseEnter={() => setActive(index)}
              onClick={() => choose(suggestion)}
            >
              <strong>{suggestion.name}</strong>
              <span>{suggestion.sourceProvider} · {suggestion.sourceApi} · {suggestion.id}</span>
            </button>
          ))}
        </div>
      </Popover>
    </div>
  );
}

interface ProvidersSectionProps {
  providers: ProviderRecord[];
  selectedId: string;
  onSelect: (id: string) => void;
  onSave: (input: SaveProviderInput) => Promise<ProviderRecord>;
  onDelete: (providerId: string) => Promise<void>;
  onSetProviderEnabled: (providerId: string, enabled: boolean) => Promise<void>;
  builtinModels: BuiltinModelSuggestion[];
  catalogLoading: boolean;
  catalogError?: string;
}

function ProvidersSection({ providers, selectedId, onSelect, onSave, onDelete, onSetProviderEnabled, builtinModels, catalogLoading, catalogError }: ProvidersSectionProps) {
  const candidate = providers.find((provider) => provider.id === selectedId);
  const selected = candidate?.kind === "custom" ? candidate : undefined;
  const [draft, setDraft] = useState<Draft>(() => selected ? fromProvider(selected) : blankDraft());
  const [modelCardKeys, setModelCardKeys] = useState<number[]>(() => newModelCardKeys(selected?.models.length ?? 0));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const [confirmDelete, setConfirmDelete] = useState(false);

  // Reseed from the saved record when the selection changes or its editable fields change —
  // keyed by content, so the enabled switch (which rewrites `providers` live) keeps unsaved
  // form edits, as does anything else that leaves the fields below untouched.
  const seed = selected ? fromProvider(selected) : blankDraft();
  const seedKey = JSON.stringify(seed);
  useEffect(() => {
    setDraft(seed);
    setModelCardKeys(newModelCardKeys(selected?.models.length ?? 0));
    setError(undefined);
    setNotice(undefined);
  }, [selectedId, seedKey]);

  const incomplete = useMemo(() => draft.models.filter((model) => !modelIsReady(model)).length, [draft.models]);

  async function save(): Promise<ProviderRecord | undefined> {
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      const saved = await onSave({ ...draft, apiKey: draft.apiKey?.trim() || undefined });
      onSelect(saved.id);
      setDraft(saved.kind === "custom" ? fromProvider(saved) : blankDraft());
      setModelCardKeys(newModelCardKeys(saved.models.length));
      setNotice("Connection saved. The API key is stored on this device.");
      return saved;
    } catch (reason) {
      setError(String(reason));
      return undefined;
    } finally {
      setBusy(false);
    }
  }

  async function fetchModels() {
    const saved = await save();
    if (!saved) return;
    setBusy(true);
    setError(undefined);
    try {
      const ids = await api.discoverModels(saved.id);
      const models = mergeDiscoveredModels(saved.models, ids);
      setDraft({ ...(saved.kind === "custom" ? fromProvider(saved) : blankDraft()), models });
      setModelCardKeys(newModelCardKeys(models.length));
      setNotice(`Found ${ids.length} model${ids.length === 1 ? "" : "s"}. Confirm limits for new entries, then save.`);
    } catch (reason) {
      setError(`${String(reason)} Manual model entry is still available below.`);
    } finally {
      setBusy(false);
    }
  }

  function updateModel(index: number, patch: Partial<ModelRecord>) {
    setDraft((current) => ({
      ...current,
      models: current.models.map((model, modelIndex) => modelIndex === index ? { ...model, ...patch } : model)
    }));
  }

  function applySuggestion(index: number, suggestion: BuiltinModelSuggestion) {
    setDraft((current) => ({
      ...current,
      models: current.models.map((model, modelIndex) => modelIndex === index
        ? applyBuiltinModelSuggestion(model, suggestion) : model)
    }));
  }

  function toggleLevel(index: number, level: ThinkingLevel) {
    const model = draft.models[index];
    const present = model.thinkingLevels.includes(level);
    const next = present ? model.thinkingLevels.filter((item) => item !== level) : [...model.thinkingLevels, level];
    const thinkingLevelMap = { ...model.thinkingLevelMap };
    if (present) delete thinkingLevelMap[level];
    else thinkingLevelMap[level] = level === "off" ? null : level;
    if (next.length === 0) thinkingLevelMap.off = null;
    updateModel(index, { thinkingLevels: next.length ? next : ["off"], thinkingLevelMap });
  }

  function updateThinkingMapping(index: number, level: ThinkingLevel, value: string) {
    const model = draft.models[index];
    updateModel(index, {
      thinkingLevelMap: {
        ...model.thinkingLevelMap,
        [level]: value.trim() ? value : level === "off" ? null : level
      }
    });
  }

  async function removeProvider() {
    if (!draft.id) return;
    setBusy(true);
    setError(undefined);
    try {
      await onDelete(draft.id);
      onSelect("new");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function toggleEnabled() {
    if (!selected) return;
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      await onSetProviderEnabled(selected.id, selected.enabled === false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <div className="settings-scroll">
        {selected && (
          <div className="connection-use-row">
            <div>
              <strong>Use this connection</strong>
              <small>Turned off, it keeps its key and models here but disappears from the model picker.</small>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={selected.enabled !== false}
              aria-label={`Use ${selected.name}`}
              className={`toggle ${selected.enabled !== false ? "on" : ""}`}
              disabled={busy}
              onClick={() => void toggleEnabled()}
            >
              <span />
            </button>
          </div>
        )}
        <div className="form-grid connection-grid">
          <label>
            <span>Name</span>
            <input value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="My inference gateway" />
          </label>
          <label>
            <span>API format</span>
            <Select
              className="settings-select"
              matchWidth
              value={draft.apiFormat}
              onChange={(value) => setDraft({ ...draft, apiFormat: value as ApiFormat })}
              options={[
                { value: "openai-completions", label: "Chat Completions compatible" },
                { value: "openai-responses", label: "Responses compatible" }
              ]}
              aria-label="API format"
            />
          </label>
          <label>
            <span>Base URL</span>
            <input value={draft.baseUrl} onChange={(event) => setDraft({ ...draft, baseUrl: event.target.value })} placeholder="https://api.example.com/v1" spellCheck={false} />
          </label>
          <label>
            <span>API key <small>{draft.id && selected?.hasApiKey ? "Leave blank to keep the saved key" : "Stored on this device"}</small></span>
            <div className="input-with-icon">
              <Icon name="key" />
              <input type="password" autoComplete="off" value={draft.apiKey ?? ""} onChange={(event) => setDraft({ ...draft, apiKey: event.target.value })} placeholder={selected?.hasApiKey ? "••••••••••••••••" : "Enter API key"} />
            </div>
          </label>
        </div>

        <div className="section-heading-row">
          <div>
            <h3>Models</h3>
            <p>Discovery adds IDs only. Search Pi’s bundled catalogue for suggested settings, then review them before saving.</p>
          </div>
          <div className="row-actions">
            <button className="secondary-button" disabled={busy} onClick={fetchModels}><Icon name="refresh" /> Fetch models</button>
            <button className="secondary-button" onClick={() => {
              setDraft((current) => ({ ...current, models: [...current.models, {
                id: "", name: "", contextWindow: null, maxTokens: null, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null }, vision: false
              }] }));
              setModelCardKeys((keys) => [...keys, ++nextModelCardKey]);
            }}><Icon name="plus" /> Add manually</button>
          </div>
        </div>

        {draft.models.length === 0 ? (
          <div className="model-empty">Fetch from the provider or add a model ID manually.</div>
        ) : (
          <div className="model-list">
            {draft.models.map((model, index) => (
              <article className={`model-card ${modelIsReady(model) ? "" : "incomplete"}`} key={modelCardKeys[index]}>
                <div className="model-card-top">
                  <div className="model-index">{String(index + 1).padStart(2, "0")}</div>
                  <label><span>Model ID</span><input value={model.id} onChange={(event) => updateModel(index, { id: event.target.value })} placeholder="provider/model-id" spellCheck={false} /></label>
                  <label><span>Display name</span><input value={model.name} onChange={(event) => updateModel(index, { name: event.target.value })} placeholder={model.id || "Model name"} /></label>
                  <button className="icon-button" aria-label="Remove model" onClick={() => {
                    setDraft((current) => ({ ...current, models: current.models.filter((_, modelIndex) => modelIndex !== index) }));
                    setModelCardKeys((keys) => keys.filter((_, modelIndex) => modelIndex !== index));
                  }}><Icon name="trash" /></button>
                </div>
                <ModelSuggestionSearch
                  modelLabel={model.name || model.id || `model ${index + 1}`}
                  catalog={builtinModels}
                  loading={catalogLoading}
                  error={catalogError}
                  onSelect={(suggestion) => applySuggestion(index, suggestion)}
                />
                <div className="model-limits">
                  <label><span>Context tokens</span><input type="number" min="1" value={model.contextWindow ?? ""} onChange={(event) => updateModel(index, { contextWindow: event.target.value ? Number(event.target.value) : null })} placeholder="Required" /></label>
                  <label><span>Max output tokens</span><input type="number" min="1" value={model.maxTokens ?? ""} onChange={(event) => updateModel(index, { maxTokens: event.target.value ? Number(event.target.value) : null })} placeholder="Required" /></label>
                  <label className="reasoning-toggle"><span>Reasoning</span><button className={`toggle ${model.reasoning ? "on" : ""}`} onClick={() => updateModel(index, {
                    reasoning: !model.reasoning,
                    thinkingLevels: !model.reasoning ? ["off", "low", "medium", "high"] : ["off"],
                    thinkingLevelMap: !model.reasoning ? { off: null, low: "low", medium: "medium", high: "high" } : { off: null }
                  })} type="button"><span /></button></label>
                  <label className="capability-toggle"><span>Vision</span><button
                    type="button"
                    role="switch"
                    aria-checked={model.vision}
                    aria-label={`Vision for ${model.name || model.id || "this model"}`}
                    title="Accepts image attachments"
                    className={`toggle ${model.vision ? "on" : ""}`}
                    onClick={() => updateModel(index, { vision: !model.vision })}
                  ><span /></button></label>
                </div>
                {model.reasoning && (
                  <>
                    <div className="reasoning-levels">
                      <span>Supported efforts</span>
                      {levels.map((level) => <button key={level} type="button" className={model.thinkingLevels.includes(level) ? "selected" : ""} onClick={() => toggleLevel(index, level)}>{level}</button>)}
                    </div>
                    <div className="reasoning-mappings">
                      <span>Provider values</span>
                      {model.thinkingLevels.map((level) => (
                        <label key={level}>
                          <span>{level}</span>
                          <input
                            value={model.thinkingLevelMap[level] ?? ""}
                            onChange={(event) => updateThinkingMapping(index, level, event.target.value)}
                            placeholder={level === "off" ? "omit" : level}
                            spellCheck={false}
                          />
                        </label>
                      ))}
                      <small>Blank “off” omits reasoning; other blanks use the effort name.</small>
                    </div>
                  </>
                )}
                {!modelIsReady(model) && <div className="model-warning">Confirm context and output limits before this model can be used.</div>}
              </article>
            ))}
          </div>
        )}
      </div>

      <footer className="settings-footer">
        <div className="form-status">
          {error && <span className="error-text">{error}</span>}
          {!error && notice && <span className="success-text">{notice}</span>}
          {!error && !notice && incomplete > 0 && <span>{incomplete} model{incomplete === 1 ? " needs" : "s need"} limits</span>}
        </div>
        {draft.id && <button className="danger-button" disabled={busy} onClick={() => setConfirmDelete(true)}>Delete</button>}
        <button className="primary-button" disabled={busy || !draft.name.trim() || !draft.baseUrl.trim()} onClick={save}>{busy ? "Working…" : "Save connection"}</button>
      </footer>

      {confirmDelete && (
        <ConfirmDialog
          title={`Delete “${draft.name || "this connection"}”?`}
          body="This removes the connection and its saved API key."
          confirmLabel="Delete"
          danger
          onConfirm={removeProvider}
          onCancel={() => setConfirmDelete(false)}
        />
      )}
    </>
  );
}

interface SubscriptionCatalogProps {
  providers: SubscriptionProviderInfo[];
  error?: string;
  onConnect: (providerId: string) => Promise<void>;
}

function SubscriptionCatalog({ providers, error, onConnect }: SubscriptionCatalogProps) {
  return <div className="settings-scroll subscription-catalog">
    <h3>Sign in with a subscription</h3>
    <p>WackCode uses Pi’s built-in sign-in for these providers. Your account remains separate from any Pi CLI installation.</p>
    {error && <div className="error-banner">Could not load subscription providers: {error}</div>}
    {providers.map((provider) => <article className="subscription-provider-card" key={provider.id}>
      <div><strong>{provider.name}</strong><p>{provider.guidance}</p></div>
      <button type="button" className="primary-button" onClick={() => void onConnect(provider.id)}>Sign in</button>
    </article>)}
    <p className="subscription-billing-note">Anthropic may charge usage credits for third-party app access. <button type="button" className="text-button" onClick={() => void api.openSubscriptionAuthUrl("https://support.claude.com/en/articles/13189465-log-in-to-your-claude-account")}>Review Anthropic’s guidance</button></p>
  </div>;
}

interface SubscriptionSectionProps {
  provider: ProviderRecord;
  guidance?: string;
  onConnect: (providerId: string) => Promise<void>;
  onSignOut: (providerId: string) => Promise<void>;
  onDelete: (providerId: string) => Promise<void>;
  onSetProviderEnabled: (providerId: string, enabled: boolean) => Promise<void>;
  onSelect: (id: string) => void;
}

function SubscriptionSection({ provider, guidance, onConnect, onSignOut, onDelete, onSetProviderEnabled, onSelect }: SubscriptionSectionProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [confirmSignOut, setConfirmSignOut] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  if (provider.kind !== "subscription") return null;

  async function action(run: () => Promise<void>) {
    setBusy(true);
    setError(undefined);
    try { await run(); }
    catch (reason) { setError(String(reason)); throw reason; }
    finally { setBusy(false); }
  }

  return <>
    <div className="settings-scroll subscription-detail">
      <div className="connection-use-row">
        <div>
          <strong>Use this connection</strong>
          <small>Turned off, it keeps its sign-in and models here but disappears from the model picker.</small>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={provider.enabled !== false}
          aria-label={`Use ${provider.name}`}
          className={`toggle ${provider.enabled !== false ? "on" : ""}`}
          disabled={busy}
          onClick={() => void action(() => onSetProviderEnabled(provider.id, provider.enabled === false))}
        >
          <span />
        </button>
      </div>
      <p className="subscription-status"><span className={`credential-dot ${provider.connected ? "connected" : ""}`} /> {provider.connected ? "Signed in" : "Signed out"}</p>
      {guidance && <p>{guidance}</p>}
      {provider.id === "anthropic" && <button type="button" className="text-button" onClick={() => void api.openSubscriptionAuthUrl("https://support.claude.com/en/articles/13189465-log-in-to-your-claude-account").catch((reason) => setError(String(reason)))}>Review Anthropic’s billing guidance</button>}
      <p>Pi manages this provider’s models and refreshes its credential when you send a request. Sign in again if authentication fails.</p>
      <button type="button" className="primary-button" disabled={busy} onClick={() => void onConnect(provider.id)}>{provider.connected ? "Reconnect" : "Sign in"}</button>
      <h3>{provider.connected ? "Available models" : "Last known models"}</h3>
      {provider.models.length ? <ul className="subscription-model-list">{provider.models.map((model) => <li key={model.id}><strong>{model.name}</strong><span>{model.id}</span></li>)}</ul>
        : <p>{provider.connected ? "No models are available to this account." : "Sign in to load models available to this account."}</p>}
    </div>
    <footer className="settings-footer">
      {error && <span className="error-text">{error}</span>}
      <button type="button" className="danger-button" disabled={busy} onClick={() => setConfirmDelete(true)}>Delete</button>
      {provider.connected && <button type="button" className="secondary-button" disabled={busy} onClick={() => setConfirmSignOut(true)}>Sign out</button>}
    </footer>
    {confirmSignOut && <ConfirmDialog title={`Sign out of ${provider.name}?`} body="Saved chats will remain, but they cannot use this connection until you sign in again." confirmLabel="Sign out" onConfirm={() => action(() => onSignOut(provider.id))} onCancel={() => setConfirmSignOut(false)} />}
    {confirmDelete && <ConfirmDialog title={`Delete ${provider.name}?`} body="This removes its WackCode sign-in. Saved chats must use another connection before deletion." confirmLabel="Delete" danger onConfirm={() => action(async () => { await onDelete(provider.id); onSelect("new"); })} onCancel={() => setConfirmDelete(false)} />}
  </>;
}
