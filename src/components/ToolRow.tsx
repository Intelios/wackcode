import { createContext, useCallback, useContext, useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { NormalizedBlock } from "../types";
import { SCREENSHOT_TOOL_NAMES, splitPathSubject, summarizeTool, toolPresentation, type ToolSummary } from "../tool-utils";
import { motionAllowed } from "../hooks/useSmoothText";
import { Icon } from "./Icons";
import { OrbitSpinner } from "./OrbitSpinner";
import { QuillMark } from "./QuillMark";
import { RollingNumber } from "./RollingNumber";
import { ToolDetail, ToolExpansion, ToolText, toolIcon, useToolDisclosure } from "./ToolDetail";
import { ImageLightbox } from "./ui/ImageLightbox";

export { ToolDetail, ToolExpansion };

/**
 * Loads the original of an image a tool result returned (`api.toolImage` for the open chat),
 * as a URL. Without a provider, the lightbox shows the preview alone.
 */
export const ToolImageSource = createContext<((toolCallId: string, index: number) => Promise<string | undefined>) | undefined>(undefined);

/** How a result's images are named to the user: a capture, a file that was read, or a tool's output. */
function imageWording(call: NormalizedBlock, result: NormalizedBlock): { noun: string; name: string; open: string; caption: string } {
  const summary = summarizeTool(call, result);
  if (SCREENSHOT_TOOL_NAMES.has(call.toolName ?? "")) {
    const subject = summary.subject || "the app";
    return { noun: "screenshot", name: `Screenshot of ${subject}`, open: `Open screenshot of ${subject}`, caption: subject };
  }
  if (summary.kind === "read" && summary.subject) {
    const file = splitPathSubject(summary.subject).name;
    return { noun: "image", name: file, open: `Open image ${file}`, caption: file };
  }
  const subject = summary.subject || summary.doneVerb;
  return { noun: "image", name: `Image from ${subject}`, open: `Open image from ${subject}`, caption: subject };
}

/**
 * The images a tool result returned, always visible under its row; each opens full size. An
 * image no preview could be made from shows nothing but keeps its position, which is what the
 * original is fetched by. `caption` names the source under each preview, for where the row
 * that would say so isn't on screen (a folded Explored group).
 */
export function ToolImages({ call, result, caption = false }: { call: NormalizedBlock; result: NormalizedBlock; caption?: boolean }) {
  const load = useContext(ToolImageSource);
  const [shown, setShown] = useState<number>();
  const images = result.images ?? [];
  const wording = imageWording(call, result);
  const loadShown = useCallback(
    () => (load && call.toolCallId && shown !== undefined ? load(call.toolCallId, shown) : Promise.resolve(undefined)),
    [load, call.toolCallId, shown]
  );
  const preview = shown !== undefined ? images[shown]?.thumbnail : undefined;
  if (images.every((image) => image.failed)) return null;
  return (
    <div className="tool-images">
      {images.map((image, index) => image.failed ? null : (
        <figure key={image.imageId} className="tool-image-figure">
          {image.thumbnail ? (
            <button type="button" className="tool-image" onClick={() => setShown(index)} aria-label={wording.open} title={caption ? undefined : wording.caption}>
              <img src={image.thumbnail} alt={wording.name} />
            </button>
          ) : (
            <span className="tool-image pending" aria-label={`Preparing ${wording.noun} preview`} />
          )}
          {caption && <figcaption className="tool-image-caption" title={wording.caption}>{wording.caption}</figcaption>}
        </figure>
      ))}
      {preview && <ImageLightbox preview={preview} load={loadShown} alt={wording.name} onClose={() => setShown(undefined)} />}
    </div>
  );
}

/**
 * Chat's springs (ChatActivity.tsx): chips pop in, and fly into an Explored head. The fly is
 * critically damped: an overshoot would squash the landing mark below its size and back.
 */
const POP = { type: "spring", stiffness: 520, damping: 28 } as const;
export const FLY = { type: "spring", stiffness: 380, damping: 36, mass: 0.8 } as const;
const SHEET = { duration: 0.24, ease: [0.33, 1, 0.68, 1] } as const;

/** True where transcript motion may play: not reduced, and a real browser (jsdom stays still). */
export function useTranscriptMotion(): boolean {
  const reduced = useReducedMotion() ?? false;
  return !reduced && motionAllowed();
}

/**
 * Calls this renderer has already drawn. A call pops in only the first time it is drawn while
 * running: history never pops, and the streamed copy being replaced by the saved message (a
 * remount under a different parent) doesn't pop it a second time.
 */
const drawnCalls = new Set<string>();
function usePopsIn(id: string | undefined, live: boolean): boolean {
  const [pops] = useState(() => {
    if (!id) return false;
    const fresh = !drawnCalls.has(id);
    drawnCalls.add(id);
    return fresh && live;
  });
  return pops;
}

/** One call's state as both shapes draw it. */
function useCallFace(call: NormalizedBlock, result: NormalizedBlock | undefined, liveText: string | undefined, running: boolean | undefined) {
  const summary = summarizeTool(call, result);
  const failed = result?.isError === true;
  const pending = Boolean(running && !result);
  const shownResult = result ?? (pending && liveText ? { type: "tool-result" as const, text: liveText } : undefined);
  const expandable = Boolean(shownResult?.text || (result?.details as Record<string, unknown> | undefined)?.diff || summary.kind === "write" || summary.kind === "other");
  return { summary, failed, pending, shownResult, expandable };
}

/** Verb, subject (directory dimmed so the file name survives truncation), +/− and failure. */
function CallWords({ summary, pending, failed }: { summary: ToolSummary; pending: boolean; failed: boolean }) {
  /** Only real file paths get a dir/name split; commands, patterns and URLs are left whole. */
  const pathParts = summary.kind === "read" || summary.kind === "edit" || summary.kind === "write" ? splitPathSubject(summary.subject) : undefined;
  return (
    <>
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
    </>
  );
}

/** A call drawn as a chip, inside a `ToolChips` strip. */
export interface ChipCall {
  call: NormalizedBlock;
  result?: NormalizedBlock;
  liveText?: string;
  running?: boolean;
  /** Shared-element id: set where this chip may fly into an Explored head (`explore-utils.ts` flyId). */
  flyId?: string;
}

function ToolChip({ entry, open, onToggle }: { entry: ChipCall; open: boolean; onToggle: () => void }) {
  const { call, result, liveText, running } = entry;
  const { summary, failed, pending, expandable } = useCallFace(call, result, liveText, running);
  const animate = useTranscriptMotion();
  const pops = usePopsIn(call.toolCallId, pending);
  const status = pending ? "live" : failed ? "failed" : "done";
  return (
    <motion.button
      type="button"
      className={`tool-chip ${status}${open ? " open" : ""}`}
      aria-expanded={open}
      disabled={!expandable}
      onClick={() => expandable && onToggle()}
      layoutId={animate ? entry.flyId : undefined}
      // Only the hand-off into a head moves it, never its own re-renders (a growing label). The
      // value must differ from the head mark's: motion only resumes a shared element from a
      // predecessor whose layoutDependency differs (NodeStack.promote).
      layoutDependency="chip"
      style={{ borderRadius: 999 }}
      initial={animate && pops ? { opacity: 0, scale: 0.8, y: 4 } : false}
      animate={{ opacity: 1, scale: 1, y: 0 }}
      transition={{ ...POP, layout: FLY }}
    >
      <Icon name={toolIcon(call, result, summary)} className="tool-row-icon" />
      <CallWords summary={summary} pending={pending} failed={failed} />
      <OrbitSpinner active={pending} failed={failed} className="tool-chip-spinner" />
    </motion.button>
  );
}

function Sheet({ entry, animate }: { entry: ChipCall; animate: boolean }) {
  const { shownResult, pending } = useCallFace(entry.call, entry.result, entry.liveText, entry.running);
  const body = <ToolDetail call={entry.call} result={shownResult} live={pending} />;
  if (!animate) return <div className="tool-sheet">{body}</div>;
  return (
    <motion.div className="tool-sheet" initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: "auto" }} exit={{ opacity: 0, height: 0 }} transition={SHEET}>
      {body}
    </motion.div>
  );
}

