import { useCallback, useEffect, useState } from "react";
import { PACKAGE_RESOURCE_KINDS, type PackageRecord, type PackageResourceKind, type PackageSearchResult } from "../types";
import { Icon, type IconName } from "./Icons";
import { PackageBrowser, type BrowsePage } from "./PackageBrowser";
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
  kind?: "subagents" | "auto_titles" | "web_fetch" | "mcp";
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
      "A read-only planning mode. Toggle Build/Plan in the composer or press ⇧Tab — the agent inspects the workspace and proposes a plan before changing anything. Click Plan again for Ultra Plan: the agent interviews you one question at a time, with no limit, before it plans. Expect more usage.",
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
    name: "Web Fetch",
    description:
      "Lets the agent read a web page by URL, returned as Markdown. Public addresses only: it can't reach localhost or your local network, and it doesn't search. The only built-in that contacts sites you didn't configure.",
    tools: ["web_fetch"],
    icon: "external",
    kind: "web_fetch"
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
  }
];

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
  subagentsEnabled?: boolean;
  /** On unless `web_fetch` is in the tool denylist. */
  webFetchEnabled?: boolean;
  autoTitlesEnabled?: boolean;
  autoTitlesConfigured?: boolean;
  onToggleSubagents?: (enabled: boolean) => Promise<void>;
  onToggleWebFetch?: (enabled: boolean) => Promise<void>;
  onToggleAutoTitles?: (enabled: boolean) => Promise<void>;
  onConfigureAutoTitles?: () => void;
  /** Opens Settings → Sub-agents. */
  onConfigureSubagents?: () => void;
  /** Opens Settings → MCP servers. */
  onConfigureMcp?: () => void;
}

type Tab = "installed" | "browse";

