import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type {
  MemoryDocument, MemoryEntry, MemoryProject, MemoryType, MemoriesChange, MemoriesOverview,
  SaveMemoryInput
} from "../types";
import { formatRelativeTime } from "../chat-utils";
import { Icon, type IconName } from "./Icons";
import { SettingsHero, stagger } from "./SettingsHero";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { Select } from "./ui/Select";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

const MEMORY_TYPES: { value: MemoryType; label: string; hint: string }[] = [
  { value: "user", label: "User", hint: "Role, expertise and lasting preferences" },
  { value: "feedback", label: "Feedback", hint: "Corrections and confirmed approaches" },
  { value: "project", label: "Project", hint: "Ongoing work, decisions, deadlines" },
  { value: "reference", label: "Reference", hint: "Where information lives outside the project" }
];

/** How memory works, for the page with no project yet. */
const MEMORY_STEPS: { icon: IconName; title: string; detail: string }[] = [
  { icon: "comment", title: "You mention something worth keeping", detail: "A preference, a correction, a decision with a tricky edge." },
  { icon: "memory", title: "The agent saves a small note", detail: "Kept on this Mac with WackCode's data, never inside the project." },
  { icon: "brain", title: "Later chats recall it", detail: "Only when the note looks relevant to what you ask." }
];

/**
 * The hero's little stage: a fresh note is filed onto a small stack, its lines appear and a
 * bookmark ribbon drops onto it; then it settles away for the next. Rests dashed and dimmed while
 * memory is off. Pure decoration; the pill says the same in words. The loop lives in styles.css,
 * which stills it under reduced motion.
 */
function MemoryStage({ live }: { live: boolean }) {
  const lines = [{ y: 43, width: 50 }, { y: 53, width: 42 }, { y: 63, width: 50 }, { y: 73, width: 28 }];
  return (
    <svg className={`settings-stage memory-stage ${live ? "live" : ""}`} viewBox="0 0 160 110" aria-hidden="true">
      <rect className="memory-stage-card" x="44" y="16" width="76" height="82" rx="9" transform="rotate(-9 82 57)" />
      <rect className="memory-stage-card" x="44" y="16" width="76" height="82" rx="9" transform="rotate(6 82 57)" />
      <g className="memory-stage-note">
        <rect className="memory-stage-card top" x="42" y="14" width="76" height="82" rx="9" />
        <rect className="memory-stage-title" x="53" y="27" width="30" height="5" rx="2.5" />
        {lines.map((line, index) => (
          <rect
            key={line.y}
            className="memory-stage-line"
            x="53"
            y={line.y}
            width={line.width}
            height="4"
            rx="2"
            style={{ "--k": index } as React.CSSProperties}
          />
        ))}
        <path className="memory-stage-ribbon" d="M96 14h12v21l-6-5-6 5z" />
      </g>
    </svg>
  );
}

/** What Settings › Memory can do; SettingsPage wires each one to `api`. Keep them stable. */
export interface MemoryActions {
  onList: () => Promise<MemoriesOverview>;
  onRead: (path: string) => Promise<MemoryDocument>;
  onSave: (input: SaveMemoryInput) => Promise<MemoriesChange>;
  onDelete: (path: string) => Promise<MemoriesChange>;
  /** Moves a whole project's folder — notes and all — to the Trash. */
  onRemoveProject: (dir: string) => Promise<MemoriesChange>;
  onSetProjectEnabled: (key: string, enabled: boolean) => Promise<MemoriesChange>;
  /** Opens the project's memory folder itself. */
  onReveal: (path: string) => Promise<void>;
  /** Shows one note's file selected in Finder. */
  onFindFile: (path: string) => Promise<void>;
  /** The master switch, saved through the app's config like the section's other props. */
  onSetEnabled: (enabled: boolean) => Promise<void>;
}

interface MemoryDraft {
  name: string;
  memoryType: MemoryType;
  title: string;
  description: string;
  body: string;
}

type View =
  | { kind: "list" }
  // `original` is the name the note already holds, so the taken-name check skips its own file.
  | { kind: "edit"; draft: MemoryDraft; dir: string; path?: string; original?: string; project: string };