const chipId = (entry: ChipCall, index: number) => entry.call.toolCallId ?? `chip:${index}`;

/**
 * A run of chip-shaped calls (`toolPresentation`) flowing side by side, as Chat's activity
 * chips do. One sheet opens beneath the strip for the chip clicked; another chip swaps it.
 * The open chip is remembered in `ToolExpansion`, under the same key a row would use, so it
 * survives the streamed message being replaced by the saved one. Images the calls returned
 * show under the strip.
 */
export function ToolChips({ entries, after }: { entries: ChipCall[]; after?: ReactNode }) {
  const store = useContext(ToolExpansion);
  const animate = useTranscriptMotion();
  const storeKey = (id: string) => JSON.stringify([store?.scope ?? "", `tool:${id}`]);
  const [open, setOpen] = useState<string | undefined>(() =>
    entries.map(chipId).find((id) => store?.keys.has(storeKey(id))));
  const toggle = (id: string) => {
    const next = open === id ? undefined : id;
    if (open) store?.keys.delete(storeKey(open));
    if (next) store?.keys.add(storeKey(next));
    setOpen(next);
  };
  const openIndex = entries.findIndex((entry, index) => chipId(entry, index) === open);
  const openEntry = openIndex >= 0 ? entries[openIndex] : undefined;
  const pictured = entries.filter((entry) => entry.result?.images?.length);
  return (
    <div className="tool-chips">
      <div className="tool-chip-strip">
        {entries.map((entry, index) => {
          const id = chipId(entry, index);
          return <ToolChip key={id} entry={entry} open={open === id} onToggle={() => toggle(id)} />;
        })}
        {after}
      </div>
      {animate ? (
        <AnimatePresence initial={false}>
          {openEntry && <Sheet key={open} entry={openEntry} animate />}
        </AnimatePresence>
      ) : openEntry && <Sheet entry={openEntry} animate={false} />}
      {pictured.map((entry) => <ToolImages key={chipId(entry, entries.indexOf(entry))} call={entry.call} result={entry.result!} />)}
    </div>
  );
}

