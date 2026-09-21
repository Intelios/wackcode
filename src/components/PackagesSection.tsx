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

/**
 * The extensions compiled into WackCode itself (worker/src/builtin/index.ts). They are shown
 * here so nobody installs a package duplicating something that already ships with the app.
 * They have no trust gate and no off switch — keep this list in sync with the worker.
 */
const BUILTIN_EXTENSIONS = [
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
  }
] as const;

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
}

type Tab = "installed" | "browse";

export function PackagesSection({ packages, onRefresh, onInstall, onTrust, onSearch, onRemove, onUpdate, onSetResources }: Props) {
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
          <p>Compiled into WackCode and always on — you don&rsquo;t need a package for these.</p>
        </div>
      </div>
      {BUILTIN_EXTENSIONS.map((extension) => (
        <BuiltinCard key={extension.name} extension={extension} />
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

/** An always-on built-in extension: same card shape as a package, but no actions and a
 *  disabled toggle pinned on. */
function BuiltinCard({ extension }: { extension: (typeof BUILTIN_EXTENSIONS)[number] }) {
  return (
    <article className="package-card builtin-card">
      <div className="package-card-head">
        <span className="package-name builtin-name">{extension.name}</span>
        <span className="package-meta">Always on</span>
        <button
          type="button"
          role="switch"
          aria-checked="true"
          aria-label={extension.name}
          className="toggle on"
          disabled
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
              aria-checked="true"
              aria-label={tool}
              className="toggle on"
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
