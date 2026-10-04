import { useCallback, useEffect, useState } from "react";
import { PACKAGE_RESOURCE_KINDS, type PackageRecord, type PackageResourceKind, type PackageSearchResult } from "../types";
import { Icon, type IconName } from "./Icons";
import { DuckMark } from "./DuckMark";
import { PackageBrowser, type BrowsePage } from "./PackageBrowser";
import { SettingsHero, stagger } from "./SettingsHero";
import { TrustDialog } from "./TrustDialog";
import { ConfirmDialog } from "./ui/ConfirmDialog";

const KIND_LABELS: Record<PackageResourceKind, string> = {
  extensions: "Extensions",
  skills: "Skills",
  prompts: "Prompts",
  themes: "Themes"
};

/** Monogram short labels for a package's resource kinds, used in the chips on its card. */
const KIND_SHORT_LABELS: Record<PackageResourceKind, string> = {
  extensions: "ext",
  skills: "skill",
  prompts: "prompt",
  themes: "theme"
};

/** Which glyph a package card shows, by where the package came from. */
const PACKAGE_KIND_ICONS: Record<PackageRecord["kind"], IconName> = {
  npm: "archive",
  git: "git",
  local: "folder"
};

interface BuiltinExtension {
  name: string;
  description: string;
  tools: readonly string[];
  icon: IconName;
  kind?: "subagents" | "auto_titles" | "web_fetch" | "browser" | "computer_use" | "mcp" | "memory";
}

/**
 * The extensions compiled into WackCode itself (worker/src/builtin/index.ts). They are shown
 * here so nobody installs a package duplicating something that already ships with the app.
 * They have no trust gate; optional built-ins have their own settings — keep this list in sync
 * with the worker.
 */
const BUILTIN_EXTENSIONS: readonly BuiltinExtension[] = [
  {
    name: "Plan Mode",
    description:
      "A planning mode, read-only by default. Toggle Build/Plan in the composer or press ⇧Tab — the agent inspects the workspace and proposes a plan for approval. Read-only restrictions can be removed in Settings › Tools. Click Plan again for Ultra Plan: the agent interviews you one question at a time, with no limit, before it plans. Expect more usage.",
    tools: ["plan_mode_complete"],
    icon: "brain"
  },
  {
    name: "Ask User Questions",
    description:
      "Lets the agent ask structured multiple-choice questions in a native dialog instead of guessing. Available in every mode.",
    tools: ["ask_user_question"],
    icon: "question"
  },
  {
    name: "Todo List",
    description:
      "A live task list the agent keeps up to date while it works, shown above the composer. Rebuilt from the conversation, so it survives restarts and compaction.",
    tools: ["todo"],
    icon: "checklist"
  },
  {
    name: "Sub-agents",
    description:
      "Lets the agent hand self-contained tasks to sub-agents with their own context window, one at a time or several in parallel. Off by default: every sub-agent is extra model usage.",
    tools: ["subagent"],
    icon: "agents",
    kind: "subagents"
  },
  {
    name: "Browser preview",
    description:
      "Lets the agent and you share an isolated, session-only WebKit browser for testing local web apps and deployed sites. Pages can make their own HTTP, HTTPS, WebSocket and asset requests, including while their chat is in the background.",
    tools: ["browser_open", "browser_snapshot", "browser_act", "browser_screenshot", "browser_console"],
    icon: "browser",
    kind: "browser"
  },
  {
    name: "Computer use",
    description:
      "Lets the agent open, look at and operate native app windows — for checking the apps it builds. Off by default: it needs macOS's Accessibility and Screen Recording permissions, the agent asks in the chat before using each app, and ⌃⌥⌘. stops it.",
    tools: ["computer_apps", "computer_open", "computer_snapshot", "computer_screenshot", "computer_act"],
    icon: "cursor",
    kind: "computer_use"
  },
  {
    name: "Web Fetch",
    description:
      "Lets the agent read a web page by URL, returned as Markdown. Public addresses only: it can't reach localhost or your local network, and it doesn't search. The only built-in that contacts sites you didn't configure.",
    tools: ["web_fetch"],
    icon: "external",
    kind: "web_fetch"
  },
  {
    name: "Memory",
    description:
      "Notes the agent keeps for itself across chats, one folder per repository: your preferences, corrections, decisions and pointers that AGENTS.md files don't carry. A one-line index rides in every chat; full notes are read only when relevant. Switch it on or off in Settings › Memory.",
    tools: ["memory_save", "memory_recall", "memory_forget"],
    icon: "memory",
    kind: "memory"
  },
  {
    name: "MCP servers",
    description:
      "Gives the agent the tools of the MCP servers you add: local commands (stdio) or remote servers over HTTP or SSE, each with its own switch. Servers start when a chat sends its first message.",
    tools: [],
    icon: "plug",
    kind: "mcp"
  },
  {
    name: "Auto chat titles",
    description: "Give new chats a short title from their first message — one extra model request per chat on a model you pick. It lives on the Sub-agents page, and pauses while Sub-agents is off.",
    tools: [],
    icon: "spark",
    kind: "auto_titles"
  },
  {
    name: "Goal loop",
    description:
      "Keeps the agent iterating until the goal is verified: /goal <objective> starts a loop, and after every round a separate no-tools check on the chat's own model decides whether the objective is met or supplies the next action. Pauses itself after 3 rounds without progress and stops at 25 rounds; /goal pause, resume and clear control it.",
    tools: [],
    icon: "flame"
  }
];

