import { useState } from "react";
import { DEFAULT_PLAN_PROMPT, DEFAULT_SYSTEM_PROMPT, DEFAULT_ULTRA_PLAN_PROMPT } from "../promptDefaults";
import type { PromptConfig } from "../types";
import { Icon } from "./Icons";
import { ConfirmDialog } from "./ui/ConfirmDialog";

/** Mirrors MAX_PROMPT_OVERRIDE_CHARS in commands.rs, which enforces it for real. */
const MAX_PROMPT_CHARS = 20_000;

type PromptKey = keyof PromptConfig;

interface PromptSpec {
  key: PromptKey;
  title: string;
  description: string;
  /** What the editor is prefilled with: the user's text, or the built-in text to start from. */
  defaultText: string;
  /** Plan contracts carry an app-owned marker line above the user's text. */
  note?: string;
}

const PROMPTS: PromptSpec[] = [
  {
    key: "systemPrompt",
    title: "Default system prompt",
    description:
      "The persona every chat opens with. Your text replaces this opening paragraph; the tool docs, rules and project context are always assembled after it.",
    defaultText: DEFAULT_SYSTEM_PROMPT
  },
  {
    key: "planPrompt",
    title: "Plan mode",
    description: "The instructions steering a chat while Plan mode is on.",
    defaultText: DEFAULT_PLAN_PROMPT,
    note: "WackCode adds one hidden header line above this text to recognise the contract; everything you write here is the contract the model sees."
  },
  {
    key: "ultraPlanPrompt",
    title: "Ultra Plan",
    description: "The instructions steering a chat while Ultra Plan is on.",
    defaultText: DEFAULT_ULTRA_PLAN_PROMPT,
    note: "WackCode adds one hidden header line above this text to recognise the contract; everything you write here is the contract the model sees."
  }
];

interface Props {
  config: PromptConfig;
  onChange: (config: PromptConfig) => Promise<void>;
}

export function PromptsSection({ config, onChange }: Props) {
  const [editingKey, setEditingKey] = useState<PromptKey>();
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [restoring, setRestoring] = useState<PromptSpec>();

  async function save(spec: PromptSpec): Promise<void> {
    setBusy(true);
    setError(undefined);
    try {
      await onChange({ ...config, [spec.key]: draft });
      setEditingKey(undefined);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function restore(spec: PromptSpec): Promise<void> {
    setBusy(true);
    setError(undefined);
    try {
      await onChange({ ...config, [spec.key]: null });
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const overLimit = draft.length > MAX_PROMPT_CHARS;
  const blank = draft.trim().length === 0;

  return (
    <div className="settings-scroll prompts-settings">
      <div className="section-heading-row">
        <div>
          <h3>Built-in prompts</h3>
          <p>
            These prompts are designed to work best with Pi and WackCode. Customising them is optional — change them to your own
            liking if you wish, and restore the built-in text at any time. Changes apply from the next message in your chats,
            including chats that are already planning.
          </p>
        </div>
      </div>
      {PROMPTS.map((spec) => {
        const customized = typeof config[spec.key] === "string";
        const editing = editingKey === spec.key;
        const effective = config[spec.key] ?? spec.defaultText;
        return (
          <article className="prompt-card" key={spec.key} aria-label={spec.title}>
            <header className="prompt-card-head">
              <div>
                <h4>
                  {spec.title} <span className={`prompt-badge ${customized ? "customized" : ""}`}>{customized ? "Customised" : "Default"}</span>
                </h4>
                <p>{spec.description}</p>
              </div>
              {!editing && (
                <div className="prompt-card-actions">
                  {customized && (
                    <button type="button" className="danger-button" disabled={busy} onClick={() => setRestoring(spec)}>
                      Restore default
                    </button>
                  )}
                  <button
                    type="button"
                    className="secondary-button"
                    disabled={busy}
                    onClick={() => {
                      setDraft(effective);
                      setEditingKey(spec.key);
                    }}
                  >
                    <Icon name="pencil" /> {customized ? "Edit" : "Customise"}
                  </button>
                </div>
              )}
            </header>
            {editing ? (
              <div className="prompt-editor">
                <textarea
                  className="prompt-editor-input"
                  value={draft}
                  onChange={(event) => setDraft(event.target.value)}
                  aria-label={`${spec.title} text`}
                  rows={spec.key === "systemPrompt" ? 5 : 16}
                  spellCheck={false}
                />
                <div className="prompt-editor-actions">
                  <small className={`prompt-count ${overLimit ? "over" : ""}`}>
                    {draft.length.toLocaleString()} / {MAX_PROMPT_CHARS.toLocaleString()} characters
                  </small>
                  <span className="prompt-editor-spacer" />
                  <button type="button" className="secondary-button" disabled={busy} onClick={() => setEditingKey(undefined)}>
                    Cancel
                  </button>
                  <button type="button" className="primary-button" disabled={busy || blank || overLimit} onClick={() => void save(spec)}>
                    Save
                  </button>
                </div>
                {spec.note && <small className="prompt-note">{spec.note}</small>}
              </div>
            ) : (
              <pre className="prompt-view">{effective}</pre>
            )}
          </article>
        );
      })}
      {error && <div className="error-banner">{error}</div>}
      {restoring && (
        <ConfirmDialog
          title="Restore the default prompt?"
          body={`This discards your custom ${restoring.title.toLowerCase()} and returns to the built-in text. Chats pick the built-in text up from their next message.`}
          confirmLabel="Restore"
          danger
          onConfirm={() => restore(restoring)}
          onCancel={() => setRestoring(undefined)}
        />
      )}
    </div>
  );
}
