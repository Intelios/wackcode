/**
 * Chat mode's conversation: a messenger-style scroll of bubbles. It never renders what the
 * Code transcript is built around — no run clock, scroll rail, completed-work folds, plan or
 * sub-agent cards. Presence is the duck's typing bubble; tool calls are chips; settled thinking
 * is a caption you can open.
 *
 * Scrolling reuses `useFollowScroll` (bottom-pinned until the user scrolls up, per-chat
 * position in `TranscriptViewState`), and rows carry `data-transcript-anchor` so a restore
 * finds its place. Clusters and dividers come from `chat-clusters.ts`; the reply that
 * answers a user message is keyed by that message, so its rows survive the partial→saved swap.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { NormalizedBlock, NormalizedMessage, SessionSnapshot, TaskRecord } from "../../types";
import { useFollowScroll } from "../../hooks/useFollowScroll";
import { latestTurn, messageText } from "../../tree-utils";
import type { TranscriptViewState } from "../../transcript-view";
import { useNavigationDismiss } from "../ui/NavigationScope";
import { DuckMark } from "../DuckMark";
import type { MessageAction, MessageImageLoader } from "../Transcript";
import { AgentReply, CompactionNote, replyPieces, SystemBubble, TimeDivider, TypingBubble, TypingRow, UserBubble, type SystemNote, type TypingMood } from "./ChatBubbles";
import { clusterMessages, dividerLabel, type ChatEntry } from "./chat-clusters";
import { scratchpadFiles } from "./chat-scratchpad";

interface ChatConversationProps {
  task: TaskRecord;
  messages: NormalizedMessage[];
  partial?: NormalizedMessage;
  /** The parent run is working (background work alone doesn't make the duck type). */
  running: boolean;
  compaction?: SessionSnapshot["compaction"];
  viewState?: TranscriptViewState;
  historyReady: boolean;
  actionsEnabled: boolean;
  vision: boolean;
  modelName?: string;
  notes: SystemNote[];
  onMessageAction: (action: MessageAction) => Promise<boolean> | void;
  onReveal: (path: string) => void;
  loadImage?: MessageImageLoader;
}

type Local = MessageAction | { type: "start-edit"; id: string } | { type: "cancel-edit" };

function moodOf(partial: NormalizedMessage | undefined, compaction: boolean): { mood: TypingMood; thought?: string } {
  if (compaction) return { mood: "tidying" };
  const last = partial?.blocks[partial.blocks.length - 1];
  if (last?.type === "thinking" && last.durationMs === undefined) return { mood: "thinking", thought: last.text };
  if (last?.type === "tool-call") return { mood: "using" };
  return { mood: "typing" };
}

