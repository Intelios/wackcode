import { createContext, useCallback, useContext, useState } from "react";
import { highlightDiffLines, languageForPath } from "../highlight";
import type { NormalizedBlock } from "../types";
import { mcpToolParts, summarizeTool } from "../tool-utils";
import { Icon, type IconName } from "./Icons";
import { ImageLightbox } from "./ui/ImageLightbox";

/**
 * Loads the original of a screenshot tool result's image (`api.toolImage` for the open chat),
 * as a URL. Without a provider, the lightbox shows the preview alone.
 */
export const ToolImageSource = createContext<((toolCallId: string, index: number) => Promise<string | undefined>) | undefined>(undefined);

const TOOL_ICONS: Record<string, IconName> = {
  read: "file",
  edit: "pencil",
  write: "file",
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
  const lines = diff.split("\n");
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

function tail(text: string, maxLines: number): string {
  const lines = text.split("\n");
  if (lines.length <= maxLines) return text;
  return `… ${lines.length - maxLines} earlier lines\n${lines.slice(-maxLines).join("\n")}`;
}

function ToolDetail({ call, result }: { call: NormalizedBlock; result?: NormalizedBlock }) {
  const summary = summarizeTool(call, result);
  const args = (call.arguments ?? {}) as Record<string, unknown>;
  const details = (result?.details ?? {}) as Record<string, unknown>;

  if (summary.kind === "edit" && typeof details.diff === "string" && details.diff) {
    return <DiffLines diff={details.diff} path={typeof args.path === "string" ? args.path : undefined} />;
  }
  if (summary.kind === "bash") {
    return (
      <div className="tool-detail">
        <pre className="tool-command">$ {String(args.command ?? "")}</pre>
        {result?.text && <pre>{tail(result.text, 60)}</pre>}
      </div>
    );
  }
  if (summary.kind === "write" && typeof args.content === "string") {
    return <div className="tool-detail"><pre>{tail(args.content, 80)}</pre></div>;
  }
  if (summary.kind === "search") {
    return <div className="tool-detail"><pre>{result?.text ? tail(result.text, 60) : "No matches"}</pre></div>;
  }
  return (
    <div className="tool-detail">
      <pre>{JSON.stringify(args, null, 2)}</pre>
      {result?.text && <pre>{tail(result.text, 60)}</pre>}
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
  const [open, setOpen] = useState(false);
  const summary = summarizeTool(call, result);
  const failed = result?.isError === true;
  const pending = running && !result;
  const shownResult = result ?? (pending && liveText ? { type: "tool-result" as const, text: liveText } : undefined);
  const expandable = Boolean(shownResult?.text || (result?.details as Record<string, unknown> | undefined)?.diff || summary.kind === "write" || summary.kind === "other");

  return (
    <div className={`tool-row ${open ? "open" : ""} ${failed ? "error" : ""}`}>
      <button type="button" className="tool-row-head" onClick={() => expandable && setOpen((value) => !value)} disabled={!expandable} aria-expanded={open}>
        <Icon name={TOOL_ICONS[call.toolName ?? ""] ?? (mcpToolParts(call.toolName ?? "") ? "plug" : "terminal")} className="tool-row-icon" />
        <span className="tool-row-verb">{pending ? summary.activeVerb : summary.doneVerb}</span>
        {summary.subject && <code className="tool-row-subject" title={summary.subject}>{summary.subject}</code>}
        {(summary.additions || summary.deletions) ? (
          <span className="tool-row-stats">
            {summary.additions ? <em className="add">+{summary.additions}</em> : null}
            {summary.deletions ? <em className="del">−{summary.deletions}</em> : null}
          </span>
        ) : null}
        {failed && <span className="tool-row-failed">failed</span>}
        <span className="tool-row-status">
          {pending && <span className="tool-spinner" aria-label="Running" />}
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
  const [open, setOpen] = useState(false);
  return (
    <div className={`tool-row ${block.isError ? "error" : ""}`}>
      <button type="button" className="tool-row-head" onClick={() => setOpen((value) => !value)} aria-expanded={open}>
        <Icon name="terminal" className="tool-row-icon" />
        <span className="tool-row-verb">{block.toolName ? `${block.toolName} result` : "Tool result"}</span>
        {block.isError && <span className="tool-row-failed">failed</span>}
        <span className="tool-row-status"><Icon name="chevron" className="tool-chevron" /></span>
      </button>
      {open && <div className="tool-detail"><pre>{tail(block.text ?? "", 60)}</pre></div>}
    </div>
  );
}
