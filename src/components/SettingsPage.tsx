import { Fragment, useEffect, useState } from "react";
import { api } from "../api";
import { agentName } from "../agentName";
import { modelIsReady } from "../model-utils";
import { BROWSER_TOOL_NAMES, WEB_FETCH_TOOL_NAME } from "../tool-utils";
import type { ExecutionPolicyConfig, AppearanceConfig, AutoTitleConfig, BuiltinModelSuggestion, CommandsConfig, ComputerUseConfig, McpConfig, MemoryConfig, PackageRecord, PromptConfig, ProviderRecord, SaveProviderInput, SubagentConfig, SubscriptionProviderInfo, ToolCatalogEntry } from "../types";
import { Icon, type IconName } from "./Icons";
import { AboutSection } from "./AboutSection";
import { CommandsSection, type SlashCommandActions } from "./CommandsSection";
import { MemorySection, type MemoryActions } from "./MemorySection";
import { ComputerUseSection, type ComputerUseActions } from "./ComputerUseSection";
import { ComputerUseSetupDialog } from "./ComputerUseSetupDialog";
import { IntegrationsSection } from "./IntegrationsSection";
import { McpSection, type McpActions } from "./McpSection";
import { PackagesSection, type PackageActions } from "./PackagesSection";
import { PromptsSection } from "./PromptsSection";
import { ProvidersSection, type ConnectionMethod } from "./ProvidersSection";
import { SkillsSection, type SkillActions } from "./SkillsSection";
import { SubagentsSection } from "./SubagentsSection";
import type { ModelFavoritesProps } from "./ModelPicker";
import { ToolsSection } from "./ToolsSection";
import { AppearanceSection } from "./AppearanceSection";
import { DuckMark } from "./DuckMark";
import { Tooltip } from "./ui/Tooltip";

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
const COMPUTER_USE_OFF: ComputerUseConfig = { enabled: false, showAgentCursor: true, neverAllow: [] };

type SectionId = "providers" | "packages" | "skills" | "commands" | "memory" | "tools" | "mcp" | "appearance" | "prompts" | "subagents" | "computer_use" | "integrations" | "about";

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

interface Props extends PackageActions, ModelFavoritesProps {
  providers: ProviderRecord[];
  packages: PackageRecord[];
  toolCatalog: ToolCatalogEntry[];
  disabledTools: string[];
  executionPolicy?: ExecutionPolicyConfig;
  onSetExecutionPolicy?: (config: ExecutionPolicyConfig) => Promise<void>;
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
  executionPolicy, onSetExecutionPolicy, providers, favoriteModels, favoriteSaving, onSetFavorite, packages, toolCatalog, disabledTools, appDataPath,
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
  /** Undefined shows the connections overview; "new" a new connection. */
  const [selectedProviderId, setSelectedProviderId] = useState<string>();
  /** Bumped on every navigation between connections, so each opens in a fresh editor. */
  const [providerEditor, setProviderEditor] = useState(0);
  const [builtinModels, setBuiltinModels] = useState<BuiltinModelSuggestion[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [catalogError, setCatalogError] = useState<string>();
  const [subscriptionProviders, setSubscriptionProviders] = useState<SubscriptionProviderInfo[]>([]);
  const [subscriptionError, setSubscriptionError] = useState<string>();
  const [newMethod, setNewMethod] = useState<ConnectionMethod>("apiKey");
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

  function openProvider(id?: string) {
    setSelectedProviderId(id);
    setProviderEditor((value) => value + 1);
    setSection("providers");
  }

  useEffect(() => {
    if (connectedSubscriptionId) openProvider(connectedSubscriptionId);
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
                onClick={() => (item.id === "providers" ? openProvider(undefined) : setSection(item.id))}
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
                          onClick={() => openProvider(provider.id)}
                        >
                          <span className={`credential-dot ${provider.connected ? "connected" : ""}`} />
                          <span>{provider.name}</span>
                        </button>
                      ))}
                      <button
                        type="button"
                        className={`settings-subnav-item ${selectedProviderId === "new" ? "active" : ""}`}
                        onClick={() => { setNewMethod("apiKey"); openProvider("new"); }}
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
        <div className="sidebar-footer settings-sidebar-footer">
          <button type="button" className="sidebar-action" title={appDataPath} onClick={() => void api.revealPath(appDataPath).catch(() => undefined)}>
            <Icon name="folder" /> Data folder
          </button>
          {/* Not a nav section: About is the app's own page, so it lives in the footer with the
              Data folder rather than between the agent's features. */}
          <button type="button" className={`sidebar-action ${section === "about" ? "active" : ""}`} onClick={() => setSection("about")}>
            <DuckMark /> About
          </button>
        </div>
      </aside>