export function ChatConversation({ task, messages, partial, running, compaction, viewState, historyReady, actionsEnabled, vision, modelName, notes, onMessageAction, onReveal, loadImage }: ChatConversationProps) {
  const reduce = useReducedMotion() ?? false;
  const { ref, onScroll, onWheel, detached, jumpToLatest } = useFollowScroll(viewState, historyReady);
  const [editingId, setEditingId] = useState<string>();
  useNavigationDismiss(() => setEditingId(undefined));

  useEffect(() => {
    if (editingId && (!actionsEnabled || !messages.some((message) => message.id === editingId))) setEditingId(undefined);
  }, [actionsEnabled, editingId, messages]);

  const handle = useCallback(async (action: Local): Promise<boolean> => {
    if (action.type === "start-edit") { setEditingId(action.id); return true; }
    if (action.type === "cancel-edit") { setEditingId(undefined); return true; }
    const done = (await onMessageAction(action)) === true;
    if (action.type === "edit" && done) setEditingId(undefined);
    return done;
  }, [onMessageAction]);

  const results = useMemo(() => {
    const map = new Map<string, NormalizedBlock>();
    for (const message of [...messages, ...(partial ? [partial] : [])]) {
      for (const block of message.blocks) if (block.type === "tool-result" && block.toolCallId) map.set(block.toolCallId, block);
    }
    return map;
  }, [messages, partial]);

  const entries = useMemo(() => clusterMessages(messages), [messages]);
  const latest = useMemo(() => latestTurn(messages), [messages]);
  const lastUserIndex = useMemo(() => messages.map((message) => message.role).lastIndexOf("user"), [messages]);
  const lastEntry = entries[entries.length - 1];
  // The live reply is the last reply after the latest user message, or a new one.
  const liveReply = running && lastEntry?.type === "reply" ? lastEntry : undefined;
  const { mood, thought } = moodOf(partial, Boolean(compaction));
  const partialHasWords = Boolean(partial?.blocks.some((block) => block.type === "text" && block.text?.trim()));
  const showTyping = running && (!partialHasWords || Boolean(compaction));

  // New words while scrolled up: the jump pill wiggles to say so.
  const [unseen, setUnseen] = useState(false);
  const length = messages.length + (partial?.blocks.length ?? 0);
  const seenLength = useRef(length);
  useEffect(() => {
    if (!detached) { setUnseen(false); seenLength.current = length; return; }
    if (length > seenLength.current) setUnseen(true);
  }, [length, detached]);

  const renderReply = (entry: Extract<ChatEntry, { type: "reply" }>, live: boolean) => {
    const pieces = replyPieces(entry.messages, live ? partial : undefined);
    const calls = [...entry.messages, ...(live && partial ? [partial] : [])].flatMap((message) => message.blocks.filter((block) => block.type === "tool-call"));
    const answer = [...entry.messages].reverse().find((message) => message.role === "assistant" && message.turn);
    const copyText = entry.messages.filter((message) => message.role === "assistant").map(messageText).filter(Boolean).join("\n\n");
    const isLatest = answer !== undefined && answer.id === latest?.answer?.id;
    return (
      <AgentReply
        key={entry.key}
        anchor={entry.key}
        pieces={pieces}
        results={results}
        first={entry.first}
        live={live}
        files={live ? [] : scratchpadFiles(calls, results, task.workspacePath)}
        copyText={copyText || undefined}
        onCopy={() => onMessageAction({ type: "copy-prompt", text: copyText })}
        onRetry={!live && isLatest && actionsEnabled ? () => void onMessageAction({ type: "retry", message: answer! }) : undefined}
        onReveal={onReveal}
        typing={live ? <AnimatePresence>{showTyping && <TypingBubble key="typing" mood={mood} thought={thought} />}</AnimatePresence> : undefined}
      />
    );
  };

  return (
    <div className="chat-scroll-zone">
      <div className="chat-scroll" ref={ref} onScroll={onScroll} onWheel={onWheel}>
        <div className="chat-thread">
          {entries.map((entry, index) => {
            if (entry.type === "divider") return <TimeDivider key={entry.key} label={dividerLabel(entry.at)} />;
            if (entry.type === "compaction") return <CompactionNote key={entry.key} message={entry.message} />;
            if (entry.type === "system") {
              const text = messageText(entry.message) || entry.message.blocks.map((block) => block.text ?? "").join("\n").trim();
              return text ? <div key={entry.key} className="chat-divider quiet" data-transcript-anchor={entry.key}><span>{text}</span></div> : null;
            }
            if (entry.type === "user") {
              const next = entries[index + 1];
              const answer = next?.type === "reply" ? next.messages.find((message) => message.stopReason === "aborted") : undefined;
              return (
                <UserBubble key={entry.key} message={entry.message} first={entry.first} actionsEnabled={actionsEnabled}
                  retry={entry.index === lastUserIndex && entry.message.id === latest?.user.id && !latest?.answer}
                  editing={editingId === entry.message.id} vision={vision} modelName={modelName}
                  stopped={Boolean(answer)} onAction={handle} loadImage={loadImage} />
              );
            }
            return renderReply(entry, entry === liveReply);
          })}
          {/* The duck starts a fresh reply: a typing row (and the first words) until the
              worker saves the message it's writing. */}
          {running && !liveReply && (
            partial && partialHasWords
              ? renderReply({ type: "reply", key: lastEntry?.type === "user" ? `reply-to:${lastEntry.key}` : "reply:live", messages: [], first: true }, true)
              : <TypingRow key="typing-row" mood={mood} thought={thought} first />
          )}
          <AnimatePresence initial={false}>
            {notes.map((note) => <SystemBubble key={note.key} note={note} />)}
          </AnimatePresence>
        </div>
      </div>
      <AnimatePresence>
        {detached && (
          <motion.button
            key="jump"
            type="button"
            className={`chat-jump${unseen ? " unseen" : ""}`}
            onClick={jumpToLatest}
            initial={reduce ? { opacity: 0 } : { opacity: 0, y: 16, scale: 0.8 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={reduce ? { opacity: 0 } : { opacity: 0, y: 12, scale: 0.85 }}
            transition={{ type: "spring", stiffness: 480, damping: 28 }}
          >
            <DuckMark className="chat-jump-duck" />
            {unseen ? "New messages" : "Jump to latest"}
            <span aria-hidden="true" className="chat-jump-arrow">↓</span>
          </motion.button>
        )}
      </AnimatePresence>
    </div>
  );
}