/** Why a draft can't be saved yet, or nothing when it can. Rust checks it all again on save. */
export function memoryDraftIssue(draft: MemoryDraft, taken: ReadonlySet<string>, original?: string): string | undefined {
  const name = draft.name.trim();
  if (!draft.title.trim()) return "Give the memory a title.";
  if (!draft.body.trim()) return "The memory needs something to remember.";
  if (name !== original && taken.has(name)) return `A memory named ${name} already exists in this project.`;
  if (name.length > 80) return "A memory name can be at most 80 characters.";
  if (!/^[a-z0-9][a-z0-9_-]*$/.test(name)) return "Use lowercase letters, numbers, hyphens and underscores, like feedback_run-tests.";
  return undefined;
}

function nameSuggestion(type: MemoryType, title: string): string {
  const slug = title
    .normalize("NFKD").replace(/[\u0300-\u036f]/g, "")
    .toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 64).replace(/-+$/g, "");
  return `${type}_${slug || "note"}`;
}

/**
 * Settings › Memory. Every project the agent has saved notes for, one directory per repository
 * (worktrees share it), listed exactly as a chat loads them. A project with notes gets its own
 * card; projects still waiting for one share a card, one row each. The master switch and the
 * per-project switches apply to running chats on their next turn; file edits do too, because
 * the worker re-reads the directory before every run.
 */
