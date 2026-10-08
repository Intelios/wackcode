import { createContext, useCallback, useContext, useId, useMemo, useState } from "react";
import { highlightDiffLines, languageForPath } from "../highlight";
import { editArgumentDiff, focusedEditDiff } from "../edit-preview";
import type { NormalizedBlock } from "../types";
import { mcpToolParts, splitPathSubject, summarizeTool } from "../tool-utils";
import { Icon, type IconName } from "./Icons";
import { OrbitSpinner } from "./OrbitSpinner";
import { QuillMark } from "./QuillMark";
import { RollingNumber } from "./RollingNumber";
import { ImageLightbox } from "./ui/ImageLightbox";
import { CopyButton } from "./ui/CopyButton";

/**
 * Loads the original of a screenshot tool result's image (`api.toolImage` for the open chat),
 * as a URL. Without a provider, the lightbox shows the preview alone.
 */
export const ToolImageSource = createContext<((toolCallId: string, index: number) => Promise<string | undefined>) | undefined>(undefined);

/** Stable tool-call keys retain disclosure choices when only one tab is mounted. */
export const ToolExpansion = createContext<{ scope: string; keys: Set<string> } | undefined>(undefined);
function useToolDisclosure(entryKey?: string) {
  const store = useContext(ToolExpansion);
  const expanded = store?.keys;
  const key = entryKey === undefined ? undefined : JSON.stringify([store?.scope ?? "", entryKey]);
  const [open, setOpen] = useState(() => key !== undefined && expanded?.has(key) === true);
  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (key === undefined) return;
    if (next) expanded?.add(key); else expanded?.delete(key);
  };
  return [open, toggle] as const;
}

const TOOL_ICONS: Record<string, IconName> = {
  read: "file",
  bash: "terminal",
  grep: "search",
  find: "search",
  ls: "search",
  ask_user_question: "question",
  plan_mode_complete: "brain",
  todo: "checklist",
  subagent: "agents",
  computer_apps: "cursor",
  computer_open: "cursor",
  computer_snapshot: "cursor",
  computer_screenshot: "image",
  computer_act: "cursor",
  memory_save: "memory",
  memory_recall: "memory",
  memory_forget: "memory",
  browser_screenshot: "image"
};

/** A screenshot result's previews, always visible under its row; each opens full size. */
function ToolImages({ call, result }: { call: NormalizedBlock; result: NormalizedBlock }) {
  const load = useContext(ToolImageSource);
  const [shown, setShown] = useState<number>();
  const images = result.images ?? [];
  const subject = summarizeTool(call, result).subject || "the app";
  const loadShown = useCallback(
    () => (load && call.toolCallId && shown !== undefined ? load(call.toolCallId, shown) : Promise.resolve(undefined)),
    [load, call.toolCallId, shown]
  );
  const preview = shown !== undefined ? images[shown]?.thumbnail : undefined;
  return (
    <div className="tool-images">
      {images.map((image, index) => image.thumbnail ? (
        <button key={image.imageId} type="button" className="tool-image" onClick={() => setShown(index)} aria-label={`Open screenshot of ${subject}`}>
          <img src={image.thumbnail} alt={`Screenshot of ${subject}`} />
        </button>
      ) : (
        <span key={image.imageId} className="tool-image pending" aria-label="Preparing screenshot preview" />
      ))}
      {preview && <ImageLightbox preview={preview} load={loadShown} alt={`Screenshot of ${subject}`} onClose={() => setShown(undefined)} />}
    </div>
  );
}

