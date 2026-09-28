import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type {
  MemoryDocument, MemoryEntry, MemoryProject, MemoryType, MemoriesChange, MemoriesOverview,
  SaveMemoryInput
} from "../types";
import { formatRelativeTime } from "../chat-utils";
import { Icon } from "./Icons";
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

/** What Settings › Memory can do; SettingsPage wires each one to `api`. Keep them stable. */
export interface MemoryActions {
  onList: () => Promise<MemoriesOverview>;
  onRead: (path: string) => Promise<MemoryDocument>;
  onSave: (input: SaveMemoryInput) => Promise<MemoriesChange>;
  onDelete: (path: string) => Promise<MemoriesChange>;
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
 * (worktrees share it), listed exactly as a chat loads them. The master switch and the
 * per-project switches apply to running chats on their next turn; file edits do too, because
 * the worker re-reads the directory before every run.
 */
export function MemorySection({ onList, onRead, onSave, onDelete, onSetProjectEnabled, onReveal, onFindFile, onSetEnabled }: MemoryActions) {
  const reduce = useReducedMotion();
  const [overview, setOverview] = useState<MemoriesOverview>();
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [view, setView] = useState<View>({ kind: "list" });
  const [deleting, setDeleting] = useState<{ entry: MemoryEntry; project: string }>();
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

  function renderProject(project: MemoryProject) {
    return (
      <section className="subagent-group" aria-label={`Memory for ${project.name}`} key={project.key}>
        <div className="skill-group-head">
          <h4>{project.name}</h4>
          <span className="skill-group-path" title={`${project.path} · ${project.dir}`}>
            {project.path} · {project.entries.length} {project.entries.length === 1 ? "note" : "notes"}
          </span>
          <span className="row-actions memory-project-actions">
            <button type="button" className="ghost-button skill-reveal" disabled={busy} onClick={() => void apply(() => onReveal(project.dir))}>
              Show in Finder
            </button>
            <button type="button" className="secondary-button memory-new" disabled={busy} onClick={() => void openEditor(project)}>
              <Icon name="plus" /> New memory
            </button>
            {renderToggle(project.key, project.enabled, `Use memory in ${project.name}`, (next) => onSetProjectEnabled(project.key, next))}
          </span>
        </div>
        {project.entries.length === 0 ? (
          <div className="package-empty memory-empty">
            <span className="package-empty-icon"><Icon name="memory" /></span>
            <h4>No memories yet</h4>
            <p>The agent saves a note here when something is worth keeping for future chats — a preference, a correction, a decision. It decides; you can edit or delete what it writes.</p>
          </div>
        ) : (
          <div className="command-list">
            <AnimatePresence initial={false}>
              {project.entries.map((entry) => renderEntry(entry, project))}
            </AnimatePresence>
          </div>
        )}
      </section>
    );
  }

  return (
    <div className="settings-scroll subagents-settings commands-settings memory-settings">
      {view.kind === "list" && (
        <div className="section-heading-row">
          <div>
            <h3>Memory</h3>
            <p>
              Notes the agent keeps for itself, per project — small things AGENTS.md files don&rsquo;t say, like a
              correction or a decision with a tricky edge. A one-line index of titles rides along in every chat;
              the agent reads a note&rsquo;s full text only when it looks relevant. Switches apply to running chats
              from their next message.
            </p>
          </div>
          <div className="row-actions">
            {overview && renderToggle("memory-master", overview.enabled, "Memory", async (next) => { await onSetEnabled(next); setOverview((current) => current ? { ...current, enabled: next } : current); })}
          </div>
        </div>
      )}
      {error && <div className="error-banner subagents-error" role="alert">{error}</div>}
      {view.kind === "list" && (loading && !overview ? (
        <div className="model-empty">Reading your memories…</div>
      ) : !overview || overview.projects.length === 0 ? (
        <div className="package-empty memory-first-empty">
          <span className="package-empty-icon"><Icon name="memory" /></span>
          <h4>No memories yet</h4>
          <p>
            Open a chat in a project and ask it to remember something. Each repository gets its own folder of
            notes here — shared by every worktree, machine-local, never inside the project.
          </p>
        </div>
      ) : (
        <>
          {!overview.enabled && (
            <div className="package-notice" role="status">
              Memory is switched off. The agent keeps nothing and its memory tools are withdrawn from every chat.
            </div>
          )}
          {overview.projects.map(renderProject)}
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
          <small className="subagent-hint">Saved to {project}&rsquo;s memory</small>
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
