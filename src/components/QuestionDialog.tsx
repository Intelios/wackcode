import { useId, useLayoutEffect, useRef, useState, type ComponentProps } from "react";
import { AnimatePresence, motion, useIsPresent, useReducedMotion } from "motion/react";
import type { ExtensionUIRequest, QuestionAnswer } from "../types";
import { DuckMark } from "./DuckMark";
import { Icon } from "./Icons";

type QuestionsRequest = Extract<ExtensionUIRequest, { method: "questions" }>;

interface Props {
  request: QuestionsRequest;
  onRespond: (response: { answers?: QuestionAnswer[]; cancelled?: true; wrapUp?: true }) => void;
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

function growOther(node: HTMLTextAreaElement | null) {
  if (!node) return;
  node.style.height = "auto";
  const maximum = parseFloat(getComputedStyle(node).lineHeight) * 4;
  node.style.height = `${Math.min(node.scrollHeight, maximum)}px`;
  node.style.overflowY = node.scrollHeight > maximum ? "auto" : "hidden";
}

function QuestionStage(props: ComponentProps<typeof motion.div>) {
  const present = useIsPresent();
  return <motion.div {...props} inert={!present} aria-hidden={!present || undefined}
    style={!present ? { position: "absolute", top: 3, left: 3, right: 3 } : undefined} />;
}

/**
 * Inline ask_user_question interview. Picks never advance automatically. Custom answers
 * replace presets, including multi-select; every terminal action is single-flight because
 * the card remains mounted during its parent's AnimatePresence exit.
 */
export function QuestionCard({ request, onRespond }: Props) {
  const questions = request.questions;
  const [tab, setTab] = useState(0);
  const [direction, setDirection] = useState(1);
  const [nod, setNod] = useState(0);
  const [bubbles, setBubbles] = useState(0);
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [sent, setSent] = useState(false);
  const responded = useRef(false);
  const focusHeading = useRef(false);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const otherRef = useRef<HTMLTextAreaElement>(null);
  const tabsRef = useRef<(HTMLButtonElement | null)[]>([]);
  const present = useIsPresent();
  const reduced = useReducedMotion();
  const id = useId();
  const question = questions[Math.min(tab, questions.length - 1)]!;
  const draft = drafts[question.id] ?? { selected: [], custom: "" };
  const allAnswered = questions.every((entry) => isAnswered(drafts[entry.id]));
  const last = tab === questions.length - 1;

  useLayoutEffect(() => { growOther(otherRef.current); }, [draft.custom, tab]);

  function update(patch: Partial<Draft>) {
    if (!present || responded.current) return;
    setDrafts((current) => {
      const base: Draft = current[question.id] ?? { selected: [], custom: "" };
      return { ...current, [question.id]: { ...base, ...patch } };
    });
  }

  function pick(label: string) {
    if (!present || responded.current) return;
    const selected = question.multiSelect
      ? draft.selected.includes(label)
        ? draft.selected.filter((entry) => entry !== label)
        : [...draft.selected, label]
      : [label];
    update({ selected, custom: "" });
    setNod((value) => value + 1);
  }

  function navigate(index: number, keyboardTab = false) {
    if (!present || responded.current || index === tab) return;
    setDirection(index > tab ? 1 : -1);
    if (index > tab) setBubbles((value) => value + 1);
    // Arrow navigation keeps focus on the tab rail; other navigation reads the new heading.
    focusHeading.current = !keyboardTab;
    setTab(index);
    if (keyboardTab) tabsRef.current[index]?.focus();
  }

  function respond(response: Parameters<Props["onRespond"]>[0]) {
    if (!present || responded.current) return;
    responded.current = true;
    setSent(true);
    onRespond(response);
  }

  function submit() {
    if (!allAnswered) return;
    respond({
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
    <motion.div
      className={`inline-dialog-card question-card${request.offerWrapUp ? " question-ultra" : ""}`}
      role="region" aria-label="Questions" tabIndex={-1} inert={!present || sent}
      layout="size"
      initial={reduced ? false : { opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={reduced ? { opacity: 0 } : { opacity: 0, scaleY: 0.72, y: -18, rotateX: 12 }}
      transition={{ duration: reduced ? 0 : 0.26, ease: [0.32, 0, 0.67, 0] }}
      style={{ transformOrigin: "top center" }}
      onKeyDown={(event) => {
        if (event.defaultPrevented || event.nativeEvent.isComposing || event.repeat || !present || responded.current) return;
        const target = event.target as HTMLElement;
        if (target.closest("textarea, input, select, [contenteditable=true]")) return;
        if (!event.metaKey && !event.ctrlKey && !event.altKey && /^[1-4]$/.test(event.key)) {
          const option = question.options[Number(event.key) - 1];
          if (option) { event.preventDefault(); pick(option.label); }
        } else if (event.key === "Enter" && event.metaKey && allAnswered) {
          event.preventDefault(); submit();
        } else if (event.key === "Enter" && !event.metaKey && !event.ctrlKey && !event.altKey &&
          (target === event.currentTarget || target === headingRef.current)) {
          event.preventDefault();
          if (last) submit();
          else if (isAnswered(draft)) navigate(tab + 1);
        }
      }}
    >
      <div className="question-head">
        <div className="question-duck-tile" aria-hidden="true">
          <div className={!isAnswered(draft) ? "question-duck-idle" : ""}>
            <motion.div key={nod} className="question-duck"
              animate={!reduced && nod > 0 ? { rotate: [0, 12, -5, 0], y: [0, 3, 0] } : { rotate: 0, y: 0 }}
              transition={{ duration: 0.42 }}><DuckMark /></motion.div>
          </div>
          {bubbles > 0 && !reduced && <span key={bubbles} className="question-bubbles"><i /><i /><i /></span>}
        </div>
        <div className="question-head-copy">
          <span className="eyebrow">{request.offerWrapUp && <Icon name="flame" />}{request.offerWrapUp ? "Ultra Plan · Question" : "Question"}</span>
          <span className="question-active-header">{question.header}</span>
        </div>
        {questions.length > 1 && (
          <div className="question-tabs" role="tablist" aria-label="Questions">
            {questions.map((entry, index) => (
              <button key={entry.id} ref={(node) => { tabsRef.current[index] = node; }}
                title={entry.header} id={`${id}-tab-${index}`} type="button" role="tab"
                aria-label={`${entry.header} · Question ${index + 1} of ${questions.length}${isAnswered(drafts[entry.id]) ? " · Answered" : ""}`}
                aria-controls={`${id}-panel-${index}`} aria-selected={index === tab} tabIndex={index === tab ? 0 : -1}
                className={`question-tab ${index === tab ? "active" : ""} ${isAnswered(drafts[entry.id]) ? "answered" : ""}`}
                onClick={() => navigate(index)}
                onKeyDown={(event) => {
                  const next = event.key === "ArrowRight" ? (tab + 1) % questions.length
                    : event.key === "ArrowLeft" ? (tab - 1 + questions.length) % questions.length
                    : event.key === "Home" ? 0 : event.key === "End" ? questions.length - 1 : null;
                  if (next !== null) { event.preventDefault(); navigate(next, true); }
                }}
              >
                {isAnswered(drafts[entry.id]) ? <Icon name="check" className="question-tab-check" /> : <span>{index + 1}</span>}
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="question-stage-shell">
        <AnimatePresence initial={false} custom={direction}>
          <QuestionStage key={question.id} className="question-stage"
            id={`${id}-panel-${tab}`} role={questions.length > 1 ? "tabpanel" : undefined}
            aria-labelledby={questions.length > 1 ? `${id}-tab-${tab}` : undefined}
            custom={direction}
            variants={{
              enter: (side: number) => ({ opacity: 0, x: reduced ? 0 : side * 22 }),
              center: { opacity: 1, x: 0 },
              leave: (side: number) => ({ opacity: 0, x: reduced ? 0 : side * -16 })
            }}
            initial="enter" animate="center" exit="leave"
            transition={reduced ? { duration: 0 } : { type: "spring", stiffness: 390, damping: 32 }}
          >
            <h3 className="question-title" tabIndex={-1} ref={(node) => {
              if (node) headingRef.current = node;
              if (node && focusHeading.current) { node.focus(); focusHeading.current = false; }
            }}>{question.question}</h3>
            <div className="extension-options" role={question.multiSelect ? "group" : "radiogroup"} aria-label={question.question}>
              {question.options.map((option, index) => {
                const selected = draft.selected.includes(option.label);
                return (
                  <motion.button type="button" key={option.label}
                    role={question.multiSelect ? "checkbox" : "radio"} aria-checked={selected}
                    className={`question-option ${selected ? "selected" : ""}`} onClick={() => pick(option.label)}
                    initial={reduced || direction < 0 ? false : { opacity: 0, y: 7 }} animate={{ opacity: 1, y: 0 }}
                    transition={reduced ? { duration: 0 } : { type: "spring", stiffness: 420, damping: 30, delay: index * 0.035 }}
                  >
                    <span aria-hidden="true" className={`question-marker ${question.multiSelect ? "box" : "dot"}`}>{selected && <Icon name="check" />}</span>
                    <span className="question-option-text">
                      <span className="question-option-label">{option.label}</span>
                      <span className="question-option-desc">{option.description}</span>
                    </span>
                    {index < 4 && <kbd className="question-digit" aria-hidden="true">{index + 1}</kbd>}
                  </motion.button>
                );
              })}
              <div className={`question-option other ${draft.custom.trim() ? "selected" : ""}`}>
                <span aria-hidden="true" className="question-marker dot">{draft.custom.trim() ? <Icon name="check" /> : null}</span>
                <textarea ref={(node) => { if (node) otherRef.current = node; growOther(node); }}
                  className="question-other" aria-label="Other — type a custom answer"
                  placeholder="Other — type a custom answer" rows={1} value={draft.custom}
                  onFocus={() => { if (draft.selected.length) update({ selected: [] }); }}
                  onChange={(event) => update({ custom: event.target.value, selected: [] })}
                />
              </div>
            </div>
          </QuestionStage>
        </AnimatePresence>
      </div>

      <div className="confirm-actions">
        {request.offerWrapUp && (
          <button type="button" className="secondary-button question-wrap-up"
            title="Stop the interview — the agent submits its plan using its recommended answers"
            onClick={() => respond({ wrapUp: true })}>Write the plan now</button>
        )}
        <button type="button" className="secondary-button" onClick={() => respond({ cancelled: true })}>Cancel</button>
        {tab > 0 && <button type="button" className="secondary-button" onClick={() => navigate(tab - 1)}>Back</button>}
        {last ? (
          <button type="button" className="primary-button" disabled={!allAnswered} onClick={submit}>Submit</button>
        ) : (
          <button type="button" className="primary-button" disabled={!isAnswered(draft)} onClick={() => navigate(tab + 1)}>Next</button>
        )}
      </div>
    </motion.div>
  );
}