function DiffLines({ diff, path }: { diff: string; path?: string }) {
  const lines = useMemo(() => focusedEditDiff(diff).split("\n"), [diff]);
  const classes = lines.map((line) =>
    line.startsWith("+") && !line.startsWith("+++") ? "addition"
      : line.startsWith("-") && !line.startsWith("---") ? "deletion"
      : line.startsWith("@@") ? "hunk"
      : line.startsWith("diff ") || line.startsWith("# ") ? "heading" : "");
  // The edit's own file language highlights the code lines; hunk and heading lines stay plain.
  const nodes = highlightDiffLines(lines, classes.map((name) => (name === "hunk" || name === "heading" ? "meta" : "code")), languageForPath(path));
  return (
    <pre className="diff-view tool-diff">
      {lines.map((line, index) => (
        <span className={classes[index]} key={index}>{nodes[index]}{"\n"}</span>
      ))}
    </pre>
  );
}

/** Preview limits only affect rendering: expansion and copying always use the source text. */
function ToolText({ text, label = "output", maxLines = 60, command = false, copy = false, expansionKey }: {
  text: string; label?: string; maxLines?: number; command?: boolean; copy?: boolean; expansionKey?: string;
}) {
  const [all, toggleAll] = useToolDisclosure(expansionKey ? `${expansionKey}:${label}` : undefined);
  const id = useId();
  const lines = useMemo(() => text.split("\n"), [text]);
  const truncated = lines.length > maxLines;
  const preview = truncated && !all ? lines.slice(-maxLines).join("\n") : text;
  // The toolbar exists only for the truncation controls, and copy (where offered at all) rides
  // in it; untruncated text has no row at all, so copy floats over the corner on hover instead.
  return (
    <div className="tool-text">
      {truncated ? (
        <div className="tool-text-toolbar">
          <span className="tool-text-count">{all ? `All ${lines.length} lines` : `Last ${maxLines} of ${lines.length} lines`}</span>
          <button type="button" className="text-button" aria-expanded={all} aria-controls={id} onClick={toggleAll}>
            {all ? "Show less" : "Show all"}
          </button>
          {copy && <CopyButton text={text} label={`Copy ${label}`} />}
        </div>
      ) : copy ? (
        <CopyButton text={text} label={`Copy ${label}`} floating />
      ) : null}
      <pre id={id} className={command ? "tool-command" : undefined}>{command ? "$ " : ""}{preview}</pre>
    </div>
  );
}

function ToolDetail({ call, result }: { call: NormalizedBlock; result?: NormalizedBlock }) {
  const summary = summarizeTool(call, result);
  const args = (call.arguments ?? {}) as Record<string, unknown>;
  const details = (result?.details ?? {}) as Record<string, unknown>;

  const editDiff = useMemo(() => summary.kind === "edit" && !result?.isError
    ? (typeof details.diff === "string" && details.diff) || editArgumentDiff(args)
    : undefined, [summary.kind, result?.isError, details.diff, args]);

  if (editDiff) {
    return (
      <div className="tool-detail">
        <CopyButton text={editDiff} label="Copy diff" floating />
        <DiffLines diff={editDiff} path={typeof args.path === "string" ? args.path : undefined} />
      </div>
    );
  }
  if (summary.kind === "bash") {
    return (
      <div className="tool-detail">
        {/* Copy follows what the agent authored — the command — never the output it got back. */}
        <ToolText text={String(args.command ?? "")} label="command" command copy expansionKey={call.toolCallId} />
        {result?.text && <ToolText text={result.text} expansionKey={call.toolCallId} />}
      </div>
    );
  }
  if (summary.kind === "write" && typeof args.content === "string") {
    return <div className="tool-detail"><ToolText text={args.content} label="file content" maxLines={80} copy expansionKey={call.toolCallId} /></div>;
  }
  if (summary.kind === "search") {
    return <div className="tool-detail">{result?.text ? <ToolText text={result.text} expansionKey={call.toolCallId} /> : <pre>No matches</pre>}</div>;
  }
  return (
    <div className="tool-detail">
      <ToolText text={JSON.stringify(args, null, 2)} label="arguments" expansionKey={call.toolCallId} />
      {result?.text && <ToolText text={result.text} expansionKey={call.toolCallId} />}
    </div>
  );
}