      <main className="workspace settings-workspace">
        <header className="settings-page-head">
          <div>
            <h2>{section === "about" ? "About" : selectedSection?.label}</h2>
          </div>
          <button type="button" className="secondary-button" onClick={onClose}>Done</button>
        </header>
        {section === "providers" && (
          <ProvidersSection
            providers={providers}
            selectedId={selectedProviderId}
            editorKey={providerEditor}
            newMethod={newMethod}
            agentName={agentName(appearance)}
            builtinModels={builtinModels}
            catalogLoading={catalogLoading}
            catalogError={catalogError}
            subscriptionProviders={subscriptionProviders}
            subscriptionError={subscriptionError}
            onSelect={openProvider}
            onCreated={setSelectedProviderId}
            onStartNew={(method) => { setNewMethod(method); openProvider("new"); }}
            onSave={onSave}
            onDelete={onDelete}
            onSetProviderEnabled={onSetProviderEnabled}
            onConnectSubscription={onConnectSubscription}
            onSignOutSubscription={onSignOutSubscription}
            onDiscover={api.discoverModels}
            onOpenAuthUrl={api.openSubscriptionAuthUrl}
          />
        )}
        {section === "integrations" && <IntegrationsSection />}
        {section === "packages" && (
          <PackagesSection
            packages={packages}
            agentName={agentName(appearance)}
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
            agentName={agentName(appearance)}
            onInstallSkills={(source) => onInstall(source, { skillsOnly: true })}
            onSetPackageSkills={(source, enabled) => onSetResources(source, "skills", enabled)}
            onOpenPackages={() => setSection("packages")}
          />
        )}
        {section === "commands" && (
          <CommandsSection {...COMMAND_ACTIONS} agentName={agentName(appearance)} onChanged={onCommandsChanged} />
        )}
        {section === "memory" && (
          <MemorySection
            {...MEMORY_ACTIONS}
            agentName={agentName(appearance)}
            onSetEnabled={(enabled) => onSetMemory({ ...memory, enabled })}
          />
        )}
        {section === "tools" && (
          <ToolsSection
            executionPolicy={executionPolicy}
            onSetExecutionPolicy={onSetExecutionPolicy}
            catalog={toolCatalog}
            disabled={disabledTools}
            packages={packages}
            agentName={agentName(appearance)}
            onSetDisabled={onSetDisabledTools}
            onOpen={setSection}
            mcpAvailable={Boolean(mcpActions)}
          />
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
        {section === "prompts" && <PromptsSection unrestrictedPlanning={executionPolicy?.unrestrictedPlanning} config={prompts} agentName={agentName(appearance)} onChange={onSetPrompts} />}
        {section === "mcp" && mcpActions && <McpSection servers={mcp?.servers ?? []} agentName={agentName(appearance)} {...mcpActions} />}
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
        {section === "about" && <AboutSection />}
        {section === "subagents" && (
          <SubagentsSection
            executionPolicy={executionPolicy}
            config={subagents}
            providers={providers}
            favoriteModels={favoriteModels}
            favoriteSaving={favoriteSaving}
            onSetFavorite={onSetFavorite}
            onChange={onSetSubagents}
            webFetchEnabled={!disabledTools.includes(WEB_FETCH_TOOL_NAME)}
            autoTitle={autoTitle}
            onSetAutoTitle={onSetAutoTitle}
            onOpenProviders={() => { setNewMethod("apiKey"); openProvider("new"); }}
            agentName={agentName(appearance)}
          />
        )}
      </main>
    </>
  );
}
