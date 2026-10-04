import { useState } from "react";
import { motion, useReducedMotion } from "motion/react";
import { DEFAULT_PLAN_PROMPT, DEFAULT_SYSTEM_PROMPT, DEFAULT_ULTRA_PLAN_PROMPT } from "../promptDefaults";
import type { PromptConfig } from "../types";
import { Icon, type IconName } from "./Icons";
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
  icon: IconName;
  /** Ultra Plan keeps its fixed warm palette in every theme; the rest follow the accent. */
  tone: "accent" | "ultra";
  /** Plan contracts carry an app-owned marker line above the user's text. */
  note?: string;
}

const PROMPTS: PromptSpec[] = [
  {
    key: "systemPrompt",
    title: "Default system prompt",
    description:
      "The persona every chat opens with. Your text replaces this opening paragraph; the tool docs, rules and project context are always assembled after it.",
    defaultText: DEFAULT_SYSTEM_PROMPT,
    icon: "spark",
    tone: "accent"
  },
  {
    key: "planPrompt",
    title: "Plan mode",
    description: "The instructions steering a chat while Plan mode is on.",
    defaultText: DEFAULT_PLAN_PROMPT,
    icon: "checklist",
    tone: "accent",
    note: "WackCode adds one hidden header line above this text to recognise the contract; everything you write here is the contract the model sees."
  },
  {
    key: "ultraPlanPrompt",
    title: "Ultra Plan",
    description: "The instructions steering a chat while Ultra Plan is on.",
    defaultText: DEFAULT_ULTRA_PLAN_PROMPT,
    icon: "flame",
    tone: "ultra",
    note: "WackCode adds one hidden header line above this text to recognise the contract; everything you write here is the contract the model sees."
  }
];

/** Past this many lines (or characters, for long wrapped paragraphs) a prompt's preview is clamped. */
const CLAMP_LINES = 7;
const CLAMP_CHARS = 600;

const step = (index: number) => ({ "--i": index } as React.CSSProperties);

/**
 * The hero's little stage: a page whose lines write themselves, one after another, ending on a
 * blinking caret while any prompt is customised, and resting dashed and dimmed while every prompt
 * is the built-in one. Pure decoration; the pill beside it says the same in words. The loop lives
 * in styles.css, which stills it under reduced motion.
 */
function PromptStage({ live }: { live: boolean }) {
  const lines = [
    { y: 38, width: 68 },
    { y: 51, width: 58 },
    { y: 64, width: 68 },
    { y: 77, width: 40 }
  ];
  return (
    <svg className={`prompt-stage ${live ? "live" : ""}`} viewBox="0 0 160 110" aria-hidden="true">
      <rect className="prompt-stage-page" x="34" y="8" width="92" height="94" rx="9" />
      <rect className="prompt-stage-title" x="46" y="20" width="26" height="5" rx="2.5" />
      {lines.map((line, index) => (
        <rect
          key={line.y}
          className="prompt-stage-line"
          x="46"
          y={line.y}
          width={line.width}
          height="4"
          rx="2"
          style={{ "--k": index } as React.CSSProperties}
        />
      ))}
      <rect className="prompt-stage-caret" x={46 + lines[3].width + 4} y="75" width="2" height="9" rx="1" />
    </svg>
  );
}

interface Props {
  config: PromptConfig;
  unrestrictedPlanning?: boolean;
  /** The persona's name from Settings › Appearance; the app's own copy never calls the agent "Pi". */
  agentName: string;
  onChange: (config: PromptConfig) => Promise<void>;
}

