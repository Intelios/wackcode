import { Fragment, memo, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import type { NormalizedBlock, NormalizedMessage, PlanState, RunTiming } from "../types";
import { useFollowScroll } from "../hooks/useFollowScroll";
import { useSmoothText } from "../hooks/useSmoothText";
import { formatRunDuration, isPlanMode } from "../chat-utils";
import { blockKey, layoutTranscript, type TranscriptSlot } from "../explore-utils";
import { splitMentions } from "../mention-utils";
import { SUBAGENT_TOOL_NAME, parseSubagentDetails, pendingSubagentDetails } from "../tool-utils";
import { hasVisibleMessages, latestTurn, messageText } from "../tree-utils";
import { ExploreExpansion, ExploreGroup, ExploreGroupingEnabled } from "./ExploreGroup";
import { Icon } from "./Icons";
import { Markdown } from "./Markdown";
import { MessageActions, type MessageActionItem } from "./MessageActions";
import { MessageEditor } from "./MessageEditor";
import { PlanCard, type PlanAction } from "./PlanCard";
import { SubagentCard } from "./SubagentCard";
import { ThinkingExpansion, ThinkingRow } from "./ThinkingRow";
import { OrphanResult, ToolRow } from "./ToolRow";

interface Props {
  messages: NormalizedMessage[];
  partial?: NormalizedMessage;
  running: boolean;
  activity?: string;
  activeRun?: { runId?: string; startedAt: number };
  runTimings?: RunTiming[];
  liveToolText?: Record<string, string>;
  /** Structured progress of in-flight tools that report it (the sub-agent card). */
  liveToolDetails?: Record<string, unknown>;
  /** Latest Plan mode state; PlanCards use it to know which proposal is awaiting a decision. */
  planState?: PlanState;
  onPlanAction?: (action: PlanAction) => void;
  /** Retry, edit, rewind, version switching and fork are offered only while this is true. */
  actionsEnabled?: boolean;
  /** Whether the chat's model accepts images, for editing a message that has some. */
  vision?: boolean;
  modelName?: string;
  /** Resolves true once an edit has been sent, which closes the editor. */
  onMessageAction?: (action: MessageAction) => Promise<boolean> | void;
  /** Offered right after a rewind. */
  onUndoRewind?: () => void;
}

/** What the transcript asks the app to do with a message. */
export type MessageAction =
  | { type: "copy"; message: NormalizedMessage }
  | { type: "edit"; message: NormalizedMessage; text: string; removeImages: number[] }
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
  return <div className="stream-text assistant-text"><Markdown>{shown}</Markdown></div>;
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

  return <div className="run-duration">{label} {formatRunDuration(elapsed)}</div>;
}

