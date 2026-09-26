import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type {
  CommandsConfig, SaveSlashCommandInput, SlashCommandDocument, SlashCommandEntry, SlashCommandsChange, SlashCommandsOverview
} from "../types";
import { APP_SLASH_COMMANDS, expandCommandPreview, validateCommandBody, validateCommandDescription, validateCommandHint, validateCommandName } from "../command-utils";
import { Icon } from "./Icons";
import { ConfirmDialog } from "./ui/ConfirmDialog";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

/** What Settings › Commands can do; SettingsPage wires each one to `api`. Keep them stable. */
export interface SlashCommandActions {
  onList: () => Promise<SlashCommandsOverview>;
  onRead: (path: string) => Promise<SlashCommandDocument>;
  onSave: (input: SaveSlashCommandInput) => Promise<SlashCommandsChange>;
  onDelete: (path: string) => Promise<SlashCommandsChange>;
  onSetEnabled: (key: string, enabled: boolean) => Promise<SlashCommandsChange>;
  onReveal: (path: string) => Promise<void>;
  /** Keeps `data.commands` in step, so the composer filters switched-off WackCode commands too. */
  onChanged: (config: CommandsConfig) => void;
}

interface CommandDraft {
  name: string;
  description: string;
  argumentHint: string;
  body: string;
}

type View =
  | { kind: "list" }
  | { kind: "edit"; draft: CommandDraft; path?: string; original?: string };

const EMPTY_DRAFT: CommandDraft = { name: "", description: "", argumentHint: "", body: "" };

/** Why a draft can't be saved yet, or nothing when it can. Rust checks it all again on save. */
export function commandDraftIssue(draft: CommandDraft, taken: ReadonlySet<string>, original?: string): string | undefined {
  const name = draft.name.trim();
  const issue = validateCommandName(name);
  if (issue) return issue;
  if (name !== original && taken.has(name)) return `A command named ${name} already exists in Your commands.`;
  return validateCommandDescription(draft.description) ?? validateCommandHint(draft.argumentHint) ?? validateCommandBody(draft.body);
}

function matches(name: string, description: string, filter: string): boolean {
  if (!filter) return true;
  const needle = filter.toLowerCase();
  return name.toLowerCase().includes(needle) || description.toLowerCase().includes(needle);
}

const KIND_BADGES: Record<SlashCommandEntry["kind"], string> = {
  app: "WackCode",
  custom: "Yours",
  extension: "Extension",
  prompt: "Prompt"
};

/**
 * Settings › Commands. WackCode's own commands first (the app's six, offered before any
 * worker catalog is consulted), then the user's own files in `<app data>/commands`, then what
 * trusted packages add — listed by the same scan a chat's `/` uses, so the names and clash
 * resolutions agree. Every change reaches running chats on their next turn.
 */