interface ToolRowProps {
  call: NormalizedBlock;
  result?: NormalizedBlock;
  liveText?: string;
  running?: boolean;
}

/**
 * One tool call. Edits, writes and commands that might change something are full-width rows;
 * everything else is a chip (`toolPresentation`), on its own strip when drawn alone.
 */
export function ToolRow(props: ToolRowProps) {
  return toolPresentation(props.call) === "chip" ? <ToolChips entries={[props]} /> : <ToolCallRow {...props} />;
}

function ToolCallRow({ call, result, liveText, running }: ToolRowProps) {
  const [open, toggle] = useToolDisclosure(call.toolCallId ? `tool:${call.toolCallId}` : undefined);
  const { summary, failed, pending, shownResult, expandable } = useCallFace(call, result, liveText, running);
  const animate = useTranscriptMotion();
  const pops = usePopsIn(call.toolCallId, pending);

  return (
    <motion.div
      className={`tool-row ${summary.kind === "bash" ? "command" : ""} ${open ? "open" : ""} ${failed ? "error" : ""}`}
      initial={animate && pops ? { opacity: 0, scale: 0.96, y: 4 } : false}
      animate={{ opacity: 1, scale: 1, y: 0 }}
      transition={POP}
    >
      <button type="button" className="tool-row-head" onClick={() => expandable && toggle()} disabled={!expandable} aria-expanded={open}>
        {/* Edits and writes get the quill mark — it writes while the tool runs, then rests —
            every other row keeps its static tool icon. */}
        {summary.kind === "edit" || summary.kind === "write" ? (
          <QuillMark live={pending} className="tool-row-icon" />
        ) : (
          <Icon name={toolIcon(call, result, summary)} className="tool-row-icon" />
        )}
        <CallWords summary={summary} pending={pending} failed={failed} />
        <span className="tool-row-status">
          <OrbitSpinner active={pending} failed={failed} />
          {expandable && <Icon name="chevron" className="tool-chevron" />}
        </span>
      </button>
      {result?.images?.length ? <ToolImages call={call} result={result} /> : null}
      {animate ? (
        <AnimatePresence initial={false}>
          {open && expandable && (
            <motion.div key="detail" className="tool-row-reveal" initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: "auto" }} exit={{ opacity: 0, height: 0 }} transition={SHEET}>
              <ToolDetail call={call} result={shownResult} live={pending} head={false} />
            </motion.div>
          )}
        </AnimatePresence>
      ) : open && expandable && <ToolDetail call={call} result={shownResult} live={pending} head={false} />}
    </motion.div>
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
