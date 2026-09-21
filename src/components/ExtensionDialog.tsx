import { useEffect, useRef, useState } from "react";
import type { ExtensionUIRequest } from "../types";

interface Props {
  /** Structured `questions` requests have their own card (QuestionCard). */
  request: Exclude<ExtensionUIRequest, { method: "questions" }>;
  onRespond: (response: { value?: string; confirmed?: boolean; cancelled?: true }) => void;
}

/**
 * Renders a question an installed extension asked (select, confirm, input, editor).
 * Displayed inline above the composer — the extension is blocked awaiting a reply,
 * usually inside a tool call, so cancelling always sends an explicit `cancelled`.
 */
export function ExtensionCard({ request, onRespond }: Props) {
  const [text, setText] = useState(request.method === "editor" ? request.prefill ?? "" : "");
  const [choice, setChoice] = useState(request.method === "select" ? request.options[0] ?? "" : "");
  const firstField = useRef<HTMLInputElement | HTMLTextAreaElement>(null);

  useEffect(() => { firstField.current?.focus(); }, []);

  function submit() {
    if (request.method === "confirm") onRespond({ confirmed: true });
    else if (request.method === "select") onRespond({ value: choice });
    else onRespond({ value: text });
  }

  return (
    <div className="inline-dialog-card extension-card" role="region" aria-label={request.title}>
      <span className="eyebrow">Extension</span>
      <h3>{request.title}</h3>

      {request.method === "confirm" && <p className="extension-dialog-body">{request.message}</p>}

      {request.method === "select" && (
        <div className="extension-options" role="radiogroup" aria-label={request.title}>
          {request.options.map((option) => (
            <label className={`extension-option ${choice === option ? "selected" : ""}`} key={option}>
              <input
                type="radio"
                name="extension-option"
                value={option}
                checked={choice === option}
                onChange={() => setChoice(option)}
              />
              <span>{option}</span>
            </label>
          ))}
        </div>
      )}

      {request.method === "input" && (
        <input
          ref={firstField as React.RefObject<HTMLInputElement>}
          value={text}
          placeholder={request.placeholder}
          aria-label={request.title}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter") submit(); }}
        />
      )}

      {request.method === "editor" && (
        <textarea
          ref={firstField as React.RefObject<HTMLTextAreaElement>}
          className="extension-editor"
          value={text}
          aria-label={request.title}
          rows={10}
          onChange={(event) => setText(event.target.value)}
        />
      )}

      <div className="confirm-actions">
        <button type="button" className="secondary-button" onClick={() => onRespond({ cancelled: true })}>Cancel</button>
        <button type="button" className="primary-button" onClick={submit}>
          {request.method === "confirm" ? "Confirm" : "Submit"}
        </button>
      </div>
    </div>
  );
}
