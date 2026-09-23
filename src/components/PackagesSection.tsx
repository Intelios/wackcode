import { useEffect, useState } from "react";
import { PACKAGE_RESOURCE_KINDS, type PackageRecord, type PackageResourceKind, type PackageSearchResult } from "../types";
import { Icon } from "./Icons";
import { PackageBrowser } from "./PackageBrowser";
import { TrustDialog } from "./TrustDialog";
import { ConfirmDialog } from "./ui/ConfirmDialog";

const KIND_LABELS: Record<PackageResourceKind, string> = {
  extensions: "Extensions",
  skills: "Skills",
  prompts: "Prompts",
  themes: "Themes"
};

interface BuiltinExtension {
  name: string;
  description: string;
  tools: readonly string[];
  /** Has its own off switch (sub-agents). Everything else is always on. */
  toggleable?: boolean;
}

/**
 * The extensions compiled into WackCode itself (worker/src/builtin/index.ts). They are shown
 * here so nobody installs a package duplicating something that already ships with the app.
 * They have no trust gate, and only sub-agents has an off switch — keep this list in sync
 * with the worker.
 */
const BUILTIN_EXTENSIONS: readonly BuiltinExtension[] = [
  {
    name: "Plan Mode",
    description:
      "A read-only planning mode. Toggle Build/Plan in the composer or press ⇧Tab — the agent inspects the workspace and proposes a plan before changing anything.",
    tools: ["plan_mode_complete"]
  },
  {
    name: "Ask User Questions",
    description:
      "Lets the agent ask structured multiple-choice questions in a native dialog instead of guessing. Available in every mode.",
    tools: ["ask_user_question"]
  },
  {
    name: "Todo List",
    description:
      "A live task list the agent keeps up to date while it works, shown above the composer. Rebuilt from the conversation, so it survives restarts and compaction.",
    tools: ["todo"]
  },
  {
    name: "Sub-agents",
    description:
      "Lets the agent hand self-contained tasks to sub-agents with their own context window, one at a time or several in parallel. Off by default: every sub-agent is extra model usage.",
    tools: ["subagent"],
    toggleable: true
  }
];

export interface PackageActions {
  /** Re-reads the shared package store. The cached list can be stale if `pi` was used elsewhere. */
  onRefresh: () => Promise<void>;
  onInstall: (source: string) => Promise<void>;
  /** Grant trust to a package already in the store that was never confirmed here. */
  onTrust: (source: string) => Promise<void>;
  onSearch: (query: string) => Promise<PackageSearchResult[]>;
  onRemove: (source: string) => Promise<void>;
  onUpdate: (source: string) => Promise<void>;
  onSetResources: (source: string, kind: PackageResourceKind, enabled: string[]) => Promise<void>;
}

interface Props extends PackageActions {
  packages: PackageRecord[];
  /** The one built-in with an off switch. */
  subagentsEnabled?: boolean;
  onToggleSubagents?: (enabled: boolean) => Promise<void>;
  /** Opens Settings → Sub-agents. */
  onConfigureSubagents?: () => void;
}

type Tab = "installed" | "browse";

export function PackagesSection({
  packages, subagentsEnabled = false, onToggleSubagents, onConfigureSubagents,
  onRefresh, onInstall, onTrust, onSearch, onRemove, onUpdate, onSetResources
}: Props) {
  const [tab, setTab] = useState<Tab>("installed");
  const [source, setSource] = useState("");
  const [pendingTrust, setPendingTrust] = useState<{ source: string; mode: "install" | "enable" }>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [removing, setRemoving] = useState<PackageRecord>();
  const [loading, setLoading] = useState(true);

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
          onSearch={onSearch}
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
        <div className="model-empty">No packages installed yet.</div>
      ) : (
        packages.map((entry) => (
          <PackageCard
            key={entry.source}
            entry={entry}
            busy={busy}
            onTrust={() => setPendingTrust({ source: entry.source, mode: "enable" })}
            onRemove={() => setRemoving(entry)}
            onUpdate={() => void run(() => onUpdate(entry.source)).catch(() => undefined)}
            onToggle={(kind, enabled) => void run(() => onSetResources(entry.source, kind, enabled)).catch(() => undefined)}
          />
        ))
      )}

      <div className="section-heading-row builtin-heading">
        <div>
          <h3>Built-In</h3>
          <p>Compiled into WackCode — you don&rsquo;t need a package for these. All are always on except Sub-agents.</p>
        </div>
      </div>
      {BUILTIN_EXTENSIONS.map((extension) => (
        <BuiltinCard
          key={extension.name}
          extension={extension}
          enabled={extension.toggleable ? subagentsEnabled : true}
          busy={busy}
          onToggle={extension.toggleable && onToggleSubagents
            ? (enabled) => void run(() => onToggleSubagents(enabled)).catch(() => undefined)
            : undefined}
          onConfigure={extension.toggleable ? onConfigureSubagents : undefined}
        />
      ))}
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
  /** Present only for a built-in with an off switch. */
  onToggle?: (enabled: boolean) => void;
  onConfigure?: () => void;
}

/** A built-in extension: same card shape as a package. Always-on ones show a disabled toggle
 *  pinned on; sub-agents has a live one, and a way to its settings page while on. */
function BuiltinCard({ extension, enabled, busy, onToggle, onConfigure }: BuiltinCardProps) {
  return (
    <article className="package-card builtin-card">
      <div className="package-card-head">
        <span className="package-name builtin-name">{extension.name}</span>
        {onToggle && enabled && onConfigure && (
          <button type="button" className="ghost-button builtin-configure" onClick={onConfigure}>
            Configure <Icon name="chevron" />
          </button>
        )}
        <span className="package-meta">{onToggle ? (enabled ? "On" : "Off") : "Always on"}</span>
        <button
          type="button"
          role="switch"
          aria-checked={enabled}
          aria-label={extension.name}
          className={`toggle ${enabled ? "on" : ""}`}
          disabled={!onToggle || busy}
          onClick={() => onToggle?.(!enabled)}
        >
          <span />
        </button>
      </div>
      <p className="builtin-desc">{extension.description}</p>
      <section className="resource-group builtin-resources">
        <h5>Tools</h5>
        {extension.tools.map((tool) => (
          <div className="resource-row" key={tool}>
            <span className="resource-name">{tool}</span>
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
          </div>
        ))}
      </section>
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
        <button
          type="button"
          className={`package-disclosure ${open ? "open" : ""}`}
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
        >
          <Icon name="chevron" />
          <span className="package-name">{entry.displayName}</span>
        </button>
        <span className="package-meta">
          {entry.version ? `v${entry.version}` : entry.kind}
          {total > 0 && ` · ${active}/${total} resources`}
        </span>
        <div className="row-actions">
          {!trusted && <button type="button" className="primary-button" disabled={busy} onClick={onTrust}>Review and enable</button>}
          <button type="button" className="secondary-button" disabled={busy} onClick={onUpdate}>Update</button>
          <button type="button" className="danger-button" disabled={busy} onClick={onRemove}>Remove</button>
        </div>
      </div>
      <p className="package-source">{entry.source}</p>
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
    </article>
  );
}
