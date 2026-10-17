/**
 * Shared by the Code composer and Chat mode's: what a large paste became, and the way back to
 * plain text. Both render it inside their own notice wrapper (`.attachment-notice` or
 * `.chat-composer-notice`), so this is only the sentence and its action.
 */
import { textBytes, type FileAttachment } from "../../attachment-utils";

interface PasteNoticeProps {
  file: FileAttachment;
  onInsert: () => void;
  disabled?: boolean;
}

export function PasteNotice({ file, onInsert, disabled }: PasteNoticeProps) {
  const lines = file.text.split("\n").length;
  const kb = Math.round(textBytes(file.text) / 1024);
  return (
    <>
      <span>Pasted {lines.toLocaleString()} {lines === 1 ? "line" : "lines"} ({kb} KB) as <strong>{file.name}</strong>.</span>{" "}
      <button type="button" className="paste-notice-action" onClick={onInsert} disabled={disabled}>Insert as text</button>
    </>
  );
}