export function PackagesSection({
  packages, subagentsEnabled = false, webFetchEnabled = true, autoTitlesEnabled = false, autoTitlesConfigured = false,
  onToggleSubagents, onToggleWebFetch, onToggleAutoTitles, onConfigureSubagents, onConfigureAutoTitles, onConfigureMcp,
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

  return (
    <div className="settings-scroll">
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
        <PackageBrowser
          installed={new Set(packages.map((entry) => entry.source))}
          busy={busy}
          onSearch={searchPage}
          onInstall={(next) => setPendingTrust({ source: next, mode: "install" })}
        />
      ) : (
      <>
      <div className="section-heading-row">
        <div>
          <h3>Installed packages</h3>
          <p>
            Pi packages add tools, skills, and prompts. Only the resources switched on here are loaded,
            and nothing from a project&rsquo;s own <code>.pi</code> folder is ever run.
          </p>
        </div>
      </div>

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
        <button type="submit" className="primary-button" disabled={busy || !source.trim()}>
          <Icon name="plus" /> Add package
        </button>
      </form>
      <p className="package-add-hint">
        Accepts <code>npm:name</code>, <code>git:github.com/user/repo</code>, or an absolute path.
      </p>

      {error && !pendingTrust && <div className="error-banner">{error}</div>}

      {loading && packages.length === 0 ? (
        <div className="model-empty">Loading packages…</div>
      ) : packages.length === 0 ? (
        <div className="package-empty">
          <span className="package-empty-icon"><Icon name="archive" /></span>
          <h4>Nothing installed yet</h4>
          <p>
            A package can add tools, skills and prompts to every chat. Add one by name above,
            or look at what other people have published.
          </p>
          <button type="button" className="secondary-button" onClick={() => setTab("browse")}>
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

      <div className="section-heading-row builtin-heading">
        <div>
          <h3>Built-In</h3>
          <p>Compiled into WackCode — you don&rsquo;t need a package for these. Sub-agents, Web Fetch and Auto chat titles are optional, and MCP does nothing until you add a server.</p>
        </div>
      </div>
      <div className="package-grid">
        {BUILTIN_EXTENSIONS.map((extension) => (
          <BuiltinCard
            key={extension.name}
            extension={extension}
            enabled={extension.kind === "subagents" ? subagentsEnabled : extension.kind === "web_fetch" ? webFetchEnabled : extension.kind === "auto_titles" ? autoTitlesEnabled : true}
            busy={busy}
            onToggle={extension.kind === "subagents" && onToggleSubagents
              ? (enabled) => void run(() => onToggleSubagents(enabled)).catch(() => undefined)
              : extension.kind === "web_fetch" && onToggleWebFetch
              ? (enabled) => void run(() => onToggleWebFetch(enabled)).catch(() => undefined)
              : extension.kind === "auto_titles" && onToggleAutoTitles
              ? (enabled) => void run(() => onToggleAutoTitles(enabled)).catch(() => undefined)
              : undefined}
            onConfigure={extension.kind === "subagents" ? onConfigureSubagents : extension.kind === "auto_titles" ? onConfigureAutoTitles : extension.kind === "mcp" ? onConfigureMcp : undefined}
            toggleDisabled={extension.kind === "auto_titles" && !autoTitlesEnabled && (!autoTitlesConfigured || !subagentsEnabled)}
            subagentsOff={extension.kind === "auto_titles" && !subagentsEnabled}
          />
        ))}
      </div>
      </>
      )}

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
  /** Auto titles only: the Sub-agents built-in it rides is off, so setup and enabling are blocked. */
  subagentsOff?: boolean;
  /** Present only for a built-in with an off switch. */
  onToggle?: (enabled: boolean) => void;
  onConfigure?: () => void;
}

/** A built-in extension: same card shape as a package. Always-on ones show a disabled toggle;
 *  sub-agents and web fetch have a live one. Auto titles lives on the Sub-agents page, so its
 *  Set up is blocked while that built-in is off. MCP is always on and has its own page, where
 *  each server has a switch. */
function BuiltinCard({ extension, enabled, busy, toggleDisabled = false, subagentsOff = false, onToggle, onConfigure }: BuiltinCardProps) {
  const autoTitles = extension.kind === "auto_titles";
  return (
    <article className={`package-card builtin-card ${autoTitles ? "auto-title-package-card" : ""}`}>
      <div className="package-card-head">
        <span className={`package-icon kind-${extension.kind ?? "core"}`} aria-hidden="true">
          <Icon name={extension.icon} />
        </span>
        <span className="package-name builtin-name">{extension.name}</span>
        <button
          type="button"
          role="switch"
          aria-checked={enabled}
          aria-label={extension.name}
          className={`toggle ${enabled ? "on" : ""}`}
          disabled={!onToggle || busy || toggleDisabled}
          title={toggleDisabled ? (subagentsOff ? "Switch on Sub-agents to use auto titles" : "Choose a title model in Set up first") : undefined}
          onClick={() => onToggle?.(!enabled)}
        >
          <span />
        </button>
      </div>
      <div className="package-card-status">
        <span className={`package-state ${enabled ? "on" : "off"}`}>
          {(autoTitles || onToggle) ? (enabled ? "On" : "Off") : "Always on"}
        </span>
        {!autoTitles && onConfigure && ((onToggle && enabled) || extension.kind === "mcp") && (
          <button type="button" className="ghost-button builtin-configure" onClick={onConfigure}>
            Configure <Icon name="chevron" />
          </button>
        )}
        {autoTitles && (
          <button
            type="button"
            className="ghost-button builtin-configure"
            disabled={subagentsOff}
            title={subagentsOff ? "Switch on Sub-agents to set up auto titles" : undefined}
            onClick={onConfigure}
          >
            {toggleDisabled ? "Set up" : "Configure"} <Icon name="chevron" />
          </button>
        )}
      </div>
      <p className="builtin-desc">{extension.description}</p>
      {extension.tools.length > 0 && <section className="resource-group builtin-resources">
        <h5>Tools</h5>
        {extension.tools.map((tool) => (
          <div className="resource-row" key={tool}>
            <span className="resource-name">{tool}</span>
            {onToggle ? (
              // Switchable built-ins (Sub-agents, Web Fetch) carry their switch here as a mirror:
              // the card's toggle is the control, so this one stays disabled.
              <button
                type="button"
                role="switch"
                aria-checked={enabled}
                aria-label={tool}
                className={`toggle ${enabled ? "on" : ""}`}
                disabled
              >
                <span />
              </button>
            ) : (
              <span className="resource-lock" title="Ships with WackCode — always available">
                <Icon name="check" />
              </span>
            )}
          </div>
        ))}
      </section>}
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