export function PromptsSection({ unrestrictedPlanning, config, agentName, onChange }: Props) {
  const reduceMotion = useReducedMotion();
  const [expanded, setExpanded] = useState<PromptKey[]>([]);
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
  const fill = Math.min(draft.length / MAX_PROMPT_CHARS, 1);
  const customisedCount = PROMPTS.filter((spec) => typeof config[spec.key] === "string").length;

  return (
    <div className="settings-scroll prompts-settings">
      <div className="prompts-page">
        <section className={`prompts-hero ${customisedCount > 0 ? "live" : ""}`} style={step(0)} aria-label="Prompts overview">
          <PromptStage live={customisedCount > 0} />
          <div className="prompts-hero-text">
            <span className={`prompts-pill ${customisedCount > 0 ? "live" : ""}`}>
              <i />
              {customisedCount === 0 ? "Built-in" : `${customisedCount} customised`}
            </span>
            <h3>Shape how {agentName} thinks</h3>
            <p>
              Every chat starts from these three prompts, tuned for WackCode. Customising them is optional: edit any of them, and
              restore the built-in text whenever you like. Changes apply from the next message, including chats that are already
              planning.
            </p>
          </div>
        </section>
        {unrestrictedPlanning && <p className="execution-policy-warning">Planning read-only restrictions are off in Settings › Tools. The app adds current access guidance after these instructions; plan review and approval still apply.</p>}
        {PROMPTS.map((spec, index) => {
          const customized = typeof config[spec.key] === "string";
          const editing = editingKey === spec.key;
          const effective = config[spec.key] ?? spec.defaultText;
          const lineCount = effective.split("\n").length;
          const clampable = lineCount > CLAMP_LINES || effective.length > CLAMP_CHARS;
          const open = expanded.includes(spec.key);
          return (
            <article
              className={`prompt-card ${spec.tone} ${customized ? "customized" : ""} ${editing ? "editing" : ""}`}
              key={spec.key}
              style={step(index + 1)}
              aria-label={spec.title}
            >
              <header className="prompt-card-head">
                <span className="prompt-mark" aria-hidden="true">
                  <Icon name={spec.icon} />
                </span>
                <div className="prompt-card-title">
                  <h4>
                    {spec.title} <span className={`prompt-badge ${customized ? "customized" : ""}`}>{customized ? "Customised" : "Default"}</span>
                  </h4>
                  <p>{spec.description}</p>
                  {!editing && (
                    <small className="prompt-meta">
                      {effective.length.toLocaleString()} characters · {lineCount.toLocaleString()} {lineCount === 1 ? "line" : "lines"}
                    </small>
                  )}
                </div>
                {!editing && (
                  <div className="prompt-card-actions">
                    {customized && (
                      <button type="button" className="text-button prompt-restore" disabled={busy} onClick={() => setRestoring(spec)}>
                        <Icon name="rewind" /> Restore default
                      </button>
                    )}
                    <button
                      type="button"
                      className="secondary-button compact"
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
                <motion.div
                  className="prompt-editor"
                  initial={reduceMotion ? { opacity: 0 } : { opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ type: "spring", stiffness: 420, damping: 34 }}
                >
                  <textarea
                    className="prompt-editor-input"
                    value={draft}
                    onChange={(event) => setDraft(event.target.value)}
                    aria-label={`${spec.title} text`}
                    rows={spec.key === "systemPrompt" ? 5 : 16}
                    spellCheck={false}
                  />
                  <div className="prompt-editor-actions">
                    <div
                      className={`prompt-meter ${overLimit ? "over" : fill >= 0.9 ? "warn" : ""}`}
                      role="meter"
                      aria-label={`${spec.title} length`}
                      aria-valuemin={0}
                      aria-valuemax={MAX_PROMPT_CHARS}
                      aria-valuenow={Math.min(draft.length, MAX_PROMPT_CHARS)}
                    >
                      <i style={{ "--fill": fill } as React.CSSProperties} />
                    </div>
                    <small className={`prompt-count ${overLimit ? "over" : ""}`}>
                      {draft.length.toLocaleString()} / {MAX_PROMPT_CHARS.toLocaleString()} characters
                    </small>
                    <span className="prompt-editor-spacer" />
                    <button type="button" className="secondary-button compact" disabled={busy} onClick={() => setEditingKey(undefined)}>
                      Cancel
                    </button>
                    <button type="button" className="primary-button compact" disabled={busy || blank || overLimit} onClick={() => void save(spec)}>
                      Save
                    </button>
                  </div>
                  {spec.note && (
                    <p className="prompt-note">
                      <Icon name="lock" />
                      <span>{spec.note}</span>
                    </p>
                  )}
                </motion.div>
              ) : (
                <motion.div
                  className="prompt-preview"
                  initial={reduceMotion ? false : { opacity: 0 }}
                  animate={{ opacity: 1 }}
                  transition={{ duration: 0.18 }}
                >
                  <pre
                    className={`prompt-view ${clampable && !open ? "clamped" : ""}`}
                    {...(open ? { tabIndex: 0, role: "region", "aria-label": `${spec.title} prompt text` } : {})}
                  >
                    {effective}
                  </pre>
                  {clampable && (
                    <button
                      type="button"
                      className="prompt-expand"
                      aria-expanded={open}
                      onClick={() => setExpanded((keys) => (open ? keys.filter((key) => key !== spec.key) : [...keys, spec.key]))}
                    >
                      <Icon name="chevron" /> {open ? "Show less" : "Show full text"}
                    </button>
                  )}
                </motion.div>
              )}
            </article>
          );
        })}
        {error && <div className="error-banner">{error}</div>}
      </div>
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
