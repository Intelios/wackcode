import { MessageBrowserContexts } from "./browser/MessageBrowserContexts";
import { Fragment, memo, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { ChatRefRecord, CommandPresentation, NormalizedBlock, NormalizedMessage, PlanState, RunTiming, SessionSnapshot, SkillDraftStatus, TaskStatus } from "../types";
import { AssistantNameContext } from "../agentName";
import { useFollowScroll } from "../hooks/useFollowScroll";
import { motionAllowed, useSmoothText } from "../hooks/useSmoothText";
import { formatRunDuration, isPlanMode } from "../chat-utils";
import { blockKey, flyId, layoutTranscript, type TranscriptSlot } from "../explore-utils";
import { completedPlan, layoutWorkTurns, type WorkRow } from "../completed-work";
import { splitFileSection, type FileAttachment } from "../attachment-utils";
import { chatMentionToken } from "../chat-refs";
import { splitMentions } from "../mention-utils";
import { SUBAGENT_TOOL_NAME, SHOW_VISUAL_TOOL_NAME, SKILL_CREATOR_TOOL_NAME, currentVisualIds, exploreKind, parseSkillPreviewDetails, parseSubagentDetails, parseVisualArgs, pendingSubagentDetails, toolPresentation } from "../tool-utils";
import { hasVisibleMessages, latestTurn, messageText } from "../tree-utils";
import { ExploreExpansion, ExploreGroup, ExploreGroupingEnabled } from "./ExploreGroup";
import { Icon } from "./Icons";
import { Markdown } from "./Markdown";
import { MessageActions, type MessageActionItem } from "./MessageActions";
import { MessageEditor } from "./MessageEditor";
import { PlanCard, type PlanAction } from "./PlanCard";
import { VisualCallCard, VisualCardScope, VisualSentNote, type VisualScope } from "./VisualCard";
import { ScrollRail } from "./ScrollRail";
import { SkillDraftCard, type SkillDraftAction } from "./SkillDraftCard";
import { turnExcerpt, type RailTurn } from "../scroll-rail";
import { splitVisualAction, type VisualSendResult } from "../visual-action";
import { SubagentGroup } from "./SubagentChip";
import { ThinkingExpansion, ThinkingRow } from "./ThinkingRow";
import { FLY, OrphanResult, ToolChips, ToolExpansion, ToolRow, useTranscriptMotion, type ChipCall } from "./ToolRow";
import { PonderingDuck } from "./PonderingDuck";
import { useContextMenu } from "./ui/ContextMenu";
import { useNavigationDismiss } from "./ui/NavigationScope";
import { ImageLightbox } from "./ui/ImageLightbox";
import { Popover } from "./ui/Popover";
import { Tooltip } from "./ui/Tooltip";
import { WorkTurn } from "./WorkTurn";
import { CompactionRow } from "./CompactionRow";
import { CompactingStage } from "./CompactingStage";
import type { TranscriptViewState } from "../transcript-view";

interface Props {
  messages: NormalizedMessage[];
  modelSwitches?: DisplayModelSwitch[];
  partial?: NormalizedMessage;
  running: boolean;
  activity?: string;
  activeRun?: { runId?: string; startedAt: number };
  compaction?: SessionSnapshot["compaction"];
  runTimings?: RunTiming[];
  /** Settings › Appearance; sub-agent inspection transcripts opt out. */
  collapseCompletedWork?: boolean;
  /** Scope transient disclosures to this chat, without remounting its transcript. */
  scopeKey?: string;
  /** App-owned reading/disclosure state; only the selected tab is mounted. */
  viewState?: TranscriptViewState;
  historyReady?: boolean;
  /** Protect the trailing diagnostics after a worker crash or refused run. */
  status?: TaskStatus;
  liveToolText?: Record<string, string>;
  /** Structured progress of in-flight tools that report it (sub-agent chips). */
  liveToolDetails?: Record<string, unknown>;
  /** Latest Plan mode state; PlanCards use it to know which proposal is awaiting a decision. */
  planState?: PlanState;
  onPlanAction?: (action: PlanAction) => void;
  /** A visual reply's button was clicked: send its words as the user (`visual-action.ts`). */
  onVisualSend?: (text: string, title: string) => Promise<VisualSendResult>;
  /** A link inside a visual: the webview cannot open one, so the app does. */
  onVisualLink?: (href: string) => void;
  /** The skill-creator review cards: draft publication states, the owning chat, and their actions. */
  skillDrafts?: Record<string, SkillDraftStatus>;
  /** Drafts with a save in flight. */
  skillDraftsSaving?: ReadonlySet<string>;
  /** The chat the transcript renders; a preview owned by another chat cannot be saved here. */
  taskId?: string;
  onSkillAction?: (action: SkillDraftAction) => void;
  /** The draft's full SKILL.md body, on demand. */
  loadSkillDocument?: (draftId: string) => Promise<string | undefined>;
  /** Retry, edit, rewind, version switching and fork are offered only while this is true. */
  actionsEnabled?: boolean;
  /** Whether the chat's model accepts images, for editing a message that has some. */
  vision?: boolean;
  modelName?: string;
  /** Resolves true once an edit has been sent, which closes the editor. */
  onMessageAction?: (action: MessageAction) => Promise<boolean> | void;
  /** Offered right after a rewind. */
  onUndoRewind?: () => void;
  /** Resolves a sent message's attached image to its full-size URL, for the lightbox. */
  loadImage?: MessageImageLoader;
  /** Opening the chats a message referenced with `@` (absent: tokens are not clickable). */
  chatLinks?: ChatLinks;
  /** What an empty chat says. Absent: the coding agent's invitation. */
  empty?: { title: string; body: string };
}

/** How the tool-related cards and their actions reach every rendered block. */
interface SkillCardContext {
  /** The newest preview's tool-call id on this branch; only that card is actionable. */
  latestId: string | undefined;
  drafts?: Record<string, SkillDraftStatus>;
  saving?: ReadonlySet<string>;
  taskId?: string;
  onAction?: (action: SkillDraftAction) => void;
  loadDocument?: (draftId: string) => Promise<string | undefined>;
}

/** How the transcript fetches the full-size original of one of a sent message's images. */
export type MessageImageLoader = (entryId: string, index: number) => Promise<string | undefined>;

/**
 * How a message's `@` chat tokens reach their chat. A reference to a chat that no longer
 * exists (`known`) stays a plain highlighted token: nothing to open.
 */
export interface ChatLinks {
  known: ReadonlySet<string>;
  open: (taskId: string) => void;
}

export interface DisplayModelSwitch {
  id: string;
  at: number;
  from: string;
  to: string;
}

/** What the transcript asks the app to do with a message. */
export type MessageAction =
  | { type: "copy"; message: NormalizedMessage }
  | { type: "copy-prompt"; text: string }
  | { type: "edit"; message: NormalizedMessage; text: string; files: FileAttachment[]; removeImages: number[] }
  /** Send this user message again as a new version. */
  | { type: "retry"; message: NormalizedMessage }
  /** Take the conversation back to just before this user message. */
  | { type: "rewind"; message: NormalizedMessage }
  | { type: "switch"; entryId: string }
  /** A new chat from the turn this assistant message ends. */
  | { type: "fork"; message: NormalizedMessage };

const NO_SLOTS: TranscriptSlot[] = [];

type LocalAction = MessageAction | { type: "start-edit"; id: string } | { type: "cancel-edit" };

function StreamingText({ text }: { text: string }) {
  const shown = useSmoothText(text, true);
  return <div className="stream-text assistant-text"><Markdown streaming>{shown}</Markdown></div>;
}

export function MentionText({ text, chatRefs, chatLinks }: { text: string; chatRefs?: ChatRefRecord[]; chatLinks?: ChatLinks }) {
  const refs = new Map((chatRefs ?? []).map((ref) => [chatMentionToken(ref.name), ref] as const));
  return <>{splitMentions(text).map((segment, index) => {
    const ref = segment.mention ? refs.get(segment.text) : undefined;
    if (!ref) return segment.mention ? <span key={index} className="mention">{segment.text}</span> : segment.text;
    if (!chatLinks?.known.has(ref.id)) return <span key={index} className="mention chat">{segment.text}</span>;
    return (
      <button key={index} type="button" className="mention chat" title="Open this chat" onClick={() => chatLinks.open(ref.id)}>
        <Icon name="comment" />{segment.text}
      </button>
    );
  })}</>;
}

function commandSummary(presentation: CommandPresentation): string {
  if (presentation.kind === "command") return presentation.arguments.trim();
  const label = presentation.kind === "goal-resume" ? "Resumed" : "Automatic";
  const round = presentation.round ? `${label} round ${presentation.round}` : `${label} goal round`;
  return presentation.nextAction?.trim() ? `${round} · ${presentation.nextAction.trim()}` : round;
}

/** Compact transcript face for a command while the complete model-visible prompt stays inspectable. */
function CommandMessage({ message, onCopy }: {
  message: NormalizedMessage;
  onCopy: (text: string) => Promise<boolean> | void;
}) {
  const presentation = message.commandPresentation!;
  const anchor = useRef<HTMLButtonElement>(null);
  const copyButton = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const prompt = message.blocks.filter((block) => block.type === "text").map((block) => block.text ?? "").join("");
  const summary = commandSummary(presentation);

  useEffect(() => { setOpen(false); setCopied(false); }, [message.id]);
  useEffect(() => {
    if (!open) return;
    const frame = requestAnimationFrame(() => copyButton.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, [open]);

  async function copyPrompt() {
    await onCopy(prompt);
    setCopied(true);
  }

  return (
    <div className="bubble command-message">
      <Tooltip label="View sent prompt">
        <button
          ref={anchor}
          type="button"
          className="command-word"
          aria-label={`${presentation.name} command. View sent prompt`}
          aria-haspopup="dialog"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
        >
          <span>{presentation.name}</span>
        </button>
      </Tooltip>
      {summary && <><span className="command-separator" aria-hidden="true">·</span><span className="command-summary"><MentionText text={summary} /></span></>}
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} side="bottom" align="end" className="command-prompt-popover">
        <div className="command-prompt-dialog" role="dialog" aria-label={`${presentation.name} prompt sent to the agent`}>
          <div className="command-prompt-head">
            <div><strong>{presentation.name}</strong><span>Prompt sent to the agent</span></div>
            <button ref={copyButton} type="button" className="secondary-button compact" onClick={() => void copyPrompt()}>
              <Icon name={copied ? "check" : "copy"} /> {copied ? "Copied" : "Copy prompt"}
            </button>
          </div>
          <pre tabIndex={0}>{prompt}</pre>
        </div>
      </Popover>
    </div>
  );
}

function RunDuration({ startedAt, durationMs }: { startedAt?: number; durationMs?: number }) {
  const [now, setNow] = useState(Date.now);

  useEffect(() => {
    if (startedAt === undefined) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [startedAt]);

  const live = startedAt !== undefined;
  const elapsed = durationMs ?? Math.max(0, now - (startedAt ?? now));
  const label = live ? "Working for" : "Worked for";

  return (
    <div className={`run-duration${live ? " live" : ""}`}>
      <span className="run-duration-chip"><Icon name="clock" />{label} <strong>{formatRunDuration(elapsed)}</strong></span>
    </div>
  );
}

function ModelSwitchDivider({ entry }: { entry: DisplayModelSwitch }) {
  return (
    <div className="model-switch-divider" data-transcript-anchor={`switch:${entry.id}`}>
      <span>Model switched <strong>{entry.from}</strong> <span className="model-switch-arrow">→</span> <strong>{entry.to}</strong></span>
    </div>
  );
}

/**
 * A reasoning row outside an Explored group carries the id its landing mark in the group's head
 * will use, so when a run takes it in, it visibly flies into the head (`ExploreGroup.tsx`).
 * Only that hand-off moves it: `layoutDependency` stays fixed so streamed text never re-measures
 * (and differs from the mark's, or motion wouldn't resume the mark from this row).
 */
function FlyingThought({ id, children }: { id: string; children: ReactNode }) {
  const animate = useTranscriptMotion();
  if (!animate) return <>{children}</>;
  return <motion.div className="fly-thought" layoutId={id} layoutDependency="thought" transition={{ layout: FLY }}>{children}</motion.div>;
}

/**
 * The cards some tool calls render as instead of a plain call: a plan, sub-agent chips, a
 * skill review. Undefined for an ordinary call, which `renderSlots` draws as a chip or row.
 */
function renderSpecialCall(
  block: NormalizedBlock,
  results: Map<string, NormalizedBlock>,
  liveToolDetails: Record<string, unknown> | undefined,
  live: boolean,
  planState: PlanState | undefined,
  onPlanAction: ((action: PlanAction) => void) | undefined,
  running: boolean,
  skillCard: SkillCardContext | undefined
): ReactNode | undefined {
  const result = block.toolCallId ? results.get(block.toolCallId) : undefined;
  const plan = block.toolName === "plan_mode_complete" ? completedPlan(result) : undefined;
  if (plan !== undefined) {
    const current = isPlanMode(planState?.mode) && planState?.phase === "ready" && planState.plan === plan;
    return <PlanCard plan={plan} current={current} busy={running} onAction={onPlanAction} ultra={current && planState?.mode === "ultraplan"} />;
  }
  if (block.toolName === SHOW_VISUAL_TOOL_NAME) {
    // The card is this call's settled outcome; while it runs the ordinary chip shows progress,
    // and a refused call keeps its error on the plain row.
    if (result && !result.isError && parseVisualArgs(block)) return <VisualCallCard block={block} />;
  }
  if (block.toolName === SUBAGENT_TOOL_NAME) {
    // The final result is authoritative; while running, the latest live update, or the
    // call's own arguments until the first update arrives. A call refused before any child
    // started has no details and falls through to the plain row with its error.
    const liveDetails = block.toolCallId ? liveToolDetails?.[block.toolCallId] : undefined;
    const details = result
      ? parseSubagentDetails(result.details)
      : live ? parseSubagentDetails(liveDetails) ?? pendingSubagentDetails(block) : undefined;
    if (details && block.toolCallId) return <SubagentGroup toolCallId={block.toolCallId} details={details} live={Boolean(details.background) || (live && !result)} />;
  }
  if (block.toolName === SKILL_CREATOR_TOOL_NAME && result) {
    // A finished preview result is the review card; while it runs the plain tool row shows
    // progress, and a refused preview (no parseable details) keeps its error visible there.
    const preview = result.isError ? undefined : parseSkillPreviewDetails(result.details);
    if (preview) {
      return (
        <SkillDraftCard
          details={preview}
          status={skillCard?.drafts?.[preview.draftId]}
          current={skillCard !== undefined && block.toolCallId === skillCard.latestId}
          owned={!skillCard?.taskId || preview.ownerTaskId === skillCard.taskId}
          busy={running}
          saving={skillCard?.saving?.has(preview.draftId)}
          onAction={skillCard?.onAction}
          loadDocument={skillCard?.loadDocument}
        />
      );
    }
  }
  return undefined;
}

function renderBlock(
  block: NormalizedBlock,
  key: string,
  results: Map<string, NormalizedBlock>,
  liveToolText: Record<string, string> | undefined,
  liveToolDetails: Record<string, unknown> | undefined,
  live: boolean,
  planState: PlanState | undefined,
  onPlanAction: ((action: PlanAction) => void) | undefined,
  running: boolean,
  skillCard: SkillCardContext | undefined,
  streaming?: boolean
): ReactNode {
  if (block.type === "thinking") {
    // A streamed block is still being written until the worker has clocked its end.
    return (
      <FlyingThought id={flyId({ block, key })}>
        <ThinkingRow text={block.text ?? ""} durationMs={block.durationMs} startedAt={block.startedAt} live={running && streaming === true && block.durationMs === undefined} expansionKey={key} />
      </FlyingThought>
    );
  }
  if (block.type === "tool-call") {
    const special = renderSpecialCall(block, results, liveToolDetails, live, planState, onPlanAction, running, skillCard);
    if (special !== undefined) return special;
    const result = block.toolCallId ? results.get(block.toolCallId) : undefined;
    const liveText = block.toolCallId ? liveToolText?.[block.toolCallId] : undefined;
    return <ToolRow call={block} result={result} liveText={liveText} running={live && !result} />;
  }
  if (block.type === "tool-result") {
    return <OrphanResult block={block} />;
  }
  if (!block.text) return null;
  if (streaming) return <StreamingText text={block.text} />;
  // `.assistant-text` is the bubble when Settings › Appearance › Message bubbles is on;
  // off it styles nothing and the prose lays out as it always has.
  return <div className="assistant-text"><Markdown>{block.text}</Markdown></div>;
}

function renderSlots(
  message: NormalizedMessage,
  slots: TranscriptSlot[],
  results: Map<string, NormalizedBlock>,
  liveToolText: Record<string, string> | undefined,
  liveToolDetails: Record<string, unknown> | undefined,
  live: boolean,
  planState: PlanState | undefined,
  onPlanAction: ((action: PlanAction) => void) | undefined,
  running: boolean,
  skillCard: SkillCardContext | undefined,
  streaming?: boolean
): ReactNode {
  // Consecutive chip-shaped calls share one wrapping strip, as Chat's activity chips do; any
  // other slot (prose, a row, reasoning, a card, a group) closes it.
  const nodes: ReactNode[] = [];
  let strip: ChipCall[] = [];
  const flush = () => {
    if (!strip.length) return;
    const first = strip[0].call.toolCallId ?? nodes.length;
    nodes.push(<div key={`chips:${first}`} className="block-slot"><ToolChips entries={strip} /></div>);
    strip = [];
  };
  for (const slot of slots) {
    if (slot.type === "explore") {
      flush();
      nodes.push(
        <div key={`explore:${slot.group.key}`} className="block-slot">
          <ExploreGroup group={slot.group} results={results} liveToolText={liveToolText} running={running} />
        </div>
      );
      continue;
    }
    const block = message.blocks[slot.index];
    const special = block.type === "tool-call" ? renderSpecialCall(block, results, liveToolDetails, live, planState, onPlanAction, running, skillCard) : undefined;
    if (block.type === "tool-call" && special === undefined && toolPresentation(block) === "chip") {
      const result = block.toolCallId ? results.get(block.toolCallId) : undefined;
      strip.push({
        call: block,
        result,
        liveText: block.toolCallId ? liveToolText?.[block.toolCallId] : undefined,
        running: live && !result,
        // Exploration may yet be taken into a group: it flies into the head when it is.
        flyId: exploreKind(block) ? flyId({ block, key: blockKey(message, slot.index) }) : undefined
      });
      continue;
    }
    flush();
    nodes.push(
      <div key={block.toolCallId ?? slot.index} className="block-slot">
        {special ?? renderBlock(block, blockKey(message, slot.index), results, liveToolText, liveToolDetails, live, planState, onPlanAction, running, skillCard, streaming)}
      </div>
    );
  }
  flush();
  return nodes;
}

// Recomputed for every message on every streamed partial, so image blocks stand in as their id
// and readiness rather than their preview data.
function signatureBlock(block: NormalizedBlock): unknown {
  return block.type === "image" ? { image: block.imageId, ready: Boolean(block.thumbnail) } : block;
}

/** A tool call's inputs from outside its message: its result and live progress. */
function callInputs(
  block: NormalizedBlock,
  results: Map<string, NormalizedBlock>,
  liveToolText?: Record<string, string>,
  liveToolDetails?: Record<string, unknown>
): unknown[] | null {
  return block.type === "tool-call" && block.toolCallId
    ? [results.get(block.toolCallId) ?? null, liveToolText?.[block.toolCallId] ?? null, liveToolDetails?.[block.toolCallId] ?? null]
    : null;
}

function signature(
  message: NormalizedMessage,
  slots: TranscriptSlot[],
  results: Map<string, NormalizedBlock>,
  liveToolText?: Record<string, string>,
  liveToolDetails?: Record<string, unknown>
): string {
  return JSON.stringify([
    message.blocks.map(signatureBlock),
    message.stopReason,
    message.errorMessage,
    message.entryId,
    message.versions,
    message.checkpoint,
    message.commandPresentation,
    message.turn,
    message.blocks.map((block) => callInputs(block, results, liveToolText, liveToolDetails)),
    // An exploration group also renders blocks from the later messages its run spans.
    slots.map((slot) => slot.type === "block" ? slot.index : [
      slot.group.key,
      slot.group.items.map((item) => [item.key, item.live, item.streaming, signatureBlock(item.block), callInputs(item.block, results, liveToolText, liveToolDetails)])
    ])
  ]);
}

/**
 * `signature` stringifies the whole message, which is O(transcript) when done for every
 * message on every event. A message object's own content is immutable once the app holds it
 * (the worker re-normalizes instead of mutating, and the delta merge reuses unchanged
 * objects), so the signature is cached by message identity and only recomputed when one of
 * its external inputs moves: the result block answering one of its tool calls, or that call's
 * live stream text, or what its exploration groups hold.
 */
const signatureCache = new WeakMap<NormalizedMessage, Map<string, { deps: unknown[]; sig: string }>>();

function cachedSignature(
  message: NormalizedMessage,
  slots: TranscriptSlot[],
  results: Map<string, NormalizedBlock>,
  liveToolText?: Record<string, string>,
  liveToolDetails?: Record<string, unknown>,
  variant = "full"
): string {
  const deps: unknown[] = [];
  const pushInputs = (block: NormalizedBlock) => {
    const inputs = callInputs(block, results, liveToolText, liveToolDetails);
    if (inputs) deps.push(...inputs);
  };
  message.blocks.forEach(pushInputs);
  deps.push(slots.map((slot) => slot.type === "block" ? slot.index : `group:${slot.group.key}`).join(","));
  for (const slot of slots) {
    if (slot.type !== "explore") continue;
    for (const item of slot.group.items) {
      deps.push(item.block, item.live, item.streaming);
      pushInputs(item.block);
    }
  }
  const variants = signatureCache.get(message) ?? new Map<string, { deps: unknown[]; sig: string }>();
  const cached = variants.get(variant);
  if (cached && cached.deps.length === deps.length && cached.deps.every((dep, index) => dep === deps[index])) {
    return cached.sig;
  }
  const sig = signature(message, slots, results, liveToolText, liveToolDetails);
  variants.set(variant, { deps, sig });
  signatureCache.set(message, variants);
  return sig;
}

/** An attached text file on a user message; opens to show the exact text the model received. */
function FileChip({ file }: { file: FileAttachment }) {
  const [open, setOpen] = useState(false);
  const lines = file.text.split("\n").length;
  return (
    <div className={`file-chip ${open ? "open" : ""}`}>
      <button type="button" className="file-chip-label" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
        <Icon name="file" />
        <strong>{file.name}</strong>
        <small>{lines} {lines === 1 ? "line" : "lines"}</small>
      </button>
      {open && <pre className="file-chip-content">{file.text}</pre>}
    </div>
  );
}

/** A sent message's attached images; each opens full size in the lightbox. */
function MessageImages({ images, entryId, loadImage, hidden = [] }: { images: NormalizedBlock[]; entryId?: string; loadImage?: MessageImageLoader; hidden?: number[] }) {
  const [shown, setShown] = useState<{ index: number; image: NormalizedBlock } | null>(null);
  const loadShown = useCallback(
    () => (loadImage && entryId && shown ? loadImage(entryId, shown.index) : Promise.resolve(undefined)),
    [loadImage, entryId, shown]
  );
  return (
    <div className="message-images">
      {images.map((image, index) => hidden.includes(index) ? null : image.thumbnail
        ? (
          <button key={image.imageId ?? index} type="button" className="message-image" onClick={() => setShown({ index, image })} aria-label={`Open attached image ${index + 1}`}>
            <img src={image.thumbnail} alt={`Attached image ${index + 1}`} />
          </button>
        )
        : <div key={image.imageId ?? index} className="image-pending" role="img" aria-label={`Attached image ${index + 1}`}><Icon name="image" /></div>
      )}
      {shown?.image.thumbnail && (
        <ImageLightbox preview={shown.image.thumbnail} load={loadShown} alt={`Attached image ${shown.index + 1}`} onClose={() => setShown(null)} />
      )}
    </div>
  );
}

interface MessageProps {
  message: NormalizedMessage;
  /** What the message renders: its blocks, minus those an exploration group folded away. */
  slots: TranscriptSlot[];
  results: Map<string, NormalizedBlock>;
  liveToolText?: Record<string, string>;
  liveToolDetails?: Record<string, unknown>;
  live: boolean;
  running: boolean;
  planState?: PlanState;
  onPlanAction?: (action: PlanAction) => void;
  skillCard: SkillCardContext | undefined;
  sig: string;
  /** A split work slice must not duplicate its original message's terminal footer/menu. */
  turnActions?: boolean;
  /** Work and outcome slices of one message must not share a scrolling anchor. */
  anchorPart?: "work" | "outcome";
  actionsEnabled: boolean;
  /** Offer Retry on this message: it ends (or, unanswered, is) the latest turn. */
  retry: boolean;
  editing: boolean;
  vision: boolean;
  modelName?: string;
  onAction: (action: LocalAction) => Promise<boolean> | void;
  loadImage?: MessageImageLoader;
  chatLinks?: ChatLinks;
}

const Message = memo(function Message({ message, slots, results, liveToolText, liveToolDetails, live, running, planState, onPlanAction, skillCard, actionsEnabled, retry, editing, vision, modelName, onAction, loadImage, chatLinks, turnActions = true, anchorPart }: MessageProps) {
  const contextMenu = useContextMenu();
  const scrollAnchor = `message:${message.id}${anchorPart ? `:${anchorPart}` : ""}`;
  if (message.role === "user") {
    const images = message.blocks.filter((block) => block.type === "image");
    // The message text carries attached files and, for a visual's button, the section naming
    // that visual; the transcript shows the words and both as their own parts.
    const spoken = splitVisualAction(messageText(message));
    const { text, files, browserContexts = [], chatRefs = [] } = splitFileSection(spoken.text);
    if (editing) {
      return (
        <div className="msg user editing" data-transcript-anchor={scrollAnchor} data-turn={message.versions ? message.versions.group : message.id}>
          <MessageEditor
            text={text}
            browserContexts={browserContexts}
            chatRefs={chatRefs}
            images={images}
            files={files}
            vision={vision}
            modelName={modelName}
            loadImage={loadImage && message.entryId ? (index) => loadImage(message.entryId!, index) : undefined}
            onCancel={() => void onAction({ type: "cancel-edit" })}
            onSend={async (edited, keptFiles, removeImages) => (await onAction({ type: "edit", message, text: edited, files: keptFiles, removeImages })) === true}
          />
        </div>
      );
    }
    const items: MessageActionItem[] = [];
    if (text) items.push({ id: "copy", label: "Copy", icon: "copy", onClick: () => void onAction({ type: "copy", message }) });
    if (actionsEnabled && message.entryId) {
      if (retry) items.push({ id: "retry", label: "Retry", icon: "refresh", onClick: () => void onAction({ type: "retry", message }) });
      items.push({ id: "edit", label: "Edit", icon: "pencil", onClick: () => void onAction({ type: "start-edit", id: message.id }) });
      items.push({ id: "rewind", label: "Rewind to here", icon: "rewind", onClick: () => void onAction({ type: "rewind", message }) });
    }
    return (
      <div className="msg user" data-transcript-anchor={scrollAnchor} onContextMenu={(event) => contextMenu(event, items.map((item) => ({ label: item.label, icon: <Icon name={item.icon} />, onSelect: item.onClick })), "Message menu")} data-turn={message.versions ? message.versions.group : message.id}>
        <MessageBrowserContexts contexts={browserContexts} images={images} loadImage={loadImage && message.entryId ? (index) => loadImage(message.entryId!, index) : undefined} />
        {images.length > 0 && <MessageImages images={images} hidden={browserContexts.map((context) => context.imageIndex)} entryId={message.entryId} loadImage={loadImage} />}
        {files.length > 0 && (
          <div className="message-files">
            {files.map((file, index) => <FileChip key={index} file={file} />)}
          </div>
        )}
        {text && (message.commandPresentation
          ? <CommandMessage message={message} onCopy={(prompt) => onAction({ type: "copy-prompt", text: prompt })} />
          : <div className="bubble"><MentionText text={text} chatRefs={chatRefs} chatLinks={chatLinks} /></div>)}
        {spoken.title && <VisualSentNote title={spoken.title} />}
        <MessageActions
          align="end"
          items={items}
          versions={message.versions}
          switchDisabled={!actionsEnabled}
          onSwitch={(entryId) => void onAction({ type: "switch", entryId })}
        />
      </div>
    );
  }
  if (message.role === "system") {
    const text = message.blocks.map((block) => block.text ?? "").join("\n").trim();
    return text ? <div className="msg system" data-transcript-anchor={scrollAnchor}>{text}</div> : null;
  }
  const failed = message.stopReason === "error" || message.stopReason === "aborted";
  // Everything it had belongs to an exploration group an earlier message shows.
  if (slots.length === 0 && !failed && !(turnActions && message.turn)) return null;
  return (
    <div className="msg assistant" data-transcript-anchor={scrollAnchor} onContextMenu={(event) => contextMenu(event, turnMenu(message, actionsEnabled && turnActions && Boolean(message.turn), retry, onAction).map((item) => ({ label: item.label, icon: <Icon name={item.icon} />, onSelect: item.onClick })), "Message menu")}>
      {renderSlots(message, slots, results, liveToolText, liveToolDetails, live, planState, onPlanAction, running, skillCard)}
      {message.stopReason === "error" && <div className="message-error">{message.errorMessage || "The provider rejected the request."}</div>}
      {message.stopReason === "aborted" && <span className="aborted-label">Stopped</span>}
      {turnActions && message.turn && <TurnActions message={message} actionsEnabled={actionsEnabled} retry={retry} onAction={onAction} />}
    </div>
  );
}, (prev, next) =>
  prev.sig === next.sig && prev.live === next.live && prev.running === next.running && prev.turnActions === next.turnActions && prev.anchorPart === next.anchorPart
  && prev.planState === next.planState && prev.onPlanAction === next.onPlanAction
  && prev.skillCard === next.skillCard
  && prev.actionsEnabled === next.actionsEnabled && prev.retry === next.retry && prev.editing === next.editing
  && prev.vision === next.vision && prev.modelName === next.modelName && prev.onAction === next.onAction
  && prev.loadImage === next.loadImage && prev.chatLinks === next.chatLinks);

// No Copy here: while a run streams only Copy would fill this row, and the hidden row still
// reserved height — a visible gap above the live stream. Copy stays on user bubbles and `/copy`.
function turnMenu(message: NormalizedMessage, actionsEnabled: boolean, retry: boolean, onAction: MessageProps["onAction"]): MessageActionItem[] {
  const items: MessageActionItem[] = [];
  if (actionsEnabled) {
    if (retry) items.push({ id: "retry", label: "Retry", icon: "refresh", onClick: () => void onAction({ type: "retry", message }) });
    items.push({ id: "fork", label: "Fork from here", icon: "branch", onClick: () => void onAction({ type: "fork", message }) });
  }
  return items;
}

// With no items this row renders nothing (MessageActions returns null), so a mid-run turn
// footer costs no space; retry/fork appear under the settled answer as before.
function TurnActions({ message, actionsEnabled, retry, onAction }: Pick<MessageProps, "message" | "actionsEnabled" | "retry" | "onAction">) {
  return <MessageActions align="start" items={turnMenu(message, actionsEnabled, retry, onAction)} />;
}

/**
 * What the agent is up to while nothing has streamed yet: Chat's typing presence, made to sit
 * in a dense transcript — the pondering duck, three bouncing dots and the caption. The run
 * clock stays on its own chip above the work. Dots hold still under reduced motion (styles.css).
 */
function WorkingPresence({ label }: { label: string }) {
  return (
    <div className="agent-working" role="status">
      <PonderingDuck live className="agent-working-duck" />
      <span className="work-dots" aria-hidden="true"><i /><i /><i /></span>
      <span className="agent-working-label">{label}</span>
    </div>
  );
}

function activityLabel(activity?: string): string {
  if (!activity) return "Working…";
  if (activity.startsWith("tool_execution")) return "";
  if (activity === "mcp_connect_start") return "Starting MCP servers…";
  if (activity.startsWith("auto_retry") || activity.startsWith("summarization_retry")) return "Retrying…";
  return "Working…";
}

export function Transcript({ messages, modelSwitches = [], partial, running, activity, activeRun, compaction, runTimings = [], collapseCompletedWork = true, scopeKey = "", viewState, historyReady = true, status, liveToolText, liveToolDetails, planState, onPlanAction, onVisualSend, onVisualLink, skillDrafts, skillDraftsSaving, taskId, onSkillAction, loadSkillDocument, actionsEnabled = false, vision = false, modelName, onMessageAction, onUndoRewind, loadImage, chatLinks, empty }: Props) {
  const scroll = useFollowScroll(viewState, historyReady);
  const { ref, onScroll, onWheel, detached, pauseFollowing, jumpToLatest } = scroll;
  const [expandedWork, setExpandedWork] = useState(() => viewState?.work ?? new Set<string>());
  const [expandedCompactions, setExpandedCompactions] = useState(() => viewState?.compactions ?? new Set<string>());
  const [editingId, setEditingId] = useState<string>();
  useNavigationDismiss(() => setEditingId(undefined));
  const [expandedThinking] = useState(() => viewState?.thinking ?? new Set<string>());
  const [expandedTools] = useState(() => viewState?.tools ?? new Set<string>());
  const toolExpansions = useMemo(() => ({ scope: scopeKey, keys: expandedTools }), [scopeKey, expandedTools]);
  // These sets already use chat/entry keys. Persist their identities without retaining
  // DOM nodes, animation timers, lightboxes or a sent-message editor across navigation.
  const reduced = useReducedMotion() ?? false;
  const animateUi = !reduced && motionAllowed();
  // Set after the render where the stage was mounted, so a boundary arriving the very next
  // render animates in — boundaries that were already saved on load never do.
  const stageWasLive = useRef(false);
  useEffect(() => { stageWasLive.current = Boolean(compaction); });
  const latest = useMemo(() => latestTurn(messages), [messages]);

  // Stable for the memoized messages: `onMessageAction` is expected to be stable too.
  const handleAction = useCallback(async (action: LocalAction): Promise<boolean> => {
    if (action.type === "start-edit") { setEditingId(action.id); return true; }
    if (action.type === "cancel-edit") { setEditingId(undefined); return true; }
    const done = (await onMessageAction?.(action)) === true;
    if (action.type === "edit" && done) setEditingId(undefined);
    return done;
  }, [onMessageAction]);

  // An edit can't outlive the message it edits, or the chat becoming busy.
  useEffect(() => {
    if (editingId && (!actionsEnabled || !messages.some((message) => message.id === editingId))) setEditingId(undefined);
  }, [actionsEnabled, editingId, messages]);

  const { results, callIds } = useMemo(() => {
    const resultMap = new Map<string, NormalizedBlock>();
    const calls = new Set<string>();
    for (const message of messages) {
      for (const block of message.blocks) {
        if (block.type === "tool-result" && block.toolCallId) resultMap.set(block.toolCallId, block);
        if (block.type === "tool-call" && block.toolCallId) calls.add(block.toolCallId);
      }
    }
    return { results: resultMap, callIds: calls };
  }, [messages]);

  // The newest preview on the branch is the one awaiting a decision; earlier ones stay
  // readable but inert. Derived from the messages alone, so navigation recomputes it.
  const latestSkillPreviewId = useMemo(() => {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const message = messages[index];
      if (message.role !== "assistant") continue;
      for (let block = message.blocks.length - 1; block >= 0; block -= 1) {
        const entry = message.blocks[block];
        if (entry.type !== "tool-call" || entry.toolName !== SKILL_CREATOR_TOOL_NAME || !entry.toolCallId) continue;
        const result = results.get(entry.toolCallId);
        if (result && !result.isError && parseSkillPreviewDetails(result.details)) return entry.toolCallId;
      }
    }
    return undefined;
  }, [messages, results]);

  const skillCard = useMemo<SkillCardContext>(() => ({
    latestId: latestSkillPreviewId,
    drafts: skillDrafts,
    saving: skillDraftsSaving,
    taskId,
    onAction: onSkillAction,
    loadDocument: loadSkillDocument,
  }), [latestSkillPreviewId, skillDrafts, skillDraftsSaving, taskId, onSkillAction, loadSkillDocument]);

  const lastAssistantId = useMemo(() => [...messages].reverse().find((message) => message.role === "assistant")?.id, [messages]);

  // Only the newest answer's visuals may send: their buttons stay live, older ones go inert
  // (`VisualCard`), so a scrollback full of widgets cannot each start a turn.
  const visualScope = useMemo<VisualScope>(() => ({
    currentIds: currentVisualIds(messages, partial),
    busy: running,
    onSend: onVisualSend,
    onOpenLink: onVisualLink
  }), [messages, partial, running, onVisualSend, onVisualLink]);

  const grouping = useContext(ExploreGroupingEnabled);
  const assistantName = useContext(AssistantNameContext);
  const [expandedGroups] = useState(() => viewState?.exploration ?? new Set<string>());
  useLayoutEffect(() => {
    if (!viewState) return;
    viewState.work = expandedWork;
    viewState.compactions = expandedCompactions;
    viewState.thinking = expandedThinking;
    viewState.exploration = expandedGroups;
    viewState.tools = expandedTools;
  }, [viewState, expandedWork, expandedCompactions, expandedThinking, expandedGroups, expandedTools]);
  const layout = useMemo(
    () => layoutTranscript(messages, partial, { grouping, liveMessageId: running ? lastAssistantId : undefined }),
    [messages, partial, grouping, running, lastAssistantId]
  );

  const activeUserId = useMemo(() => activeRun
    ? [...messages].reverse().find((message) => message.role === "user" && message.timestamp !== undefined && message.timestamp >= activeRun.startedAt)?.id
    : undefined, [messages, activeRun]);

  // One rail tick per user turn, keyed like the rendered element (versions share it).
  const turns = useMemo<RailTurn[]>(() => messages
    .filter((message) => message.role === "user")
    .map((message) => {
      const text = message.commandPresentation
        ? `/${message.commandPresentation.name} ${commandSummary(message.commandPresentation)}`
        : splitFileSection(splitVisualAction(messageText(message)).text).text;
      return { id: message.versions ? message.versions.group : message.id, excerpt: turnExcerpt(text) };
    }), [messages]);
  const liveTurnId = useMemo(() => {
    const active = activeUserId && messages.find((message) => message.id === activeUserId);
    return active ? (active.versions ? active.versions.group : active.id) : undefined;
  }, [messages, activeUserId]);
  const timingsByMessage = useMemo(() => new Map(runTimings.map((timing) => [timing.userMessageId, timing.durationMs])), [runTimings]);
  const switchesByPosition = useMemo(() => {
    const grouped = new Map<number, DisplayModelSwitch[]>();
    for (const entry of modelSwitches) {
      const position = Math.max(0, Math.min(messages.length, entry.at));
      grouped.set(position, [...(grouped.get(position) ?? []), entry]);
    }
    return grouped;
  }, [messages.length, modelSwitches]);
  const renderSwitches = (position: number) => switchesByPosition.get(position)?.map((entry) => (
    <ModelSwitchDivider key={entry.id} entry={entry} />
  ));

  const workTurns = useMemo(() => layoutWorkTurns(messages, {
    layout, results, callIds, switchPositions: new Set(switchesByPosition.keys()),
    running, activeRun, partial, status, settledUserIds: new Set(timingsByMessage.keys())
  }), [messages, layout, results, callIds, switchesByPosition, running, activeRun, partial, status, timingsByMessage]);

  const renderCompaction = (message: NormalizedMessage) => {
    const key = JSON.stringify([scopeKey, message.id]);
    const row = <CompactionRow message={message} open={expandedCompactions.has(key)} onToggle={() => {
      pauseFollowing();
      setExpandedCompactions((current) => {
        const next = new Set(current);
        if (next.has(key)) next.delete(key); else next.add(key);
        return next;
      });
    }} />;
    // When the live stage is handing off to this freshly-saved boundary, the row pops in a
    // beat after the well's collapse so the shrink-into-row reads as one motion.
    const arrivesAfterStage = message.id === messages[messages.length - 1]?.id
      && stageWasLive.current && animateUi;
    return arrivesAfterStage
      ? <motion.div initial={{ opacity: 0, scale: .97 }} animate={{ opacity: 1, scale: 1 }}
        transition={{ delay: .45, duration: .3, ease: [0.33, 1, 0.68, 1] }}>{row}</motion.div>
      : row;
  };
  const renderMessage = (message: NormalizedMessage, slots = layout.messages.get(message.id) ?? NO_SLOTS, turnActions = true, anchorPart?: "work" | "outcome") => message.compaction ? renderCompaction(message) : (
    <Message message={message} slots={slots} results={results} liveToolText={liveToolText} liveToolDetails={liveToolDetails}
      live={running && message.id === lastAssistantId} running={running} planState={planState} onPlanAction={onPlanAction} skillCard={skillCard}
      sig={cachedSignature(message, slots, results, liveToolText, liveToolDetails, anchorPart)} turnActions={turnActions} anchorPart={anchorPart}
      actionsEnabled={actionsEnabled} retry={message.id === (latest?.answer ?? latest?.user)?.id}
      editing={editingId === message.id} vision={vision} modelName={modelName} onAction={handleAction} loadImage={loadImage} chatLinks={chatLinks} />
  );
  const renderOrphans = (message: NormalizedMessage, blocks = message.blocks.filter((block) => !block.toolCallId || !callIds.has(block.toolCallId))) =>
    blocks.length > 0 ? <div className="orphan-group" data-transcript-anchor={`message:${message.id}`}>
      {blocks.map((block, index) => <OrphanResult key={block.toolCallId ?? index} block={block} />)}
    </div> : null;
  const renderRows = (rows: WorkRow[], part?: "work" | "outcome") => rows.map((row) => row.type === "switches"
    ? <Fragment key={`switch:${row.position}`}>{renderSwitches(row.position)}</Fragment>
    : <Fragment key={messages[row.index].id}>{row.type === "orphans"
      ? renderOrphans(messages[row.index], row.blocks) : row.type === "compaction"
        ? renderCompaction(messages[row.index]) : renderMessage(messages[row.index], row.slots, row.actions, part)}</Fragment>);
  const firstUserIndex = workTurns.keys().next().value ?? messages.length;

  const waiting = running && (Boolean(compaction) || !partial || partial.blocks.length === 0);
  const label = compaction ? "Compacting context…" : activityLabel(activity);

  const rewindBar = onUndoRewind && actionsEnabled ? (
    <div className="rewind-bar" role="status">
      <span>Rewound. The later messages are kept as another version.</span>
      <button type="button" className="secondary-button" onClick={onUndoRewind}><Icon name="rewind" /> Undo rewind</button>
    </div>
  ) : null;

  if (!hasVisibleMessages(messages) && !partial && !activeRun) {
    return (
      <div className="transcript-zone">
        <div className="conversation-scroll">
          <div className="conversation-empty">
            <h2>{empty?.title ?? "What should we build?"}</h2>
            <p>{empty?.body ?? <>Describe the change, bug, or question — {assistantName} can read this project, run commands, and edit files.</>}</p>
            {rewindBar}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="transcript-zone">
      {/* `layoutScroll`: chips flying into an Explored head are measured in page space, so the
          scroller's offset must be part of the measurement or follow-scrolling skews the flight. */}
      <motion.div className="conversation-scroll" ref={ref} onScroll={onScroll} onWheel={onWheel} layoutScroll>
        <ThinkingExpansion.Provider value={expandedThinking}>
          <ExploreExpansion.Provider value={expandedGroups}>
          <ToolExpansion.Provider value={toolExpansions}>
          <VisualCardScope.Provider value={visualScope}>
            <div className="transcript">
              {messages.map((message, messageIndex) => {
                const turn = workTurns.get(messageIndex);
                if (!turn) return messageIndex < firstUserIndex ? (
                  <Fragment key={message.id}>{renderSwitches(messageIndex)}
                    {message.role === "tool" ? renderOrphans(message) : renderMessage(message)}
                  </Fragment>
                ) : null;
                // The logical version key preserves focus on the user's version switcher.
                // The disclosure preference uses the actual entry/outcome instead.
                const key = message.versions?.group ?? message.id;
                const expansionKey = JSON.stringify([scopeKey, turn.key]);
                const duration = message.id === activeUserId ? <RunDuration startedAt={activeRun?.startedAt} />
                  : timingsByMessage.has(message.id) ? <RunDuration durationMs={timingsByMessage.get(message.id)} /> : null;
                return <Fragment key={key}>
                  {renderSwitches(messageIndex)}
                  <WorkTurn turn={turn} scopeKey={scopeKey} user={renderMessage(message)} duration={duration} durationMs={timingsByMessage.get(message.id)}
                    enabled={collapseCompletedWork} open={expandedWork.has(expansionKey)} renderRows={renderRows} scroll={scroll}
                    onAutoCollapse={() => setExpandedWork((current) => {
                      if (!current.has(expansionKey)) return current;
                      const next = new Set(current);
                      next.delete(expansionKey);
                      return next;
                    })}
                    onToggle={() => {
                      pauseFollowing();
                      setExpandedWork((current) => {
                        const next = new Set(current);
                        if (next.has(expansionKey)) next.delete(expansionKey); else next.add(expansionKey);
                        return next;
                      });
                    }} />
                </Fragment>;
              })}
              {renderSwitches(messages.length)}
              {partial && layout.partial?.length ? (
                <div className="msg assistant streaming">
                  {renderSlots(partial, layout.partial, results, liveToolText, liveToolDetails, true, planState, onPlanAction, running, skillCard, true)}
                </div>
              ) : null}
              {/* AnimatePresence must outlive the stage for its collapse exit to play. */}
              <AnimatePresence>
                {compaction ? <CompactingStage key="compacting" reason={compaction.reason} /> : null}
              </AnimatePresence>
              {!compaction && waiting && label && <WorkingPresence label={label} />}
              {rewindBar}
            </div>
          </VisualCardScope.Provider>
          </ToolExpansion.Provider>
          </ExploreExpansion.Provider>
        </ThinkingExpansion.Provider>
      </motion.div>
      <ScrollRail target={ref} turns={turns} liveTurnId={liveTurnId} detached={detached} onNavigate={pauseFollowing} onJumpToLatest={jumpToLatest} />
    </div>
  );
}
