import { useCallback, useEffect, useMemo, useState } from "react";
import type {
  PackageRecord, PackageSearchResult, SaveSkillInput, SkillDocument, SkillEntry, SkillFolderView, SkillPackageView,
  SkillSearchPage, SkillSearchSort, SkillsChange, SkillsOverview
} from "../types";
import { Icon } from "./Icons";
import { PackageBrowser, type BrowsePage } from "./PackageBrowser";
import { TrustDialog } from "./TrustDialog";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { MenuButton } from "./ui/MenuButton";
import { Select } from "./ui/Select";

/** What Settings › Skills can do; SettingsPage wires each one to `api`. Keep them stable. */
export interface SkillActions {
  onList: () => Promise<SkillsOverview>;
  onRead: (path: string) => Promise<SkillDocument>;
  onSave: (input: SaveSkillInput) => Promise<SkillsChange>;
  onDelete: (path: string) => Promise<SkillsChange>;
  onSetEnabled: (path: string, enabled: boolean) => Promise<SkillsChange>;
  onSetFolderEnabled: (id: string, enabled: boolean) => Promise<SkillsChange>;
  /** Opens a native folder panel; null when cancelled. */
  onAddFolder: () => Promise<SkillsChange | null>;
  onRemoveFolder: (id: string) => Promise<SkillsChange>;
  /** Opens a native panel; null when cancelled. */
  onImport: (kind: "folder" | "file") => Promise<SkillsChange | null>;
  onCopyToLibrary: (path: string) => Promise<SkillsChange>;
  onReveal: (path: string) => Promise<void>;
  onSearch: (query: string, sort: SkillSearchSort, page: number) => Promise<SkillSearchPage>;
  onDetails: (name: string) => Promise<PackageSearchResult>;
}

interface Props extends SkillActions {
  packages: PackageRecord[];
  /** Installs a package with only its skills switched on. */
  onInstallSkills: (source: string) => Promise<void>;
  /** A package skill's switch is its package resource's: the resource names left on. */
  onSetPackageSkills: (source: string, enabled: string[]) => Promise<void>;
  onOpenPackages: () => void;
}

interface SkillDraft {
  name: string;
  description: string;
  manual: boolean;
  body: string;
}

type Tab = "yours" | "browse";
type View =
  | { kind: "list" }
  | { kind: "detail"; skill: SkillEntry; origin: string }
  | { kind: "edit"; draft: SkillDraft; path?: string; original?: string };

// The Agent Skills rules (agentskills.io/specification); Rust checks them again on save.
const NAME_PATTERN = /^[a-z0-9]+(-[a-z0-9]+)*$/;
const MAX_NAME_CHARS = 64;
const MAX_DESCRIPTION_CHARS = 1_024;

const WHEN_OPTIONS = [
  { value: "auto", label: "The agent decides", hint: "Listed to the model by its description. /skill:name works too." },
  { value: "manual", label: "Only when I type /skill:name", hint: "Kept out of the model's list." }
];

const SORT_OPTIONS = [
  { value: "downloads", label: "Most downloads" },
  { value: "recent", label: "Recently published" },
  { value: "name", label: "A–Z" }
];

const EMPTY_DRAFT: SkillDraft = { name: "", description: "", manual: false, body: "" };

/** Why a draft can't be saved yet, or nothing when it can. */
export function skillDraftIssue(draft: SkillDraft, taken: ReadonlySet<string>, original?: string): string | undefined {
  const name = draft.name.trim();
  if (!name) return "Give the skill a name.";
  if (name.length > MAX_NAME_CHARS) return `A skill name can be at most ${MAX_NAME_CHARS} characters.`;
  if (!NAME_PATTERN.test(name)) return "Use lowercase letters, numbers and single hyphens, like pdf-tools.";
  if (name !== original && taken.has(name)) return `A skill named ${name} already exists in Your skills.`;
  const description = draft.description.trim();
  if (!description) return "Describe what the skill does and when to use it.";
  if (description.length > MAX_DESCRIPTION_CHARS) return `A description can be at most ${MAX_DESCRIPTION_CHARS} characters.`;
  return undefined;
}