/**
 * The hero's little stage: the duck beside a board of modules, the built-ins, lighting up in a
 * wave, while a new one drops into the empty slot as a package would. Pure decoration; the pill
 * says the same in words. The loop lives in styles.css, which stills it under reduced motion.
 */
function PackagesStage() {
  const slots = [0, 1, 2, 3, 4, 5, 6, 7, 8];
  return (
    <svg className="settings-stage packages-stage live" viewBox="0 0 160 110" aria-hidden="true">
      <rect className="packages-stage-tile" x="12" y="33" width="44" height="44" rx="12" />
      <DuckMark x="22" y="43" width="24" height="24" />
      <path className="packages-stage-bus" d="M56 55h12M68 29v52M68 29h8M68 55h8M68 81h8" />
      {slots.map((slot) => {
        const x = 78 + (slot % 3) * 24;
        const y = 19 + Math.floor(slot / 3) * 26;
        const incoming = slot === 8;
        return (
          <g key={slot} className={`packages-stage-module ${incoming ? "incoming" : ""}`} style={{ "--k": slot } as React.CSSProperties}>
            {incoming && <rect className="packages-stage-slot" x={x} y={y} width="20" height="20" rx="6" />}
            <rect className="packages-stage-block" x={x} y={y} width="20" height="20" rx="6" />
            <circle className="packages-stage-light" cx={x + 10} cy={y + 10} r="3" />
          </g>
        );
      })}
    </svg>
  );
}

export interface PackageActions {
  /** Re-reads the shared package store. The cached list can be stale if `pi` was used elsewhere. */
  onRefresh: () => Promise<void>;
  /** `skillsOnly`: Settings › Skills installs switch on only the package's skills. */
  onInstall: (source: string, options?: { skillsOnly?: boolean }) => Promise<void>;
  /** Grant trust to a package already in the store that was never confirmed here. */
  onTrust: (source: string) => Promise<void>;
  onSearch: (query: string) => Promise<PackageSearchResult[]>;
  onRemove: (source: string) => Promise<void>;
  onUpdate: (source: string) => Promise<void>;
  onSetResources: (source: string, kind: PackageResourceKind, enabled: string[]) => Promise<void>;
}

interface Props extends PackageActions {
  packages: PackageRecord[];
  /** The agent's name in the app's own copy (Settings › Appearance). */
  agentName?: string;
  subagentsEnabled?: boolean;
  /** The Memory master switch (Settings › Memory); the card shows state, the page holds the switch. */
  memoryEnabled?: boolean;
  /** On unless `web_fetch` is in the tool denylist. */
  webFetchEnabled?: boolean;
  browserEnabled?: boolean;
  computerUseEnabled?: boolean;
  /** macOS 14+; otherwise the switch stays off. */
  computerUseSupported?: boolean;
  autoTitlesEnabled?: boolean;
  autoTitlesConfigured?: boolean;
  onToggleSubagents?: (enabled: boolean) => Promise<void>;
  onToggleWebFetch?: (enabled: boolean) => Promise<void>;
  onToggleBrowser?: (enabled: boolean) => Promise<void>;
  onToggleComputerUse?: (enabled: boolean) => Promise<void>;
  /** Opens Settings → Computer use. */
  onConfigureComputerUse?: () => void;
  onToggleAutoTitles?: (enabled: boolean) => Promise<void>;
  onConfigureAutoTitles?: () => void;
  /** Settings › Packages › Memory card: opens Settings › Memory, where the switch lives. */
  onConfigureMemory?: () => void;
  /** Opens Settings → Sub-agents. */
  onConfigureSubagents?: () => void;
  /** Opens Settings → MCP servers. */
  onConfigureMcp?: () => void;
}