export function MemorySection({
  agentName = "WackCode", onList, onRead, onSave, onDelete, onRemoveProject, onSetProjectEnabled, onReveal, onFindFile, onSetEnabled
}: MemoryActions & { agentName?: string }) {
  const reduce = useReducedMotion();
  const [overview, setOverview] = useState<MemoriesOverview>();
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [view, setView] = useState<View>({ kind: "list" });
  const [deleting, setDeleting] = useState<{ entry: MemoryEntry; project: string }>();
  const [removing, setRemoving] = useState<MemoryProject>();
  const [opening, setOpening] = useState<string>();

  useEffect(() => {
    let active = true;
    void onList()
      .then((next) => { if (active) setOverview(next); })
      .catch((reason: unknown) => { if (active) setError(String(reason)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [onList]);

  async function apply(action: () => Promise<MemoriesChange | null | void>, after?: () => void): Promise<void> {
    setBusy(true);
    setError(undefined);
    try {
      const change = await action();
      if (change) setOverview(change.overview);
      after?.();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const takenNames = useMemo(
    () => new Set((view.kind === "edit" ? overview?.projects.find((project) => project.dir === view.dir)?.entries ?? [] : []).map((entry) => entry.name)),
    [overview, view]
  );

  async function openEditor(project: MemoryProject, entry?: MemoryEntry): Promise<void> {
    setError(undefined);
    if (!entry) {
      setView({ kind: "edit", dir: project.dir, project: project.name, draft: { name: "", memoryType: "project", title: "", description: "", body: "" } });
      return;
    }
    setOpening(entry.filePath);
    try {
      const document = await onRead(entry.filePath);
      setView({
        kind: "edit",
        dir: project.dir,
        path: entry.filePath,
        original: entry.name,
        project: project.name,
        draft: { name: entry.name, memoryType: entry.kind, title: entry.title, description: entry.description, body: document.body }
      });
    } catch (reason) {
      setError(String(reason));
    } finally {
      setOpening(undefined);
    }
  }

  function renderToggle(key: string, enabled: boolean, label: string, onToggle: (next: boolean) => Promise<MemoriesChange | null | void>) {
    return (
      <button
        type="button"
        role="switch"
        aria-checked={enabled}
        aria-label={label}
        className={`toggle ${enabled ? "on" : ""}`}
        disabled={busy}
        onClick={() => void apply(() => onToggle(!enabled))}
      >
        <span />
      </button>
    );
  }

  function renderEntry(entry: MemoryEntry, project: MemoryProject) {
    const openingThis = opening === entry.filePath;
    return (
      <motion.div
        layout="position"
        initial={reduce ? false : { opacity: 0, y: 6 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: -4 }}
        transition={reduce ? { duration: 0 } : { duration: 0.22, ease: EASE }}
        className={`command-row memory-row ${project.enabled && overview?.enabled ? "" : "off"}`}
        key={entry.filePath}
      >
        <button
          type="button"
          className="command-row-main"
          disabled={busy || openingThis}
          title={`Edit ${entry.title}`}
          onClick={() => void openEditor(project, entry)}
        >
          <span className="command-row-head">
            <span className="command-name">{entry.title}</span>
            <span className="subagent-badge memory-type-badge" data-type={entry.kind}>{entry.kind}</span>
            {entry.modified && <span className="memory-when" title={entry.modified}>{formatRelativeTime(entry.modified)}</span>}
          </span>
          <span className="command-description">
            {entry.description || <em>No description — the index shows the title alone</em>}
          </span>
          <code className="command-hint">{entry.name}</code>
        </button>
        <span className="command-row-actions">
          <button
            type="button"
            className="command-icon-button"
            aria-label={`Find ${entry.title} in Finder`}
            title="Show this note's file in Finder"
            disabled={busy || openingThis}
            onClick={() => void apply(() => onFindFile(entry.filePath))}
          >
            <Icon name="folder" />
          </button>
          <button
            type="button"
            className="command-icon-button"
            aria-label={`Delete ${entry.title}`}
            disabled={busy || openingThis}
            onClick={() => setDeleting({ entry, project: project.name })}
          >
            <Icon name="trash" />
          </button>
          {openingThis ? <span className="command-row-status">Opening…</span> : null}
        </span>
      </motion.div>
    );
  }

  function renderFolderActions(project: MemoryProject) {
    return (
      <span className="memory-project-actions">
        <button
          type="button"
          className="command-icon-button memory-reveal"
          aria-label={`Show ${project.name}'s memory folder in Finder`}
          disabled={busy}
          onClick={() => void apply(() => onReveal(project.dir))}
        >
          <Icon name="folder" />
        </button>
        <button
          type="button"
          className="command-icon-button"
          aria-label={`Delete ${project.name}'s memory folder`}
          disabled={busy}
          onClick={() => setRemoving(project)}
        >
          <Icon name="trash" />
        </button>
        {renderToggle(project.key, project.enabled, `Use memory in ${project.name}`, (next) => onSetProjectEnabled(project.key, next))}
      </span>
    );
  }

  function renderProject(project: MemoryProject, index: number) {
    const count = project.entries.length;
    const kinds = MEMORY_TYPES
      .map((type) => ({ ...type, count: project.entries.filter((entry) => entry.kind === type.value).length }))
      .filter((type) => type.count > 0);
    return (
      <section
        className={`settings-block memory-project ${project.enabled && overview?.enabled ? "" : "off"}`}
        style={stagger(index)}
        aria-label={`Memory for ${project.name}`}
        key={project.key}
      >
        <header className="memory-project-head">
          <span className="memory-project-mono" aria-hidden="true">{project.name.charAt(0).toUpperCase()}</span>
          <div className="memory-project-title">
            <h3 className="settings-block-title">{project.name}</h3>
            <span className="memory-project-path" title={`${project.path} · ${project.dir}`}>
              {project.path} · {count} {count === 1 ? "note" : "notes"}
            </span>
          </div>
          {renderFolderActions(project)}
        </header>
        <ul className="memory-kinds" aria-label={`${project.name}'s notes by type`}>
          {kinds.map((kind) => (
            <li key={kind.value} data-type={kind.value}><i aria-hidden="true" />{kind.count} {kind.label.toLowerCase()}</li>
          ))}
        </ul>
        <div className="command-list">
          <AnimatePresence initial={false}>
            {project.entries.map((entry) => renderEntry(entry, project))}
          </AnimatePresence>
        </div>
        <button type="button" className="memory-add" disabled={busy} onClick={() => void openEditor(project)}>
          <Icon name="plus" /> New memory
        </button>
      </section>
    );
  }

  /** Projects without a note yet share one card, so an empty folder costs a row, not a page. */
  function renderWaiting(projects: MemoryProject[], index: number) {
    return (
      <section className="settings-block memory-waiting" style={stagger(index)} aria-labelledby="memory-waiting-title">
        <h3 className="settings-block-title" id="memory-waiting-title">Waiting for a first note</h3>
        <p className="settings-block-sub">
          The agent saves a note when something is worth keeping for future chats: a preference, a correction, a decision.
          It decides; you can edit or delete what it writes, or write one yourself.
        </p>
        <ul className="memory-waiting-list">
          {projects.map((project) => (
            <li key={project.key} className={project.enabled && overview?.enabled ? "" : "off"} aria-label={`Memory for ${project.name}`}>
              <span className="memory-project-mono" aria-hidden="true">{project.name.charAt(0).toUpperCase()}</span>
              <div className="memory-waiting-text">
                <strong>{project.name}</strong>
                <span title={`${project.path} · ${project.dir}`}>{project.path}</span>
              </div>
              <button
                type="button"
                className="ghost-button memory-write"
                aria-label={`New memory in ${project.name}`}
                disabled={busy}
                onClick={() => void openEditor(project)}
              >
                <Icon name="plus" /> Write one
              </button>
              {renderFolderActions(project)}
            </li>
          ))}
        </ul>
      </section>
    );
  }

  const projects = overview?.projects ?? [];
  const noted = projects.filter((project) => project.entries.length > 0);
  const waiting = projects.filter((project) => project.entries.length === 0);
  const total = noted.reduce((sum, project) => sum + project.entries.length, 0);
  const on = overview?.enabled ?? false;
  const pill = !overview ? "Reading…" : !on ? "Off" : total === 0 ? "No notes yet" : `${total} ${total === 1 ? "note" : "notes"}`;

  return (
    <div className="settings-scroll subagents-settings commands-settings memory-settings">
      <div className="settings-page">
        <SettingsHero
          label="Memory overview"
          stage={<MemoryStage live={on} />}
          live={on}
          pill={pill}
          title={`What ${agentName} remembers`}
          action={overview && (
            <span className="settings-hero-switch">
              <span aria-hidden="true">Use memory</span>
              {renderToggle("memory-master", on, "Use memory", async (next) => { await onSetEnabled(next); setOverview((current) => current ? { ...current, enabled: next } : current); })}
            </span>
          )}
        >
          <p>
            Notes the agent keeps for itself, per project. Chats carry an index of their titles and read a note only when it
            looks relevant. Switches apply from a chat&rsquo;s next message.
          </p>
        </SettingsHero>
        {error && <div className="error-banner" role="alert">{error}</div>}
        {view.kind === "list" && overview && !on && (
          <div className="package-notice" role="status">
            Memory is switched off. The agent keeps nothing and its memory tools are withdrawn from every chat.
          </div>
        )}
        {view.kind === "list" && (loading && !overview ? (
          <section className="settings-block memory-loading" style={stagger(1)}>Reading your memories…</section>
        ) : projects.length === 0 ? (
          <section className="settings-block" style={stagger(1)} aria-labelledby="memory-empty-title">
            <h3 className="settings-block-title" id="memory-empty-title">No memories yet</h3>
            <p className="settings-block-sub">
              Open a chat in a project and ask it to remember something. Each repository gets its own folder of notes here,
              shared by every worktree, machine-local, never inside the project.
            </p>
            <ol className="memory-steps">
              {MEMORY_STEPS.map((step, index) => (
                <li key={step.title}>
                  <span className="memory-step-mark" aria-hidden="true"><Icon name={step.icon} /><b>{index + 1}</b></span>
                  <strong>{step.title}</strong>
                  <span>{step.detail}</span>
                </li>
              ))}
            </ol>
          </section>
        ) : (
          <>
            {noted.map((project, index) => renderProject(project, index + 1))}
            {waiting.length > 0 && renderWaiting(waiting, noted.length + 1)}
          </>
        ))}
        {view.kind === "edit" && (
          <MemoryEditor
            draft={view.draft}
            project={view.project}
            isNew={!view.path}
            issue={memoryDraftIssue(view.draft, takenNames, view.original)}
            busy={busy}
            onChange={(draft) => setView({ ...view, draft })}
            onCancel={() => { setView({ kind: "list" }); setError(undefined); }}
            onSave={() => void apply(
              () => onSave({
                path: view.path,
                dir: view.dir,
                name: view.draft.name.trim(),
                memoryType: view.draft.memoryType,
                title: view.draft.title.trim(),
                description: view.draft.description.trim(),
                body: view.draft.body
              }),
              () => setView({ kind: "list" })
            )}
          />
        )}
      </div>

      {deleting && (
        <ConfirmDialog
          title={`Delete “${deleting.entry.title}”?`}
          body="Its file moves to the Trash, so chats stop recalling it. You can put it back from the Trash."
          confirmLabel="Move to Trash"
          danger
          onConfirm={async () => {
            const change = await onDelete(deleting.entry.filePath);
            setOverview(change.overview);
            setView({ kind: "list" });
            setDeleting(undefined);
          }}
          onCancel={() => setDeleting(undefined)}
        />
      )}

      {removing && (
        <ConfirmDialog
          title={`Delete “${removing.name}”’s memory?`}
          body={
            removing.entries.length > 0
              ? `Its ${removing.entries.length} ${removing.entries.length === 1 ? "note moves" : "notes move"} to the Trash with the folder.`
                + " If this project is still added, a fresh empty folder appears the next time the agent saves a note."
              : "The folder is removed. If this project is still added, a fresh empty folder appears the next time the agent saves a note."
          }
          confirmLabel="Move to Trash"
          danger
          onConfirm={async () => {
            const change = await onRemoveProject(removing.dir);
            setOverview(change.overview);
            setView({ kind: "list" });
            setRemoving(undefined);
          }}
          onCancel={() => setRemoving(undefined)}
        />
      )}
    </div>
  );
}

interface EditorProps {
  draft: MemoryDraft;
  project: string;
  isNew: boolean;
  issue?: string;
  busy: boolean;
  onChange: (draft: MemoryDraft) => void;
  onCancel: () => void;
  onSave: () => void;
}

/** Type, title, one-line description and the note itself. The name is the id the agent recalls. */
function MemoryEditor({ draft, project, isNew, issue, busy, onChange, onCancel, onSave }: EditorProps) {
  const reduce = useReducedMotion();
  // The name follows the title until the user edits it themselves; it is derived from the
  // patch's title, not the previous draft's, so it never lags a keystroke behind.
  const [namedByHand, setNamedByHand] = useState(!isNew);
  const next = (patch: Partial<MemoryDraft>) => {
    const merged = { ...draft, ...patch };
    onChange(namedByHand ? merged : { ...merged, name: nameSuggestion(merged.memoryType, merged.title) });
  };
  const shownIssue = draft.title || draft.body ? issue : undefined;
  return (
    <motion.article
      className="subagent-setting command-editor open new"
      initial={reduce ? false : { opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={reduce ? { duration: 0 } : { duration: 0.25, ease: EASE }}
    >
      <div className="subagent-editor">
        <div>
          <h3 className="settings-block-title">{isNew ? "New memory" : "Edit memory"}</h3>
          <p className="settings-block-sub memory-editor-sub">In {project}&rsquo;s memory folder. Chats pick it up from their next message.</p>
        </div>
        <div className="form-grid">
          <label>
            <span>Type <small>What kind of note this is; the index groups by it.</small></span>
            <Select
              value={draft.memoryType}
              options={MEMORY_TYPES}
              onChange={(value) => next({ memoryType: value as MemoryType })}
              aria-label="Memory type"
            />
          </label>
          <label>
            <span>Title <small>Shown in the index every chat carries.</small></span>
            <input
              value={draft.title}
              onChange={(event) => next({ title: event.target.value })}
              placeholder="Run worker tests after protocol changes"
              maxLength={200}
            />
          </label>
          <label className="wide-field">
            <span>Description <small>One line saying when the note matters.</small></span>
            <input
              value={draft.description}
              onChange={(event) => next({ description: event.target.value })}
              placeholder="Protocol edits need pnpm test:worker before review"
              maxLength={300}
            />
          </label>
          <label className="wide-field">
            <span>Name <small>The id the agent recalls the note by. Usually derived from the title.</small></span>
            <div className="command-name-field memory-name-field">
              <input
                value={draft.name}
                onChange={(event) => { setNamedByHand(true); onChange({ ...draft, name: event.target.value.toLowerCase().replace(/\s+/g, "-") }); }}
                placeholder="feedback_run-worker-tests"
                spellCheck={false}
                maxLength={80}
              />
            </div>
          </label>
          <label className="wide-field">
            <span>Note <small>What to remember. Keep it as small as it can be and still be useful.</small></span>
            <textarea
              className="subagent-prompt-input command-body-input"
              value={draft.body}
              onChange={(event) => next({ body: event.target.value })}
              placeholder={"Any change to worker/src/protocol.ts needs pnpm test:worker before review."}
              rows={8}
            />
          </label>
        </div>
        {shownIssue && <p className="subagent-issue command-editor-issue" role="status">{shownIssue}</p>}
        <div className="subagent-editor-actions">
          <span className="subagent-editor-spacer" />
          <button type="button" className="secondary-button" disabled={busy} onClick={onCancel}>Cancel</button>
          <button type="button" className="primary-button" disabled={busy || Boolean(issue)} onClick={onSave}>
            {isNew ? "Create memory" : "Save"}
          </button>
        </div>
      </div>
    </motion.article>
  );
}