/** The plan text a plan_mode_complete result carries, or undefined if it isn't one. */
function completedPlan(result?: NormalizedBlock): string | undefined {
  const details = result?.details as { plan?: unknown } | undefined;
  return typeof details?.plan === "string" && details.plan.trim() ? details.plan : undefined;
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
  streaming?: boolean
): ReactNode {
  if (block.type === "thinking") {
    // A streamed block is still being written until the worker has clocked its end.
    return <ThinkingRow text={block.text ?? ""} durationMs={block.durationMs} live={streaming === true && block.durationMs === undefined} expansionKey={key} />;
  }
  if (block.type === "tool-call") {
    const result = block.toolCallId ? results.get(block.toolCallId) : undefined;
    const liveText = block.toolCallId ? liveToolText?.[block.toolCallId] : undefined;
    const plan = block.toolName === "plan_mode_complete" ? completedPlan(result) : undefined;
    if (plan !== undefined) {
      const current = isPlanMode(planState?.mode) && planState?.phase === "ready" && planState.plan === plan;
      return <PlanCard plan={plan} current={current} busy={running} onAction={onPlanAction} />;
    }
    if (block.toolName === SUBAGENT_TOOL_NAME) {
      // The final result is authoritative; while running, the latest live update, or the
      // call's own arguments until the first update arrives. A call refused before any child
      // started has no details and falls through to the plain row with its error.
      const liveDetails = block.toolCallId ? liveToolDetails?.[block.toolCallId] : undefined;
      const details = result
        ? parseSubagentDetails(result.details)
        : live ? parseSubagentDetails(liveDetails) ?? pendingSubagentDetails(block) : undefined;
      if (details) return <SubagentCard details={details} running={live && !result} />;
    }
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
  streaming?: boolean
): ReactNode {
  return slots.map((slot) => {
    if (slot.type === "explore") {
      return (
        <div key={`explore:${slot.group.key}`} className="block-slot">
          <ExploreGroup group={slot.group} results={results} liveToolText={liveToolText} />
        </div>
      );
    }
    const block = message.blocks[slot.index];
    return (
      <div key={block.toolCallId ?? slot.index} className="block-slot">
        {renderBlock(block, blockKey(message, slot.index), results, liveToolText, liveToolDetails, live, planState, onPlanAction, running, streaming)}
      </div>
    );
  });
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
const signatureCache = new WeakMap<NormalizedMessage, { deps: unknown[]; sig: string }>();

function cachedSignature(
  message: NormalizedMessage,
  slots: TranscriptSlot[],
  results: Map<string, NormalizedBlock>,
  liveToolText?: Record<string, string>,
  liveToolDetails?: Record<string, unknown>
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
  const cached = signatureCache.get(message);
  if (cached && cached.deps.length === deps.length && cached.deps.every((dep, index) => dep === deps[index])) {
    return cached.sig;
  }
  const sig = signature(message, slots, results, liveToolText, liveToolDetails);
  signatureCache.set(message, { deps, sig });
  return sig;
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
  sig: string;
  actionsEnabled: boolean;
  /** Offer Retry on this message: it ends (or, unanswered, is) the latest turn. */
  retry: boolean;
  editing: boolean;
  vision: boolean;
  modelName?: string;
  onAction: (action: LocalAction) => Promise<boolean> | void;
}

const Message = memo(function Message({ message, slots, results, liveToolText, liveToolDetails, live, running, planState, onPlanAction, actionsEnabled, retry, editing, vision, modelName, onAction }: MessageProps) {
  if (message.role === "user") {
    const images = message.blocks.filter((block) => block.type === "image");
    const text = messageText(message);
    if (editing) {
      return (
        <div className="msg user editing">
          <MessageEditor
            text={text}
            images={images}
            vision={vision}
            modelName={modelName}
            onCancel={() => void onAction({ type: "cancel-edit" })}
            onSend={async (edited, removeImages) => (await onAction({ type: "edit", message, text: edited, removeImages })) === true}
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
      <div className="msg user">
        {images.length > 0 && (
          <div className="message-images">
            {images.map((image, index) => image.thumbnail
              ? <img key={image.imageId ?? index} src={image.thumbnail} alt={`Attached image ${index + 1}`} />
              : <div key={image.imageId ?? index} className="image-pending" role="img" aria-label={`Attached image ${index + 1}`}><Icon name="image" /></div>
            )}
          </div>
        )}
        {text && <div className="bubble">{splitMentions(text).map((segment, index) => segment.mention ? <span key={index} className="mention">{segment.text}</span> : segment.text)}</div>}
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
    return text ? <div className="msg system">{text}</div> : null;
  }
  const failed = message.stopReason === "error" || message.stopReason === "aborted";
  // Everything it had belongs to an exploration group an earlier message shows.
  if (slots.length === 0 && !failed && !message.turn) return null;
  return (
    <div className="msg assistant">
      {renderSlots(message, slots, results, liveToolText, liveToolDetails, live, planState, onPlanAction, running)}
      {message.stopReason === "error" && <div className="message-error">{message.errorMessage || "The provider rejected the request."}</div>}
      {message.stopReason === "aborted" && <span className="aborted-label">Stopped</span>}
      {message.turn && <TurnActions message={message} actionsEnabled={actionsEnabled} retry={retry} onAction={onAction} />}
    </div>
  );
}, (prev, next) =>
  prev.sig === next.sig && prev.live === next.live && prev.running === next.running
  && prev.planState === next.planState && prev.onPlanAction === next.onPlanAction
  && prev.actionsEnabled === next.actionsEnabled && prev.retry === next.retry && prev.editing === next.editing
  && prev.vision === next.vision && prev.modelName === next.modelName && prev.onAction === next.onAction);

function TurnActions({ message, actionsEnabled, retry, onAction }: Pick<MessageProps, "message" | "actionsEnabled" | "retry" | "onAction">) {
  const items: MessageActionItem[] = [];
  if (messageText(message)) items.push({ id: "copy", label: "Copy", icon: "copy", onClick: () => void onAction({ type: "copy", message }) });
  if (actionsEnabled) {
    if (retry) items.push({ id: "retry", label: "Retry", icon: "refresh", onClick: () => void onAction({ type: "retry", message }) });
    items.push({ id: "fork", label: "Fork from here", icon: "branch", onClick: () => void onAction({ type: "fork", message }) });
  }
  return <MessageActions align="start" items={items} />;
}

function activityLabel(activity?: string): string {
  if (!activity) return "Working…";
  if (activity.startsWith("tool_execution")) return "";
  if (activity.startsWith("compaction")) return "Compacting context…";
  if (activity === "mcp_connect_start") return "Starting MCP servers…";
  if (activity.startsWith("auto_retry") || activity.startsWith("summarization_retry")) return "Retrying…";
  return "Working…";
}

export function Transcript({ messages, partial, running, activity, activeRun, runTimings = [], liveToolText, liveToolDetails, planState, onPlanAction, actionsEnabled = false, vision = false, modelName, onMessageAction, onUndoRewind }: Props) {
  const { ref, onScroll, detached, jumpToLatest } = useFollowScroll();
  const [editingId, setEditingId] = useState<string>();
  const [expandedThinking] = useState(() => new Set<string>());
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

  const lastAssistantId = useMemo(() => [...messages].reverse().find((message) => message.role === "assistant")?.id, [messages]);

  const grouping = useContext(ExploreGroupingEnabled);
  const [expandedGroups] = useState(() => new Set<string>());
  const layout = useMemo(
    () => layoutTranscript(messages, partial, { grouping, liveMessageId: running ? lastAssistantId : undefined }),
    [messages, partial, grouping, running, lastAssistantId]
  );

  const activeUserId = useMemo(() => activeRun
    ? [...messages].reverse().find((message) => message.role === "user" && message.timestamp !== undefined && message.timestamp >= activeRun.startedAt)?.id
    : undefined, [messages, activeRun]);
  const timingsByMessage = useMemo(() => new Map(runTimings.map((timing) => [timing.userMessageId, timing.durationMs])), [runTimings]);

  const waiting = running && (!partial || partial.blocks.length === 0);
  const label = activityLabel(activity);

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
            <h2>What should we build?</h2>
            <p>Describe the change, bug, or question — Pi can read this project, run commands, and edit files.</p>
            {rewindBar}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="transcript-zone">
      <div className="conversation-scroll" ref={ref} onScroll={onScroll}>
        <ThinkingExpansion.Provider value={expandedThinking}>
          <ExploreExpansion.Provider value={expandedGroups}>
            <div className="transcript">
              {messages.map((message) => {
                if (message.role === "tool") {
                  const orphans = message.blocks.filter((block) => !block.toolCallId || !callIds.has(block.toolCallId));
                  if (orphans.length === 0) return null;
                  return <div key={message.id} className="orphan-group">{orphans.map((block, index) => <OrphanResult key={index} block={block} />)}</div>;
                }
                // Every version of a user message shares one element, so the switcher keeps focus.
                const key = message.role === "user" && message.versions ? message.versions.group : message.id;
                const slots = layout.messages.get(message.id) ?? NO_SLOTS;
                const retry = message.id === (latest?.answer ?? latest?.user)?.id;
                return (
                  <Fragment key={key}>
                    <Message
                      message={message}
                      slots={slots}
                      results={results}
                      liveToolText={liveToolText}
                      liveToolDetails={liveToolDetails}
                      live={running && message.id === lastAssistantId}
                      running={running}
                      planState={planState}
                      onPlanAction={onPlanAction}
                      sig={cachedSignature(message, slots, results, liveToolText, liveToolDetails)}
                      actionsEnabled={actionsEnabled}
                      retry={retry}
                      editing={editingId === message.id}
                      vision={vision}
                      modelName={modelName}
                      onAction={handleAction}
                    />
                    {message.role === "user" && message.id === activeUserId && (
                      <RunDuration startedAt={activeRun?.startedAt} />
                    )}
                    {message.role === "user" && message.id !== activeUserId && timingsByMessage.has(message.id) && (
                      <RunDuration durationMs={timingsByMessage.get(message.id)} />
                    )}
                  </Fragment>
                );
              })}
              {activeRun && !activeUserId && <RunDuration startedAt={activeRun.startedAt} />}
              {partial && layout.partial?.length ? (
                <div className="msg assistant streaming">
                  {renderSlots(partial, layout.partial, results, liveToolText, liveToolDetails, true, planState, onPlanAction, running, true)}
                </div>
              ) : null}
              {waiting && label && (
                <div className="agent-working"><span className="thinking-shimmer">{label}</span></div>
              )}
              {rewindBar}
            </div>
          </ExploreExpansion.Provider>
        </ThinkingExpansion.Provider>
      </div>
      {detached && (
        <button type="button" className="jump-latest" onClick={jumpToLatest}>
          <Icon name="chevron" style={{ transform: "rotate(90deg)" }} /> Latest
        </button>
      )}
    </div>
  );
}