type Tab = "installed" | "browse";

export function PackagesSection({
  packages, agentName = "WackCode", subagentsEnabled = false, webFetchEnabled = true, browserEnabled = true, computerUseEnabled = false, computerUseSupported = true, memoryEnabled = true,
  autoTitlesEnabled = false, autoTitlesConfigured = false,
  onToggleSubagents, onToggleWebFetch, onToggleBrowser, onToggleComputerUse, onConfigureComputerUse, onToggleAutoTitles, onConfigureSubagents, onConfigureAutoTitles, onConfigureMemory, onConfigureMcp,
  onRefresh, onInstall, onTrust, onSearch, onRemove, onUpdate, onSetResources
}: Props) {
  const [tab, setTab] = useState<Tab>("installed");
  const [source, setSource] = useState("");
  const [pendingTrust, setPendingTrust] = useState<{ source: string; mode: "install" | "enable" }>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [removing, setRemoving] = useState<PackageRecord>();
  const [loading, setLoading] = useState(true);
  // npm's registry search answers one page, so there is never a "more".
  const searchPage = useCallback(
    (query: string): Promise<BrowsePage> => onSearch(query).then((results) => ({ results, hasMore: false })),
    [onSearch]
  );

  // The cached list is only as fresh as the last mutation, so reconcile with disk on open.
  // A failure here is reported but never blocks the rest of Settings.
  useEffect(() => {
    let active = true;
    void onRefresh()
      .catch((reason: unknown) => { if (active) setError(String(reason)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [onRefresh]);

  async function run(action: () => Promise<void>): Promise<void> {
    setBusy(true);
    setError(undefined);
    try {
      await action();
    } catch (reason) {
      setError(String(reason));
      throw reason;
    } finally {
      setBusy(false);
    }
  }

  async function acceptTrust(): Promise<void> {
    const pending = pendingTrust;
    if (!pending) return;
    try {
      await run(() => pending.mode === "install" ? onInstall(pending.source) : onTrust(pending.source));
      setPendingTrust(undefined);
      setSource("");
    } catch {
      // The dialog stays open so the user can read the failure and retry.
    }
  }

  const builtinEnabled = (extension: BuiltinExtension) =>
    extension.kind === "subagents" ? subagentsEnabled
    : extension.kind === "browser" ? browserEnabled
    : extension.kind === "computer_use" ? computerUseEnabled
    : extension.kind === "web_fetch" ? webFetchEnabled
    : extension.kind === "auto_titles" ? autoTitlesEnabled
    : extension.kind === "memory" ? memoryEnabled
    : true;
  const builtinsOn = BUILTIN_EXTENSIONS.filter(builtinEnabled).length;
  const pill = packages.length
    ? `${packages.length} ${packages.length === 1 ? "package" : "packages"} · ${builtinsOn} built-ins on`
    : `${builtinsOn} of ${BUILTIN_EXTENSIONS.length} built-ins on`;

  return (
    <div className="settings-scroll packages-settings">
      <div className="settings-page">
        <SettingsHero
          label="Packages overview"
          stage={<PackagesStage />}
          live
          pill={pill}
          title={`Add to what ${agentName} can do`}
          action={tab === "installed" && packages.length > 0 && (
            <button type="button" className="secondary-button compact" onClick={() => setTab("browse")}>
              <Icon name="search" /> Browse packages
            </button>
          )}
        >
          <p>
            WackCode ships with the built-ins below. Pi packages add more tools, skills and prompts; only what you switch on
            here loads, and nothing from a project&rsquo;s own <code>.pi</code> folder ever runs.
          </p>
        </SettingsHero>

        <div className="package-tabs" role="tablist">
          {(["installed", "browse"] as Tab[]).map((id) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={tab === id}
              className={`package-tab ${tab === id ? "active" : ""}`}
              onClick={() => setTab(id)}
            >
              {id === "installed" ? `Installed${packages.length ? ` (${packages.length})` : ""}` : "Browse"}
            </button>
          ))}
        </div>

        {tab === "browse" ? (
          <section className="settings-block skills-browse" style={stagger(1)}>
            <PackageBrowser
              installed={new Set(packages.map((entry) => entry.source))}
              busy={busy}
              onSearch={searchPage}
              onInstall={(next) => setPendingTrust({ source: next, mode: "install" })}
            />
          </section>
        ) : (
          <>
            {error && !pendingTrust && <div className="error-banner" role="alert">{error}</div>}
            <section className="settings-block" style={stagger(1)} aria-labelledby="packages-yours-title">
              <h3 className="settings-block-title" id="packages-yours-title">Your packages</h3>
              <p className="settings-block-sub">
                Add one by source: <code>npm:name</code>, <code>git:github.com/user/repo</code>, or an absolute path. You review what it
                can reach before anything is installed.
              </p>
              <form
                className="package-add"
                onSubmit={(event) => { event.preventDefault(); if (source.trim()) setPendingTrust({ source: source.trim(), mode: "install" }); }}
              >
                <input
                  value={source}
                  onChange={(event) => setSource(event.target.value)}
                  placeholder="npm:pi-web-access"
                  aria-label="Package source"
                  spellCheck={false}
                />
                <button type="submit" className="primary-button compact" disabled={busy || !source.trim()}>
                  <Icon name="plus" /> Add package
                </button>
              </form>

              {loading && packages.length === 0 ? (
                <p className="models-empty">Loading packages…</p>
              ) : packages.length === 0 ? (
                <div className="packages-empty">
                  <p>Nothing installed yet. A package can add tools, skills and prompts to every chat; see what other people have published.</p>
                  <button type="button" className="secondary-button compact" onClick={() => setTab("browse")}>
                    <Icon name="search" /> Browse packages
                  </button>
                </div>
              ) : (
                <div className="package-grid">
                  {packages.map((entry) => (
                    <PackageCard
                      key={entry.source}
                      entry={entry}
                      busy={busy}
                      onTrust={() => setPendingTrust({ source: entry.source, mode: "enable" })}
                      onRemove={() => setRemoving(entry)}
                      onUpdate={() => void run(() => onUpdate(entry.source)).catch(() => undefined)}
                      onToggle={(kind, enabled) => void run(() => onSetResources(entry.source, kind, enabled)).catch(() => undefined)}
                    />
                  ))}
                </div>
              )}
            </section>

            <section className="settings-block" style={stagger(2)} aria-labelledby="packages-builtin-title">
              <h3 className="settings-block-title" id="packages-builtin-title">Built-ins</h3>
              <p className="settings-block-sub">
                Compiled into WackCode, so you don&rsquo;t need a package for these. The ones with a switch are optional, and MCP does nothing
                until you add a server.
              </p>
              <div className="package-grid">
                {BUILTIN_EXTENSIONS.map((extension) => (
                  <BuiltinCard
                    key={extension.name}
                    extension={extension}
                    enabled={builtinEnabled(extension)}
                    busy={busy}
                    onToggle={extension.kind === "subagents" && onToggleSubagents
                      ? (enabled) => void run(() => onToggleSubagents(enabled)).catch(() => undefined)
                      : extension.kind === "web_fetch" && onToggleWebFetch
                      ? (enabled) => void run(() => onToggleWebFetch(enabled)).catch(() => undefined)
                      : extension.kind === "browser" && onToggleBrowser
                      ? (enabled) => void run(() => onToggleBrowser(enabled)).catch(() => undefined)
                      : extension.kind === "computer_use" && onToggleComputerUse
                      ? (enabled) => void run(() => onToggleComputerUse(enabled)).catch(() => undefined)
                      : extension.kind === "auto_titles" && onToggleAutoTitles
                      ? (enabled) => void run(() => onToggleAutoTitles(enabled)).catch(() => undefined)
                      : undefined}
                    onConfigure={extension.kind === "subagents" ? onConfigureSubagents : extension.kind === "computer_use" ? onConfigureComputerUse : extension.kind === "auto_titles" ? onConfigureAutoTitles : extension.kind === "memory" ? onConfigureMemory : extension.kind === "mcp" ? onConfigureMcp : undefined}
                    toggleDisabled={(extension.kind === "auto_titles" && !autoTitlesEnabled && (!autoTitlesConfigured || !subagentsEnabled)) || (extension.kind === "computer_use" && !computerUseEnabled && !computerUseSupported)}
                    disabledReason={extension.kind === "computer_use" ? "Requires macOS 14 or later" : undefined}
                    subagentsOff={extension.kind === "auto_titles" && !subagentsEnabled}
                  />
                ))}
              </div>
            </section>
          </>
        )}
      </div>

      {pendingTrust && (
        <TrustDialog
          source={pendingTrust.source}
          mode={pendingTrust.mode}
          busy={busy}
          error={error}
          onCancel={() => { setPendingTrust(undefined); setError(undefined); }}
          onConfirm={() => void acceptTrust()}
        />
      )}
      {removing && (
        <ConfirmDialog
          title={`Remove ${removing.displayName}?`}
          body="Its tools, skills, and prompts stop loading. Installed files are deleted."
          confirmLabel="Remove"
          danger
          onConfirm={async () => { await onRemove(removing.source); setRemoving(undefined); }}
          onCancel={() => setRemoving(undefined)}
        />
      )}
    </div>
  );
}

interface BuiltinCardProps {
  extension: BuiltinExtension;
  enabled: boolean;
  busy: boolean;
  toggleDisabled?: boolean;
  /** Tooltip for a disabled switch; auto titles has its own. */
  disabledReason?: string;
  /** Auto titles only: the Sub-agents built-in it rides is off, so setup and enabling are blocked. */
  subagentsOff?: boolean;
  /** Present only for a built-in with an off switch. */
  onToggle?: (enabled: boolean) => void;
  onConfigure?: () => void;
}

/**
 * A built-in extension, in the same card shape as a package. Optional ones (sub-agents, browser
 * preview, computer use, web fetch, auto titles) carry a live switch. Memory's switch lives on its
 * own page, so its card shows the state and links there. The rest are part of the app and say
 * "Always on" rather than showing a switch that can't move. Auto titles lives on the Sub-agents
 * page, so its Set up is blocked while that built-in is off. Its tools are listed as names; a
 * built-in's tools switch together.
 */
function BuiltinCard({ extension, enabled, busy, toggleDisabled = false, disabledReason, subagentsOff = false, onToggle, onConfigure }: BuiltinCardProps) {
  const autoTitles = extension.kind === "auto_titles";
  const elsewhere = extension.kind === "memory";
  const switchable = Boolean(onToggle) || autoTitles;
  const showState = switchable || elsewhere;
  const configurable = onConfigure && (autoTitles || elsewhere || extension.kind === "mcp" || (onToggle && enabled));
  return (
    <article className={`package-card builtin-card ${showState && !enabled ? "off" : ""} ${autoTitles ? "auto-title-package-card" : ""}`} aria-label={extension.name}>
      <div className="package-card-head">
        <span className={`package-icon kind-${extension.kind ?? "core"}`} aria-hidden="true">
          <Icon name={extension.icon} />
        </span>
        <span className="package-name builtin-name">{extension.name}</span>
        {switchable ? (
          <button
            type="button"
            role="switch"
            aria-checked={enabled}
            aria-label={extension.name}
            className={`toggle ${enabled ? "on" : ""}`}
            disabled={!onToggle || busy || toggleDisabled}
            title={toggleDisabled ? (disabledReason ?? (subagentsOff ? "Switch on Sub-agents to use auto titles" : "Choose a title model in Set up first")) : undefined}
            onClick={() => onToggle?.(!enabled)}
          >
            <span />
          </button>
        ) : !elsewhere && (
          <span className="builtin-always"><Icon name="check" /> Always on</span>
        )}
      </div>
      {(showState || configurable) && (
        <div className="package-card-status">
          {showState && <span className={`package-state ${enabled ? "on" : "off"}`}>{enabled ? "On" : "Off"}</span>}
          {configurable && (
            <button
              type="button"
              className="ghost-button builtin-configure"
              disabled={autoTitles && subagentsOff}
              title={autoTitles && subagentsOff ? "Switch on Sub-agents to set up auto titles" : undefined}
              onClick={onConfigure}
            >
              {autoTitles && toggleDisabled ? "Set up" : "Configure"} <Icon name="chevron" />
            </button>
          )}
        </div>
      )}
      <p className="builtin-desc">{extension.description}</p>
      {extension.tools.length > 0 && (
        <ul className={`builtin-tools ${enabled ? "" : "off"}`} aria-label={`${extension.name} tools`}>
          {extension.tools.map((tool) => <li key={tool}>{tool}</li>)}
        </ul>
      )}
    </article>
  );
}

interface CardProps {
  entry: PackageRecord;
  busy: boolean;
  onTrust: () => void;
  onRemove: () => void;
  onUpdate: () => void;
  onToggle: (kind: PackageResourceKind, enabled: string[]) => void;
}

function PackageCard({ entry, busy, onTrust, onRemove, onUpdate, onToggle }: CardProps) {
  const trusted = entry.trustedAt !== "";
  const [open, setOpen] = useState(false);
  const kinds = PACKAGE_RESOURCE_KINDS.filter((kind) => entry[kind].length > 0);
  const total = kinds.reduce((sum, kind) => sum + entry[kind].length, 0);
  const active = kinds.reduce((sum, kind) => sum + entry[kind].filter((resource) => resource.enabled).length, 0);

  function toggle(kind: PackageResourceKind, name: string, next: boolean): void {
    const enabled = entry[kind]
      .filter((resource) => (resource.name === name ? next : resource.enabled))
      .map((resource) => resource.name);
    onToggle(kind, enabled);
  }

  return (
    <article className={`package-card ${entry.errors.length || !trusted ? "incomplete" : ""}`}>
      <div className="package-card-head">
        <span className={`package-icon kind-${entry.kind}`} aria-hidden="true">
          <Icon name={PACKAGE_KIND_ICONS[entry.kind]} />
        </span>
        <button
          type="button"
          className={`package-disclosure ${open ? "open" : ""}`}
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
        >
          <span className="package-name">{entry.displayName}</span>
          <Icon name="chevron" />
        </button>
      </div>
      <div className="package-card-status">
        <span className={`package-state ${!trusted ? "off" : active > 0 ? "on" : "idle"}`}>
          {!trusted ? "Needs review" : active > 0 ? "Enabled" : "Nothing on"}
        </span>
        {entry.version && <span className="package-badge">v{entry.version}</span>}
        <span className="package-badge">{entry.kind}</span>
      </div>
      <p className="package-source">{entry.source}</p>
      {kinds.length > 0 && (
        <div className="package-chips">
          {kinds.map((kind) => {
            const on = entry[kind].filter((resource) => resource.enabled).length;
            return (
              <span className={`package-chip ${on === 0 ? "idle" : ""}`} key={kind}>
                <b>{on}</b>/{entry[kind].length} {KIND_SHORT_LABELS[kind]}
              </span>
            );
          })}
        </div>
      )}
      {!trusted && (
        <div className="package-error">
          Not enabled. This package is installed but was never confirmed here, so nothing it contains is loaded.
        </div>
      )}
      {entry.errors.map((message) => (
        <div className="package-error" key={message}>{message}</div>
      ))}
      {open && (
        total === 0
          ? <p className="package-source">This package declares no loadable resources.</p>
          : kinds.map((kind) => (
              <section className="resource-group" key={kind}>
                <h5>{KIND_LABELS[kind]}</h5>
                {entry[kind].map((resource) => (
                  <div className="resource-row" key={resource.path}>
                    <span className="resource-name">{resource.name}</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={resource.enabled}
                      aria-label={`${KIND_LABELS[kind]}: ${resource.name}`}
                      className={`toggle ${resource.enabled ? "on" : ""}`}
                      disabled={busy}
                      onClick={() => toggle(kind, resource.name, !resource.enabled)}
                    >
                      <span />
                    </button>
                  </div>
                ))}
              </section>
            ))
      )}
      <div className="package-card-foot">
        {open && total > 0 && <span className="package-foot-note">{active} of {total} resources loaded</span>}
        <div className="row-actions">
          {!trusted && <button type="button" className="primary-button" disabled={busy} onClick={onTrust}>Review and enable</button>}
          <button type="button" className="secondary-button" disabled={busy} onClick={onUpdate}>Update</button>
          <button type="button" className="danger-button" disabled={busy} onClick={onRemove}>Remove</button>
        </div>
      </div>
    </article>
  );
}
