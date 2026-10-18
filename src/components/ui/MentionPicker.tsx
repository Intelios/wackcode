/**
 * The `@` picker both composers share: workspace files (Code) and referenced chats (Code and
 * Chat mode), in one listbox. Presentational only — the composer owns the draft, the active
 * row and the insertion; this just draws what it is given and reports a pick.
 *
 * Chats always follow the files, under their own heading, so the file ranking stays exactly as
 * it was (`rankSuggestions` in `chat-refs.ts`).
 */
import { Fragment, type ReactNode } from "react";
import type { MentionSuggestion } from "../../mention-utils";
import { Icon } from "../Icons";

/** The keys both pickers answer to, under their lists. Decoration: the listbox's options carry the semantics. */
export const PICKER_KEYS = (
  <div className="slash-picker-foot" aria-hidden="true">
    <kbd>↑</kbd><kbd>↓</kbd><span>move</span><kbd>⏎</kbd><span>pick</span><kbd>⎋</kbd><span>close</span>
  </div>
);

interface MentionPickerProps {
  /** The listbox's id: the composer's textarea points `aria-controls` at it. */
  id: string;
  /** What the listbox is called, for screen readers ("Files", "Chats"). */
  label: string;
  suggestions: MentionSuggestion[];
  /** The row the arrow keys are on. */
  index: number;
  onPick: (entry: MentionSuggestion) => void;
  /** Shown instead of the list when nothing matches. */
  empty: ReactNode;
  /** Loading or error notes above the list; chats still list when it has any. */
  notice?: ReactNode;
  /** A note under the list, e.g. the file list's truncation. */
  footnote?: ReactNode;
}

export function MentionPicker({ id, label, suggestions, index, onPick, empty, notice, footnote }: MentionPickerProps) {
  return (
    <div id={id} className="slash-picker mention-picker" role="listbox" aria-label={label}>
      {notice}
      {suggestions.length === 0
        ? <div className="slash-picker-status">{empty}</div>
        : suggestions.map((entry, position) => {
          const row = `mention-option-${position}`;
          const selected = position === index;
          const heading = entry.kind === "chat" && suggestions[position - 1]?.kind !== "chat"
            ? <div className="mention-group" role="presentation">Chats</div>
            : null;
          return (
            <Fragment key={entry.kind === "chat" ? `chat:${entry.id}` : `file:${entry.path}`}>
              {heading}
              {entry.kind === "chat"
                ? (
                  <button id={row} type="button" role="option" aria-selected={selected}
                    className={`slash-option mention-option chat-option${selected ? " selected" : ""}`}
                    onMouseDown={(event) => event.preventDefault()} onClick={() => onPick(entry)}>
                    <span className="slash-mark" aria-hidden="true"><Icon name="comment" /></span>
                    <strong>{entry.name}</strong><span>{entry.label}</span>
                  </button>
                )
                : (
                  <button id={row} type="button" role="option" aria-selected={selected}
                    className={`slash-option mention-option${entry.directory ? " folder" : ""}${selected ? " selected" : ""}`}
                    onMouseDown={(event) => event.preventDefault()} onClick={() => onPick(entry)}>
                    <span className="slash-mark" aria-hidden="true"><Icon name={entry.directory ? "folder" : "file"} /></span>
                    <strong>{basename(entry.path)}</strong><span>{dirname(entry.path)}</span>
                  </button>
                )}
            </Fragment>
          );
        })}
      {footnote}
      {suggestions.length > 0 && PICKER_KEYS}
    </div>
  );
}

/** The row's name: the last path segment, with a folder's trailing slash (it is a folder). */
function basename(path: string): string {
  return path.slice(path.replace(/\/$/, "").lastIndexOf("/") + 1);
}

function dirname(path: string): string {
  const bare = path.replace(/\/$/, "");
  return bare.slice(0, bare.lastIndexOf("/") + 1);
}