function matches(skill: SkillEntry, filter: string): boolean {
  if (!filter) return true;
  const needle = filter.toLowerCase();
  return skill.name.toLowerCase().includes(needle) || skill.description.toLowerCase().includes(needle);
}

function shadowNote(skill: SkillEntry): string | undefined {
  return skill.shadowedBy ? `Not loaded: a skill with the same name in ${skill.shadowedBy} loads instead.` : undefined;
}

/**
 * Settings › Skills. The user's own skills live in `~/.agents/skills`, the folder other agent
 * tools read too; other tools' folders load only when switched on; package skills are listed
 * with their package's switches. Browse is pi.dev's skill filter. Every change reaches running
 * chats on their next message.
 */
export function SkillsSection({
  onList, onRead, onSave, onDelete, onSetEnabled, onSetFolderEnabled, onAddFolder, onRemoveFolder, onImport,
  onCopyToLibrary, onReveal, onSearch, onDetails, packages, onInstallSkills, onSetPackageSkills, onOpenPackages
}: Props) {
  const [tab, setTab] = useState<Tab>("yours");
  const [overview, setOverview] = useState<SkillsOverview>();
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [note, setNote] = useState<string>();
  const [view, setView] = useState<View>({ kind: "list" });
  const [filter, setFilter] = useState("");
  const [openGroups, setOpenGroups] = useState<ReadonlySet<string>>(new Set());
  const [deleting, setDeleting] = useState<SkillEntry>();
  const [removingFolder, setRemovingFolder] = useState<SkillFolderView>();
  const [sort, setSort] = useState<SkillSearchSort>("downloads");
  const [pendingInstall, setPendingInstall] = useState<{ source: string; declares?: string[] | "unknown" }>();
  const [installError, setInstallError] = useState<string>();

  useEffect(() => {
    let active = true;
    void onList()
      .then((next) => { if (active) setOverview(next); })
      .catch((reason: unknown) => { if (active) setError(String(reason)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [onList]);

  async function apply(action: () => Promise<SkillsChange | null | void>, after?: () => void): Promise<void> {
    setBusy(true);
    setError(undefined);
    setNote(undefined);
    try {
      const change = await action();
      if (change) {
        setOverview(change.overview);
        setNote(change.note);
      }
      after?.();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const library = overview?.folders.find((folder) => folder.kind === "library");
  const others = overview?.folders.filter((folder) => folder.kind !== "library") ?? [];
  const packageGroups = overview?.packages.filter((entry) => entry.skills.length > 0 || entry.diagnostics.length > 0) ?? [];
  const taken = useMemo(() => new Set(library?.skills.map((skill) => skill.name) ?? []), [library]);

  const toggleGroup = (id: string) => setOpenGroups((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  });

  function togglePackageSkill(group: SkillPackageView, skill: SkillEntry, next: boolean): void {
    const record = packages.find((entry) => entry.source === group.source);
    if (!record || !skill.resourceName) return;
    const enabled = record.skills
      .filter((resource) => (resource.name === skill.resourceName ? next : resource.enabled))
      .map((resource) => resource.name);
    void apply(async () => {
      await onSetPackageSkills(group.source, enabled);
      setOverview(await onList());
    });
  }

  const searchSkills = useCallback(async (query: string, page: number): Promise<BrowsePage> => {
    const found = await onSearch(query, sort, page);
    return {
      results: found.results,
      hasMore: found.hasMore,
      notice: found.source === "npm" ? "pi.dev couldn't be read, so these are npm keyword matches, which are less precise." : undefined
    };
  }, [onSearch, sort]);

  function startInstall(source: string): void {
    setInstallError(undefined);
    setPendingInstall({ source });
    void onDetails(source.replace(/^npm:/, ""))
      .then((details) => setPendingInstall((current) => current?.source === source
        ? { source, declares: details.declares.length ? details.declares : "unknown" }
        : current))
      .catch(() => setPendingInstall((current) => current?.source === source ? { source, declares: "unknown" } : current));
  }

  async function confirmInstall(): Promise<void> {
    if (!pendingInstall) return;
    setBusy(true);
    setInstallError(undefined);
    try {
      await onInstallSkills(pendingInstall.source);
      setOverview(await onList());
      setNote(`Installed ${pendingInstall.source.replace(/^npm:/, "")}. Its skills are listed under From packages.`);
      setPendingInstall(undefined);
    } catch (reason) {
      // The dialog stays open so the failure can be read and retried.
      setInstallError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  function renderRow(skill: SkillEntry, origin: string, toggle?: { onToggle: (next: boolean) => void; disabled?: boolean; title?: string }) {
    const shadow = shadowNote(skill);
    return (
      <div className={`skill-row ${skill.enabled && !skill.shadowedBy ? "" : "off"}`} key={skill.filePath}>
        <button type="button" className="skill-row-main" onClick={() => setView({ kind: "detail", skill, origin })}>
          <span className="skill-row-head">
            <span className="skill-name">{skill.name}</span>
            {skill.manual && <span className="subagent-badge" title="Kept out of the model's list: runs only when you type /skill:name">/skill only</span>}
            {skill.shadowedBy && <span className="subagent-badge skill-shadowed" title={shadow}>Not loaded</span>}
          </span>
          <span className="skill-description">{skill.description}</span>
        </button>
        {toggle && (
          <button
            type="button"
            role="switch"
            aria-checked={skill.enabled}
            aria-label={`Use ${skill.name}`}
            className={`toggle ${skill.enabled ? "on" : ""}`}
            disabled={busy || toggle.disabled}
            title={toggle.title}
            onClick={() => toggle.onToggle(!skill.enabled)}
          >
            <span />
          </button>
        )}
      </div>
    );
  }

  function renderDiagnostics(group: { diagnostics: SkillFolderView["diagnostics"] }, root?: string) {
    if (group.diagnostics.length === 0) return null;
    return (
      <ul className="skill-diagnostics">
        {group.diagnostics.map((diagnostic, index) => {
          const where = diagnostic.path && root && diagnostic.path.startsWith(`${root}/`) ? diagnostic.path.slice(root.length + 1) : diagnostic.path;
          return <li key={`${diagnostic.path}:${index}`}>{where ? <code>{where}</code> : null}{where ? ": " : null}{diagnostic.message}</li>;
        })}
      </ul>
    );
  }

  function renderFolder(folder: SkillFolderView) {
    const open = openGroups.has(folder.id);
    const shown = folder.skills.filter((skill) => matches(skill, filter));
    const count = folder.exists ? `${folder.skills.length} ${folder.skills.length === 1 ? "skill" : "skills"}` : "Not found";
    return (
      <article className={`subagent-setting skill-folder ${open ? "open" : ""} ${folder.enabled ? "" : "off"}`} key={folder.id}>
        <div className="subagent-setting-head">
          <button type="button" className={`package-disclosure ${open ? "open" : ""}`} aria-expanded={open} onClick={() => toggleGroup(folder.id)}>
            <Icon name="chevron" />
            <span className="subagent-setting-name skill-folder-name">{folder.label}</span>
            <span className="subagent-badge">{count}</span>
            {folder.kind === "custom" && <span className="subagent-badge">Added</span>}
          </button>
          <span className="subagent-model-summary mcp-summary" title={folder.path}>{folder.displayPath}</span>
          <button
            type="button"
            role="switch"
            aria-checked={folder.enabled}
            aria-label={`Load skills from ${folder.label}`}
            className={`toggle ${folder.enabled ? "on" : ""}`}
            disabled={busy || (!folder.exists && !folder.enabled)}
            title={!folder.exists ? "This folder isn't on this Mac" : undefined}
            onClick={() => void apply(() => onSetFolderEnabled(folder.id, !folder.enabled))}
          >
            <span />
          </button>
        </div>
        {open && (
          <div className="subagent-setting-body">
            {!folder.exists ? (
              <small className="subagent-hint">Nothing is at {folder.displayPath}. It appears here once that tool has skills there.</small>
            ) : shown.length === 0 ? (
              <small className="subagent-hint">{filter ? "No skills here match the filter." : "No skills in this folder."}</small>
            ) : (
              <div className="skill-list nested">
                {shown.map((skill) => renderRow(skill, folder.label, {
                  onToggle: (next) => void apply(() => onSetEnabled(skill.filePath, next)),
                  disabled: !folder.enabled,
                  title: folder.enabled ? undefined : "Switch the folder on first"
                }))}
              </div>
            )}
            {renderDiagnostics(folder, folder.path)}
            <div className="subagent-editor-actions">
              {folder.kind === "custom" && (
                <button type="button" className="danger-button" disabled={busy} onClick={() => setRemovingFolder(folder)}>Remove</button>
              )}
              <span className="subagent-editor-spacer" />
              {folder.exists && (
                <button type="button" className="secondary-button" disabled={busy} onClick={() => void apply(() => onReveal(folder.path))}>
                  <Icon name="folder" /> Show in Finder
                </button>
              )}
            </div>
          </div>
        )}
      </article>
    );
  }

  function renderPackage(group: SkillPackageView) {
    const open = openGroups.has(`package:${group.source}`);
    const on = group.skills.filter((skill) => skill.enabled).length;
    const shown = group.skills.filter((skill) => matches(skill, filter));
    return (
      <article className={`subagent-setting skill-folder ${open ? "open" : ""} ${on === 0 ? "off" : ""}`} key={group.source}>
        <div className="subagent-setting-head">
          <button type="button" className={`package-disclosure ${open ? "open" : ""}`} aria-expanded={open} onClick={() => toggleGroup(`package:${group.source}`)}>
            <Icon name="chevron" />
            <span className="subagent-setting-name">{group.label}</span>
            <span className="subagent-badge">{on}/{group.skills.length} on</span>
          </button>
          <span className="subagent-model-summary mcp-summary" title={group.source}>{group.source}</span>
        </div>
        {open && (
          <div className="subagent-setting-body">
            {shown.length === 0
              ? <small className="subagent-hint">{filter ? "No skills here match the filter." : "This package's skills could not be read."}</small>
              : <div className="skill-list nested">
                  {shown.map((skill) => renderRow(skill, group.label, { onToggle: (next) => togglePackageSkill(group, skill, next) }))}
                </div>}
            {renderDiagnostics(group)}
          </div>
        )}
      </article>
    );
  }

  function renderList() {
    if (loading && !overview) return <div className="model-empty">Reading your skill folders…</div>;
    if (!overview || !library) return null;
    const mine = library.skills.filter((skill) => matches(skill, filter));
    const total = overview.folders.reduce((sum, folder) => sum + folder.skills.length, 0) + packageGroups.reduce((sum, group) => sum + group.skills.length, 0);
    return (
      <>
        {total > 6 && (
          <input
            className="package-search skill-filter"
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            placeholder="Filter skills…"
            aria-label="Filter skills"
            spellCheck={false}
          />
        )}
        <section className="subagent-group" aria-label="Your skills">
          <div className="skill-group-head">
            <h4>Your skills</h4>
            <span className="skill-group-path" title={library.path}>{library.displayPath} · shared with Codex, OpenCode and the Pi CLI</span>
            {library.exists && (
              <button type="button" className="ghost-button skill-reveal" disabled={busy} onClick={() => void apply(() => onReveal(library.path))}>
                Show in Finder
              </button>
            )}
          </div>
          {library.skills.length === 0 ? (
            <div className="package-empty">
              <span className="package-empty-icon"><Icon name="book" /></span>
              <h4>No skills yet</h4>
              <p>
                A skill is a set of instructions, with any scripts or references it needs, that the agent picks up when a task
                matches its description. Write one, or import skills you already have.
              </p>
              <button type="button" className="secondary-button" disabled={busy} onClick={() => setView({ kind: "edit", draft: EMPTY_DRAFT })}>
                <Icon name="plus" /> New skill
              </button>
            </div>
          ) : mine.length === 0 ? (
            <small className="subagent-hint">None of your skills match the filter.</small>
          ) : (
            <div className="skill-list">
              {mine.map((skill) => renderRow(skill, library.label, { onToggle: (next) => void apply(() => onSetEnabled(skill.filePath, next)) }))}
            </div>
          )}
          {renderDiagnostics(library, library.path)}
        </section>

        <div className="section-heading-row skill-subheading">
          <div>
            <h3>Other tools&rsquo; folders</h3>
            <p>Skills you already use with other agents. A folder loads only once you switch it on, and nothing here is changed.</p>
          </div>
          <div className="row-actions">
            <button type="button" className="secondary-button" disabled={busy} onClick={() => void apply(onAddFolder)}>
              <Icon name="plus" /> Add folder…
            </button>
          </div>
        </div>
        <section className="subagent-group" aria-label="Other tools' folders">{others.map(renderFolder)}</section>

        {packageGroups.length > 0 && (
          <>
            <div className="section-heading-row skill-subheading">
              <div>
                <h3>From packages</h3>
                <p>Skills that installed Pi packages add. Their switches are the package&rsquo;s own, and changing one restarts idle chats.</p>
              </div>
              <div className="row-actions">
                <button type="button" className="ghost-button builtin-configure" onClick={onOpenPackages}>
                  Manage in Packages <Icon name="chevron" />
                </button>
              </div>
            </div>
            <section className="subagent-group" aria-label="Skills from packages">{packageGroups.map(renderPackage)}</section>
          </>
        )}
      </>
    );
  }

  function openEditor(skill?: SkillEntry, body = ""): void {
    setError(undefined);
    setNote(undefined);
    setView(skill
      ? { kind: "edit", path: skill.filePath, original: skill.name, draft: { name: skill.name, description: skill.description, manual: skill.manual, body } }
      : { kind: "edit", draft: EMPTY_DRAFT });
  }

  const listing = view.kind === "list";

  return (
    <div className="settings-scroll subagents-settings skills-settings">
      <div className="package-tabs" role="tablist">
        {(["yours", "browse"] as Tab[]).map((id) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={tab === id}
            className={`package-tab ${tab === id ? "active" : ""}`}
            onClick={() => { setTab(id); setView({ kind: "list" }); }}
          >
            {id === "yours" ? `Your skills${library?.skills.length ? ` (${library.skills.length})` : ""}` : "Browse"}
          </button>
        ))}
      </div>

      {tab === "browse" ? (
        <>
          {note && <div className="package-notice" role="status">{note}</div>}
          <PackageBrowser
            installed={new Set(packages.map((entry) => entry.source))}
            busy={busy}
            onSearch={searchSkills}
            onInstall={startInstall}
            heading="Browse skills"
            blurb="Skill packages from pi.dev's catalogue. Installing one switches on only its skills. Searching contacts pi.dev."
            placeholder="Search skills…"
            toolbar={
              <Select
                className="settings-select"
                value={sort}
                options={SORT_OPTIONS}
                onChange={(value) => setSort(value as SkillSearchSort)}
                aria-label="Sort skills"
              />
            }
          />
        </>
      ) : (
        <>
          {listing && (
            <div className="section-heading-row">
              <div>
                <h3>Skills</h3>
                <p>
                  Instructions the agent loads when a task matches. Only each skill&rsquo;s name and description stay in context, and the
                  agent reads the rest when it needs it. Use one directly with <code>/skill:name</code>. Changes apply to running chats
                  from their next message.
                </p>
              </div>
              <div className="row-actions">
                <button type="button" className="secondary-button" disabled={busy || !overview} onClick={() => openEditor()}>
                  <Icon name="plus" /> New skill
                </button>
                <MenuButton
                  label="Import skills"
                  className="skill-import-button"
                  icon={<><Icon name="archive" /><span>Import</span></>}
                  items={[
                    { label: "A folder of skills…", hint: "One skill's folder, or a folder holding several", onSelect: () => void apply(() => onImport("folder")) },
                    { label: "A single .md file…", hint: "Copied in as its own skill", onSelect: () => void apply(() => onImport("file")) }
                  ]}
                />
              </div>
            </div>
          )}
          {error && <div className="error-banner subagents-error" role="alert">{error}</div>}
          {note && <div className="package-notice" role="status">{note}</div>}
          {view.kind === "list" && renderList()}
          {view.kind === "detail" && (
            <SkillDetail
              skill={view.skill}
              origin={view.origin}
              busy={busy}
              onRead={onRead}
              onBack={() => setView({ kind: "list" })}
              onEdit={(body) => openEditor(view.skill, body)}
              onDelete={() => setDeleting(view.skill)}
              onCopy={() => void apply(() => onCopyToLibrary(view.skill.filePath), () => {
                setView({ kind: "list" });
                setNote(`Copied ${view.skill.name} to Your skills. Your copy loads instead of the original, so edit it freely.`);
              })}
              onReveal={() => void apply(() => onReveal(view.skill.baseDir))}
            />
          )}
          {view.kind === "edit" && (
            <SkillEditor
              draft={view.draft}
              isNew={!view.path}
              issue={skillDraftIssue(view.draft, taken, view.original)}
              busy={busy}
              onChange={(draft) => setView({ ...view, draft })}
              onCancel={() => { setView({ kind: "list" }); setError(undefined); }}
              onSave={() => void apply(
                () => onSave({ path: view.path, name: view.draft.name.trim(), description: view.draft.description.trim(), manual: view.draft.manual, body: view.draft.body }),
                () => setView({ kind: "list" })
              )}
            />
          )}
        </>
      )}

      {pendingInstall && (
        <TrustDialog
          source={pendingInstall.source}
          mode="install"
          skillsOnly
          declares={pendingInstall.declares}
          busy={busy}
          error={installError}
          onCancel={() => { setPendingInstall(undefined); setInstallError(undefined); }}
          onConfirm={() => void confirmInstall()}
        />
      )}
      {deleting && (
        <ConfirmDialog
          title={`Delete ${deleting.name}?`}
          body="Its folder moves to the Trash, so Codex, OpenCode and the Pi CLI stop seeing it too. You can put it back from the Trash."
          confirmLabel="Move to Trash"
          danger
          onConfirm={async () => {
            const change = await onDelete(deleting.filePath);
            setOverview(change.overview);
            setView({ kind: "list" });
            setDeleting(undefined);
          }}
          onCancel={() => setDeleting(undefined)}
        />
      )}
      {removingFolder && (
        <ConfirmDialog
          title={`Stop using ${removingFolder.label}?`}
          body="Its skills stop loading. The folder and its files are left as they are."
          confirmLabel="Remove"
          danger
          onConfirm={async () => {
            const change = await onRemoveFolder(removingFolder.id);
            setOverview(change.overview);
            setRemovingFolder(undefined);
          }}
          onCancel={() => setRemovingFolder(undefined)}
        />
      )}
    </div>
  );
}

interface DetailProps {
  skill: SkillEntry;
  origin: string;
  busy: boolean;
  onRead: (path: string) => Promise<SkillDocument>;
  onBack: () => void;
  onEdit: (body: string) => void;
  onDelete: () => void;
  onCopy: () => void;
  onReveal: () => void;
}

/** One skill, read-only: its instructions and files, with what can be done to it. */
function SkillDetail({ skill, origin, busy, onRead, onBack, onEdit, onDelete, onCopy, onReveal }: DetailProps) {
  const [contents, setContents] = useState<SkillDocument>();
  const [error, setError] = useState<string>();

  useEffect(() => {
    let active = true;
    void onRead(skill.filePath)
      .then((next) => { if (active) setContents(next); })
      .catch((reason: unknown) => { if (active) setError(String(reason)); });
    return () => { active = false; };
  }, [onRead, skill.filePath]);

  const shadow = shadowNote(skill);
  return (
    <article className="subagent-setting skill-detail open">
      <button type="button" className="ghost-button skill-back" onClick={onBack}>
        <Icon name="back" /> All skills
      </button>
      <div className="skill-detail-head">
        <span className="subagent-setting-name">{skill.name}</span>
        <span className="subagent-badge">{origin}</span>
        {skill.manual && <span className="subagent-badge">/skill only</span>}
        {!skill.enabled && <span className="subagent-badge">Off</span>}
      </div>
      <p className="skill-detail-description">{skill.description}</p>
      {shadow && <p className="subagent-issue skill-detail-note">{shadow}</p>}
      <div className="subagent-prompt-field">
        <span className="subagent-field-label">Instructions</span>
        {error ? (
          <div className="error-banner">{error}</div>
        ) : !contents ? (
          <small className="subagent-hint">Reading…</small>
        ) : contents.body ? (
          <pre className="subagent-prompt-view skill-body-view">{contents.body}</pre>
        ) : (
          <small className="subagent-hint">This skill has no instructions beyond its description.</small>
        )}
      </div>
      {contents && contents.files.length > 0 && (
        <div className="subagent-tools-field">
          <span className="subagent-field-label">Files the skill can use</span>
          <ul className="skill-files">
            {contents.files.map((file) => <li key={file}>{file}</li>)}
            {contents.filesTruncated && <li className="subagent-hint">…and more</li>}
          </ul>
        </div>
      )}
      <p className="skill-path" title={skill.filePath}>{skill.filePath}</p>
      <div className="subagent-editor-actions">
        {skill.editable && <button type="button" className="danger-button" disabled={busy} onClick={onDelete}>Delete</button>}
        <span className="subagent-editor-spacer" />
        <button type="button" className="secondary-button" disabled={busy} onClick={onReveal}><Icon name="folder" /> Show in Finder</button>
        {skill.editable ? (
          <button type="button" className="secondary-button" disabled={busy || !contents} onClick={() => onEdit(contents?.body ?? "")}>
            <Icon name="pencil" /> Edit
          </button>
        ) : (
          <button type="button" className="secondary-button" disabled={busy} onClick={onCopy}>
            <Icon name="copy" /> Copy to Your skills
          </button>
        )}
      </div>
    </article>
  );
}

interface EditorProps {
  draft: SkillDraft;
  isNew: boolean;
  issue?: string;
  busy: boolean;
  onChange: (draft: SkillDraft) => void;
  onCancel: () => void;
  onSave: () => void;
}

/** Name, description, when it's used, and instructions. The folder follows the name. */
function SkillEditor({ draft, isNew, issue, busy, onChange, onCancel, onSave }: EditorProps) {
  const descriptionLength = draft.description.trim().length;
  // Only flag the name once there is one: an empty new form isn't an error yet.
  const shownIssue = draft.name || draft.description ? issue : undefined;
  return (
    <article className="subagent-setting skill-editor open new">
      <div className="subagent-editor">
        <div className="form-grid">
          <label>
            <span>Name <small>Lowercase letters, numbers and hyphens. Also the folder&rsquo;s name.</small></span>
            <input
              value={draft.name}
              onChange={(event) => onChange({ ...draft, name: event.target.value.toLowerCase().replace(/\s+/g, "-") })}
              placeholder="pdf-tools"
              spellCheck={false}
              maxLength={MAX_NAME_CHARS}
            />
          </label>
          <label>
            <span>When it&rsquo;s used</span>
            <Select
              className="settings-select"
              matchWidth
              value={draft.manual ? "manual" : "auto"}
              options={WHEN_OPTIONS}
              disabled={busy}
              onChange={(value) => onChange({ ...draft, manual: value === "manual" })}
              aria-label="When the skill is used"
            />
          </label>
          <label className="wide-field">
            <span>
              Description{" "}
              <small className={descriptionLength > MAX_DESCRIPTION_CHARS ? "mcp-invalid" : undefined}>
                What it does and when to use it: the agent decides from this alone. {descriptionLength}/{MAX_DESCRIPTION_CHARS}
              </small>
            </span>
            <textarea
              className="subagent-prompt-input skill-description-input"
              value={draft.description}
              onChange={(event) => onChange({ ...draft, description: event.target.value })}
              placeholder="Extracts text and tables from PDF files and fills PDF forms. Use when working with PDFs."
              rows={3}
            />
          </label>
          <label className="wide-field">
            <span>Instructions <small>Markdown. The agent reads these when it uses the skill.</small></span>
            <textarea
              className="subagent-prompt-input skill-body-input"
              value={draft.body}
              onChange={(event) => onChange({ ...draft, body: event.target.value })}
              placeholder={"# PDF tools\n\n1. Check the file is a PDF.\n2. …"}
              rows={14}
            />
          </label>
        </div>
        {shownIssue && <p className="subagent-issue skill-editor-issue" role="status">{shownIssue}</p>}
        <div className="subagent-editor-actions">
          <small className="subagent-hint">Saved to ~/.agents/skills{draft.name ? `/${draft.name}` : ""}/SKILL.md</small>
          <span className="subagent-editor-spacer" />
          <button type="button" className="secondary-button" disabled={busy} onClick={onCancel}>Cancel</button>
          <button type="button" className="primary-button" disabled={busy || Boolean(issue)} onClick={onSave}>
            {isNew ? "Create skill" : "Save"}
          </button>
        </div>
      </div>
    </article>
  );
}