interface ToolRowProps {
  call: NormalizedBlock;
  result?: NormalizedBlock;
  liveText?: string;
  running?: boolean;
}

export function ToolRow({ call, result, liveText, running }: ToolRowProps) {
  const [open, toggle] = useToolDisclosure(call.toolCallId ? `tool:${call.toolCallId}` : undefined);
  const summary = summarizeTool(call, result);
  const failed = result?.isError === true;
  const pending = running && !result;
  const shownResult = result ?? (pending && liveText ? { type: "tool-result" as const, text: liveText } : undefined);
  /** Only real file paths get a dir/name split; commands, patterns and URLs are left whole. */
  const pathParts = summary.kind === "read" || summary.kind === "edit" || summary.kind === "write" ? splitPathSubject(summary.subject) : undefined;
  const expandable = Boolean(shownResult?.text || (result?.details as Record<string, unknown> | undefined)?.diff || summary.kind === "write" || summary.kind === "other");

  return (
    <div className={`tool-row ${open ? "open" : ""} ${failed ? "error" : ""}`}>
      <button type="button" className="tool-row-head" onClick={() => expandable && toggle()} disabled={!expandable} aria-expanded={open}>
        {/* Edits and writes get the quill mark — it writes while the tool runs, then rests —
            every other row keeps its static tool icon. */}
        {summary.kind === "edit" || summary.kind === "write" ? (
          <QuillMark live={Boolean(pending)} className="tool-row-icon" />
        ) : (
          <Icon name={TOOL_ICONS[call.toolName ?? ""] ?? (mcpToolParts(call.toolName ?? "") ? "plug" : "terminal")} className="tool-row-icon" />
        )}
        <span className="tool-row-verb">{pending ? summary.activeVerb : summary.doneVerb}</span>
        {summary.subject && (
          <code className={`tool-row-subject ${pathParts?.dir ? "has-dir" : ""}`} title={summary.subject}>
            {pathParts?.dir && <span className="tool-row-subject-dir">{pathParts.dir}</span>}
            <span className="tool-row-subject-name">{pathParts ? pathParts.name : summary.subject}</span>
          </code>
        )}
        {(summary.additions || summary.deletions) ? (
          <span className="tool-row-stats">
            {summary.additions ? <em className="add"><RollingNumber prefix="+" value={summary.additions} /></em> : null}
            {summary.deletions ? <em className="del"><RollingNumber prefix="−" value={summary.deletions} /></em> : null}
          </span>
        ) : null}
        {failed && <span className="tool-row-failed">failed</span>}
        <span className="tool-row-status">
          <OrbitSpinner active={Boolean(pending)} failed={failed} />
          {expandable && <Icon name="chevron" className="tool-chevron" />}
        </span>
      </button>
      {result?.images?.length ? <ToolImages call={call} result={result} /> : null}
      {open && expandable && <ToolDetail call={call} result={shownResult} />}
    </div>
  );
}

/** Fallback row for a tool result that never matched a call (rare). */
export function OrphanResult({ block }: { block: NormalizedBlock }) {
  const [open, toggle] = useToolDisclosure(block.toolCallId ? `orphan:${block.toolCallId}` : undefined);
  return (
    <div className={`tool-row ${open ? "open" : ""} ${block.isError ? "error" : ""}`}>
      <button type="button" className="tool-row-head" onClick={toggle} aria-expanded={open}>
        <Icon name="terminal" className="tool-row-icon" />
        <span className="tool-row-verb">{block.toolName ? `${block.toolName} result` : "Tool result"}</span>
        {block.isError && <span className="tool-row-failed">failed</span>}
        <span className="tool-row-status"><Icon name="chevron" className="tool-chevron" /></span>
      </button>
      {open && <div className="tool-detail"><ToolText text={block.text ?? ""} /></div>}
    </div>
  );
}