export function CommandsSection({ onList, onRead, onSave, onDelete, onSetEnabled, onReveal, onChanged }: SlashCommandActions) {
  const reduce = useReducedMotion();
  const [overview, setOverview] = useState<SlashCommandsOverview>();
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [note, setNote] = useState<string>();
  const [view, setView] = useState<View>({ kind: "list" });
  const [filter, setFilter] = useState("");
  const [deleting, setDeleting] = useState<SlashCommandEntry>();
  const [opening, setOpening] = useState<string>();

  useEffect(() => {
    let active = true;
    void onList()
      .then((next) => {
        if (!active) return;
        setOverview(next);
        onChanged({ disabled: next.disabled });
      })
      .catch((reason: unknown) => { if (active) setError(String(reason)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [onList, onChanged]);

  async function apply(action: () => Promise<SlashCommandsChange | null | void>, after?: () => void): Promise<void> {
    setBusy(true);
    setError(undefined);
    setNote(undefined);
    try {
      const change = await action();
      if (change) {
        setOverview(change.overview);
        onChanged(change.config);
        setNote(change.note);
      }
      after?.();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const disabledSet = useMemo(() => new Set(overview?.disabled ?? []), [overview]);
  const customGroup = overview?.groups.find((group) => group.kind === "custom");
  const packageGroups = overview?.groups.filter((group) => group.kind === "package" && (group.entries.length > 0 || group.diagnostics.length > 0)) ?? [];
  const taken = useMemo(() => new Set(customGroup?.entries.map((entry) => entry.rawName ?? entry.name) ?? []), [customGroup]);

  const appEntries = useMemo(() => APP_SLASH_COMMANDS
    .filter((command) => matches(command.name, command.description ?? "", filter))
    .map((command) => ({ command, enabled: !disabledSet.has(command.id) })), [disabledSet, filter]);

  const total = appEntries.length + (overview?.groups.reduce((sum, group) => sum + group.entries.length, 0) ?? 0);

  async function openEditor(entry?: SlashCommandEntry): Promise<void> {
    setError(undefined);
    setNote(undefined);
    if (!entry?.filePath) {
      setView({ kind: "edit", draft: EMPTY_DRAFT });
      return;
    }
    setOpening(entry.key);
    try {
      const document = await onRead(entry.filePath);
      setView({
        kind: "edit",
        path: entry.filePath,
        original: entry.rawName ?? entry.name,
        draft: {
          name: entry.rawName ?? entry.name,
          description: entry.description,
          argumentHint: entry.argumentHint ?? "",
          body: document.body
        }
      });
    } catch (reason) {
      setError(String(reason));
    } finally {
      setOpening(undefined);
    }
  }

  function renderToggle(key: string, enabled: boolean, label: string) {
    return (
      <button
        type="button"
        role="switch"
        aria-checked={enabled}
        aria-label={label}
        className={`toggle ${enabled ? "on" : ""}`}
        disabled={busy}
        onClick={() => void apply(() => onSetEnabled(key, !enabled))}
      >
        <span />
      </button>
    );
  }

  function renderEntry(entry: SlashCommandEntry, origin: string) {
    const openingThis = opening === entry.key;
    return (
      <motion.div
        layout="position"
        initial={reduce ? false : { opacity: 0, y: 6 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: -4 }}
        transition={reduce ? { duration: 0 } : { duration: 0.22, ease: EASE }}
        className={`command-row ${entry.enabled ? "" : "off"}`}
        key={entry.key}
      >
        <button
          type="button"
          className="command-row-main"
          disabled={busy || openingThis || !entry.editable}
          title={entry.editable ? `Edit /${entry.rawName ?? entry.name}` : undefined}
          onClick={() => entry.editable && void openEditor(entry)}
        >
          <span className="command-row-head">
            <span className="command-name">/{entry.name}</span>
            {entry.rawName && <span className="subagent-badge" title={`Offered as /${entry.name} because another command is already /${entry.rawName}`}>was /{entry.rawName}</span>}
            <span className="subagent-badge">{origin}</span>
            {!entry.enabled && <span className="subagent-badge">Off</span>}
          </span>
          <span className="command-description">
            {entry.description || <em>No description</em>}
          </span>
          {entry.argumentHint && <code className="command-hint">/{entry.name} {entry.argumentHint}</code>}
        </button>
        <span className="command-row-actions">
          {entry.editable && (
            <button
              type="button"
              className="command-icon-button"
              aria-label={`Delete /${entry.rawName ?? entry.name}`}
              disabled={busy || openingThis}
              onClick={() => setDeleting(entry)}
            >
              <Icon name="trash" />
            </button>
          )}
          {openingThis ? <span className="command-row-status">Opening…</span> : renderToggle(entry.key, entry.enabled, `Use /${entry.name}`)}
        </span>
      </motion.div>
    );
  }

  function renderDiagnostics(group: { diagnostics: SlashCommandsOverview["groups"][number]["diagnostics"] }) {
    if (group.diagnostics.length === 0) return null;
    return (
      <ul className="skill-diagnostics">
        {group.diagnostics.map((diagnostic, index) => (
          <li key={`${diagnostic.path}:${index}`}>{diagnostic.path ? <code>{diagnostic.path}</code> : null}{diagnostic.path ? ": " : null}{diagnostic.message}</li>
        ))}
      </ul>
    );
  }

  function renderList() {
    if (loading && !overview) return <div className="model-empty">Reading your commands…</div>;
    if (!overview) return null;
    const mine = (customGroup?.entries ?? []).filter((entry) => matches(entry.name, entry.description, filter));
    return (
      <>
        {total > 6 && (
          <div className="command-try" role="search">
            <span className="command-try-slash" aria-hidden="true">/</span>
            <input
              value={filter}
              onChange={(event) => setFilter(event.target.value.replace(/^\//, ""))}
              placeholder="Type to filter commands…"
              aria-label="Filter commands"
              spellCheck={false}
            />
          </div>
        )}

        <section className="subagent-group" aria-label="WackCode commands">
          <div className="skill-group-head">
            <h4>WackCode</h4>
            <span className="skill-group-path">The app&rsquo;s own commands — they act on the chat or the app itself</span>
          </div>
          <div className="command-list">
            <AnimatePresence initial={false}>
              {appEntries.map(({ command, enabled }) => (
                <motion.div
                  layout="position"
                  initial={reduce ? false : { opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={reduce ? { duration: 0 } : { duration: 0.22, ease: EASE }}
                  className={`command-row ${enabled ? "" : "off"}`}
                  key={command.id}
                >
                  <div className="command-row-main static">
                    <span className="command-row-head">
                      <span className="command-name">/{command.name}</span>
                      <span className="subagent-badge">WackCode</span>
                      {!enabled && <span className="subagent-badge">Off</span>}
                    </span>
                    <span className="command-description">{command.description}</span>
                  </div>
                  <span className="command-row-actions">{renderToggle(command.id, enabled, `Use /${command.name}`)}</span>
                </motion.div>
              ))}
            </AnimatePresence>
            {appEntries.length === 0 && <small className="subagent-hint">No WackCode commands match the filter.</small>}
          </div>
        </section>

        <section className="subagent-group" aria-label="Your commands">
          <div className="skill-group-head">
            <h4>Your commands</h4>
            <span className="skill-group-path" title={overview.customPath}>{overview.customPath}</span>
            <button type="button" className="ghost-button skill-reveal" disabled={busy} onClick={() => void apply(() => onReveal(overview.customPath))}>
              Show in Finder
            </button>
          </div>
          {(customGroup?.entries.length ?? 0) === 0 ? (
            <div className="package-empty">
              <span className="package-empty-icon"><Icon name="slash" /></span>
              <h4>No commands of your own yet</h4>
              <p>
                A command is a saved instruction the agent expands when you type its name — like a snippet with arguments.
                Write <code>Review $ARGUMENTS</code> once and run it as <code>/review src/main.ts</code>.
              </p>
              <button type="button" className="secondary-button" disabled={busy} onClick={() => void openEditor()}>
                <Icon name="plus" /> New command
              </button>
            </div>
          ) : mine.length === 0 ? (
            <small className="subagent-hint">None of your commands match the filter.</small>
          ) : (
            <div className="command-list">
              <AnimatePresence initial={false}>
                {mine.map((entry) => renderEntry(entry, "Yours"))}
              </AnimatePresence>
            </div>
          )}
        </section>

        {packageGroups.map((group) => {
          const shown = group.entries.filter((entry) => matches(entry.name, entry.description, filter));
          const on = group.entries.filter((entry) => entry.enabled).length;
          return (
            <section className="subagent-group" aria-label={`${group.label} commands`} key={group.id}>
              <div className="skill-group-head">
                <h4>{group.label}</h4>
                <span className="skill-group-path" title={group.id}>{group.id} · {on}/{group.entries.length} on</span>
              </div>
              {shown.length === 0 ? (
                <small className="subagent-hint">{filter ? "No commands here match the filter." : "This package&rsquo;s commands could not be read."}</small>
              ) : (
                <div className="command-list">
                  <AnimatePresence initial={false}>
                    {shown.map((entry) => renderEntry(entry, KIND_BADGES[entry.kind]))}
                  </AnimatePresence>
                </div>
              )}
              {renderDiagnostics(group)}
            </section>
          );
        })}
      </>
    );
  }

  return (
    <div className="settings-scroll subagents-settings commands-settings">
      {view.kind === "list" && (
        <div className="section-heading-row">
          <div>
            <h3>Slash commands</h3>
            <p>
              What typing <code>/</code> in a chat offers. Commands a package adds run its code; your own expand saved
              instructions with arguments like <code>$1</code> and <code>$ARGUMENTS</code>. Switches apply to running
              chats from their next message.
            </p>
          </div>
          <div className="row-actions">
            <button type="button" className="secondary-button" disabled={busy || !overview} onClick={() => void openEditor()}>
              <Icon name="plus" /> New command
            </button>
          </div>
        </div>
      )}
      {error && <div className="error-banner subagents-error" role="alert">{error}</div>}
      {note && <div className="package-notice" role="status">{note}</div>}
      {view.kind === "list" && renderList()}
      {view.kind === "edit" && (
        <CommandEditor
          draft={view.draft}
          isNew={!view.path}
          issue={commandDraftIssue(view.draft, taken, view.original)}
          busy={busy}
          customPath={overview?.customPath ?? ""}
          onChange={(draft) => setView({ ...view, draft })}
          onCancel={() => { setView({ kind: "list" }); setError(undefined); }}
          onDelete={view.path ? () => {
            const entry = customGroup?.entries.find((item) => item.filePath === view.path);
            if (entry) setDeleting(entry);
          } : undefined}
          onSave={() => void apply(
            () => onSave({
              path: view.path,
              name: view.draft.name.trim(),
              description: view.draft.description.trim(),
              argumentHint: view.draft.argumentHint.trim(),
              body: view.draft.body
            }),
            () => setView({ kind: "list" })
          )}
        />
      )}

      {deleting && (
        <ConfirmDialog
          title={`Delete /${deleting.rawName ?? deleting.name}?`}
          body="Its file moves to the Trash, so chats stop offering it. You can put it back from the Trash."
          confirmLabel="Move to Trash"
          danger
          onConfirm={async () => {
            if (!deleting.filePath) return;
            const change = await onDelete(deleting.filePath);
            setOverview(change.overview);
            onChanged(change.config);
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
  draft: CommandDraft;
  isNew: boolean;
  issue?: string;
  busy: boolean;
  customPath: string;
  onChange: (draft: CommandDraft) => void;
  onCancel: () => void;
  onDelete?: () => void;
  onSave: () => void;
}

/** Name, description, optional argument hint, and the instructions — with a live preview of
 * exactly what the agent receives for a sample argument line, using Pi's substitution rules. */
function CommandEditor({ draft, isNew, issue, busy, customPath, onChange, onCancel, onDelete, onSave }: EditorProps) {
  const reduce = useReducedMotion();
  const [sampleArgs, setSampleArgs] = useState("");
  const descriptionLength = draft.description.trim().length;
  // Only flag the name once there is one: an empty new form isn't an error yet.
  const shownIssue = draft.name || draft.body ? issue : undefined;
  const preview = useMemo(() => expandCommandPreview(draft.body, sampleArgs), [draft.body, sampleArgs]);
  const hasArgs = /\$(?:\d|@|ARGUMENTS|\{)/.test(draft.body);
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
            <span>Name <small>Lowercase letters, numbers and hyphens. What you type after /.</small></span>
            <div className="command-name-field">
              <span aria-hidden="true">/</span>
              <input
                value={draft.name}
                onChange={(event) => onChange({ ...draft, name: event.target.value.toLowerCase().replace(/\s+/g, "-") })}
                placeholder="review"
                spellCheck={false}
                maxLength={64}
              />
            </div>
          </label>
          <label>
            <span>Argument hint <small>Shown next to the name while typing, e.g. &lt;files&gt;.</small></span>
            <input
              value={draft.argumentHint}
              onChange={(event) => onChange({ ...draft, argumentHint: event.target.value })}
              placeholder="<files to review>"
              spellCheck={false}
              maxLength={256}
            />
          </label>
          <label className="wide-field">
            <span>
              Description{" "}
              <small className={descriptionLength > 1024 ? "mcp-invalid" : undefined}>
                One line the `/` list shows. {descriptionLength}/1024
              </small>
            </span>
            <input
              value={draft.description}
              onChange={(event) => onChange({ ...draft, description: event.target.value })}
              placeholder="Review files or a diff and report issues"
              spellCheck={false}
            />
          </label>
          <label className="wide-field">
            <span>Instructions <small>What the agent is asked to do. <code>$1</code>, <code>$ARGUMENTS</code>, <code>${"@:2"}</code> and <code>{"${1:-default}"}</code> are filled in when it runs.</small></span>
            <textarea
              className="subagent-prompt-input command-body-input"
              value={draft.body}
              onChange={(event) => onChange({ ...draft, body: event.target.value })}
              placeholder={"Review these files for bugs and style issues:\n$ARGUMENTS\n\nReport each issue with the file and line."}
              rows={8}
            />
          </label>
          {draft.body.trim() && (
            <div className="wide-field command-preview">
              <span className="subagent-field-label">
                Preview <small className="subagent-hint">{hasArgs ? "Try arguments; the agent receives this expanded text." : "The agent receives this text as typed."}</small>
              </span>
              <input
                className="command-preview-args"
                value={sampleArgs}
                onChange={(event) => setSampleArgs(event.target.value)}
                placeholder={`/${draft.name || "name"} …type sample arguments…`}
                aria-label="Sample arguments for the preview"
                spellCheck={false}
              />
              <pre className="subagent-prompt-view command-preview-view">{preview || <em>Nothing yet</em>}</pre>
            </div>
          )}
        </div>
        {shownIssue && <p className="subagent-issue command-editor-issue" role="status">{shownIssue}</p>}
        <div className="subagent-editor-actions">
          {isNew ? (
            <small className="subagent-hint">Saved to {customPath}{draft.name ? `/${draft.name}` : ""}.md</small>
          ) : (
            <button type="button" className="danger-button" disabled={busy} onClick={onDelete}>Delete</button>
          )}
          <span className="subagent-editor-spacer" />
          <button type="button" className="secondary-button" disabled={busy} onClick={onCancel}>Cancel</button>
          <button type="button" className="primary-button" disabled={busy || Boolean(issue)} onClick={onSave}>
            {isNew ? "Create command" : "Save"}
          </button>
        </div>
      </div>
    </motion.article>
  );
}
