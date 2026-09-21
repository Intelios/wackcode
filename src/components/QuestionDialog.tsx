import { useRef, useState } from "react";
import type { AskQuestion, ExtensionUIRequest, QuestionAnswer } from "../types";
import { Icon } from "./Icons";

type QuestionsRequest = Extract<ExtensionUIRequest, { method: "questions" }>;

interface Props {
  request: QuestionsRequest;
  onRespond: (response: { answers?: QuestionAnswer[]; cancelled?: true }) => void;
}

interface Draft {
  /** Chosen option labels. */
  selected: string[];
  /** Free-form "Other" answer; non-empty means it replaces the preset choices. */
  custom: string;
}

function isAnswered(draft: Draft | undefined): boolean {
  return Boolean(draft && (draft.selected.length > 0 || draft.custom.trim()));
}

/**
 * The questionnaire raised by the built-in ask_user_question tool: one card per question,
 * preset options with descriptions, and a free-form "Other". Rendered inline above the
 * composer — the extension is blocked inside the run awaiting a reply, so cancelling always
 * sends an explicit `cancelled`.
 */
export function QuestionCard({ request, onRespond }: Props) {
  const questions = request.questions;
  const [tab, setTab] = useState(0);
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const otherRef = useRef<HTMLTextAreaElement>(null);
  const question = questions[Math.min(tab, questions.length - 1)]!;
  const draft = drafts[question.id] ?? { selected: [], custom: "" };
  const allAnswered = questions.every((entry) => isAnswered(drafts[entry.id]));
  const last = tab === questions.length - 1;

  function update(patch: Partial<Draft>) {
    setDrafts((current) => {
      const base: Draft = current[question.id] ?? { selected: [], custom: "" };
      return { ...current, [question.id]: { ...base, ...patch } };
    });
  }

  function pick(label: string) {
    if (question.multiSelect) {
      const selected = draft.selected.includes(label)
        ? draft.selected.filter((entry) => entry !== label)
        : [...draft.selected, label];
      update({ selected });
      return;
    }
    update({ selected: [label], custom: "" });
    if (!last) setTab(tab + 1);
  }

  function submit() {
    onRespond({
      answers: questions.map((entry) => {
        const answer = drafts[entry.id];
        return {
          questionId: entry.id,
          selected: answer?.custom.trim() ? [] : (answer?.selected ?? []),
          ...(answer?.custom.trim() ? { custom: answer.custom.trim() } : {})
        };
      })
    });
  }

  return (
    <div className="inline-dialog-card question-card" role="region" aria-label="Questions">
      <span className="eyebrow">Question</span>

      {questions.length > 1 && (
        <div className="question-tabs" role="tablist">
          {questions.map((entry, index) => (
            <button
              key={entry.id}
              type="button"
              role="tab"
              aria-selected={index === tab}
              className={`question-tab ${index === tab ? "active" : ""}`}
              onClick={() => setTab(index)}
            >
              {isAnswered(drafts[entry.id]) && <Icon name="check" className="question-tab-check" />}
              {entry.header}
            </button>
          ))}
        </div>
      )}

      <h3 className="question-title">{question.question}</h3>

      <div className="extension-options" role={question.multiSelect ? "group" : "radiogroup"} aria-label={question.question}>
        {question.options.map((option) => {
          const selected = draft.selected.includes(option.label);
          return (
            <button
              type="button"
              key={option.label}
              className={`question-option ${selected ? "selected" : ""}`}
              onClick={() => pick(option.label)}
            >
              <span className={`question-marker ${question.multiSelect ? "box" : "dot"}`}>{selected && <Icon name="check" />}</span>
              <span className="question-option-text">
                <span className="question-option-label">{option.label}</span>
                <span className="question-option-desc">{option.description}</span>
              </span>
            </button>
          );
        })}
        <div className={`question-option other ${draft.custom.trim() ? "selected" : ""}`}>
          <span className="question-marker dot">{draft.custom.trim() ? <Icon name="check" /> : null}</span>
          <textarea
            ref={otherRef}
            className="question-other"
            placeholder="Other — type a custom answer"
            rows={2}
            value={draft.custom}
            onFocus={() => { if (draft.selected.length) update({ selected: [] }); }}
            onChange={(event) => update({ custom: event.target.value, selected: [] })}
          />
        </div>
      </div>

      <div className="confirm-actions">
        <button type="button" className="secondary-button" onClick={() => onRespond({ cancelled: true })}>Cancel</button>
        {tab > 0 && (
          <button type="button" className="secondary-button" onClick={() => setTab(tab - 1)}>Back</button>
        )}
        {last ? (
          <button type="button" className="primary-button" disabled={!allAnswered} onClick={submit}>Submit</button>
        ) : (
          <button type="button" className="primary-button" onClick={() => setTab(tab + 1)}>Next</button>
        )}
      </div>
    </div>
  );
}
