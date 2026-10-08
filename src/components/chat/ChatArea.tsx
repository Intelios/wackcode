/**
 * The whole Chat area: what App renders in place of the Code chat view and composer layer when
 * `area === "chat"`. It is fed one bundle of state and callbacks App already owns (sends,
 * drafts, message actions, the browser toggle), so nothing here reaches the bridge except the
 * scratchpad reveal App hands in. The tab bar, side panel and Settings stay App's chrome.
 *
 * Layout: the conversation (header + bubbles) or the hero stage, with one `ChatComposer` in a
 * layer above that glides from the hero slot to the dock. The first send keeps its words in
 * the frozen pill only until the chat exists; from then the pending echo is the first bubble,
 * springing up from the composer, while the duck dives off the hero.
 *
 * Chat mode never shows the Code UI's run clock, scroll rail, completed-work folds, planning
 * modes, slash commands, mentions, todo/goal banners or Git: see docs/frontend.md › Areas.
 */
import { useCallback, useMemo, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { ImageContent, NormalizedMessage, ProviderRecord, TaskRecord, TaskRuntime, ThinkingLevel } from "../../types";
import type { FileAttachment } from "../../attachment-utils";
import type { ComposerDraftState } from "../../hooks/useComposerDrafts";
import type { TranscriptViewState } from "../../transcript-view";
import type { ModelFavoritesProps } from "../ModelPicker";
import type { MessageAction, MessageImageLoader } from "../Transcript";
import type { TaskAction } from "../Sidebar";
import { Icon } from "../Icons";
import { ChatAreaHeader } from "./ChatAreaHeader";
import { ChatComposer, type ChatComposerHandle } from "./ChatComposer";
import { ChatConversation } from "./ChatConversation";
import { ChatHero } from "./ChatHero";
import { EASE, type SystemNote } from "./ChatBubbles";

export interface ChatAreaProps extends ModelFavoritesProps {
  task?: TaskRecord;
  runtime?: TaskRuntime;
  /** The transcript's messages, pending echo included. */
  messages: NormalizedMessage[];
  /** The parent run is working; background-only work doesn't make the duck type. */
  running: boolean;
  viewState?: TranscriptViewState;
  actionsEnabled: boolean;
  vision: boolean;
  modelName?: string;
  /** Errors, model problems and notices, as system bubbles. */
  notes: SystemNote[];
  /** The agent's questions (InlineDialog), rendered above the composer. */
  dialogs?: ReactNode;
  browserOpen: boolean;
  titlePulse?: number;
  agentName: string;
  providers: ProviderRecord[];
  providerId?: string;
  modelId?: string;
  thinkingLevel?: ThinkingLevel;
  draftState: ComposerDraftState;
  composerDisabled: boolean;
  /** The first send's words, while the hero hands them to the new chat. */
  handoff?: string;
  seed?: { text: string; nonce: number };
  /** Tab panel wiring when chat tabs are on. */
  panel?: { id: string; labelledBy?: string };
  onMessageAction: (action: MessageAction) => Promise<boolean> | void;
  onReveal: (path: string) => void;
  loadImage?: MessageImageLoader;
  onToggleBrowser: () => void;
  onRename: (name: string) => void;
  onTaskAction: (task: TaskRecord, action: TaskAction) => void;
  onConfigure: (patch: { providerId?: string; modelId?: string; thinkingLevel?: ThinkingLevel }) => void;
  onSend: (message: string, images: ImageContent[], files: FileAttachment[], queue?: boolean) => Promise<boolean>;
  onSteer: (messageId: string) => Promise<boolean>;
  onDequeue: () => Promise<string[] | undefined>;
  onStop: () => void;
  onOpenSettings: () => void;
}

function hashSeed(text: string): number {
  let hash = 7;
  for (let index = 0; index < text.length; index += 1) hash = (hash * 31 + text.charCodeAt(index)) | 0;
  return Math.abs(hash) + 1;
}

export function ChatArea(props: ChatAreaProps) {
  const { task, runtime, messages, running, notes, draftState, handoff } = props;
  const reduce = useReducedMotion() ?? false;
  const composer = useRef<ChatComposerHandle>(null);
  const [typing, setTyping] = useState(false);
  const onTyping = useCallback((value: boolean) => setTyping(value), []);
  const partial = runtime?.partial;
  const last = partial?.blocks[partial.blocks.length - 1];
  const thinking = running && last?.type === "thinking" && last.durationMs === undefined;
  const heroSeed = useMemo(() => hashSeed(draftState.key), [draftState.key]);
  const archived = task?.archived === true;

  return (
    <div className={`chat-area ${task ? "dock" : "hero"}`} role={props.panel ? "tabpanel" : undefined} id={props.panel?.id} aria-labelledby={props.panel?.labelledBy}>
      <AnimatePresence initial={false}>
        {task ? (
          <motion.div key={`chat:${task.id}`} className="chat-area-view"
            initial={reduce ? false : { opacity: 0 }} animate={{ opacity: 1 }}
            exit={{ opacity: 0, transition: { duration: reduce ? 0 : 0.16, ease: EASE } }} transition={{ duration: 0.3, ease: EASE }}>
            <motion.div initial={reduce ? false : { opacity: 0, y: -24 }} animate={{ opacity: 1, y: 0 }} transition={{ type: "spring", stiffness: 380, damping: 30, delay: 0.04 }}>
              <ChatAreaHeader task={task} live={running} thinking={thinking} browserOpen={props.browserOpen} onToggleBrowser={props.onToggleBrowser}
                onRename={props.onRename} onTaskAction={props.onTaskAction} titlePulse={props.titlePulse} />
            </motion.div>
            <ChatConversation
              key={task.id}
              task={task}
              messages={messages}
              partial={partial}
              running={running}
              compaction={runtime?.compaction}
              viewState={props.viewState}
              historyReady={runtime?.snapshot !== undefined}
              actionsEnabled={props.actionsEnabled}
              vision={props.vision}
              modelName={props.modelName}
              notes={notes}
              onMessageAction={props.onMessageAction}
              onReveal={props.onReveal}
              loadImage={props.loadImage}
            />
            {props.dialogs}
          </motion.div>
        ) : null}
      </AnimatePresence>
      {/* One composer for both slots: the layer swaps from centred to docked and the pill
          layout-glides between them, with the hero stage leaving above it. */}
      <div className={`chat-composer-layer ${task ? "dock" : "hero"}`}>
        <AnimatePresence initial={false} mode="popLayout">
          {!task && (
            <motion.div key="hero" className="chat-area-hero"
              initial={reduce ? false : { opacity: 0 }} animate={{ opacity: 1 }}
              exit={{ opacity: 0, transition: { duration: reduce ? 0 : 0.35, ease: EASE } }}>
              <ChatHero typing={typing} leaving={handoff !== undefined} seed={heroSeed} onIdea={(idea) => composer.current?.fill(idea.prompt)} />
            </motion.div>
          )}
        </AnimatePresence>
        {archived && task ? (
          <motion.div className="chat-archived" role="status" initial={reduce ? false : { opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }}>
            <Icon name="archive" /> This chat is archived.
            <button type="button" className="secondary-button compact" onClick={() => props.onTaskAction(task, "unarchive")}>Unarchive</button>
          </motion.div>
        ) : (
          <ChatComposer
            ref={composer}
            hero={!task}
            draftState={draftState}
            status={task?.status ?? "idle"}
            backgroundWorking={task?.status === "running" && runtime?.workActivity?.parent === "idle"}
            providers={props.providers}
            providerId={props.providerId}
            modelId={props.modelId}
            thinkingLevel={props.thinkingLevel}
            favoriteModels={props.favoriteModels}
            favoriteSaving={props.favoriteSaving}
            onSetFavorite={props.onSetFavorite}
            placeholder={task ? `Message ${props.agentName}…` : "Ask anything…"}
            agentName={props.agentName}
            disabled={props.composerDisabled}
            frozen={task ? undefined : handoff}
            queuedMessages={runtime?.queued}
            seed={props.seed}
            onConfigure={props.onConfigure}
            onSend={props.onSend}
            onSteer={props.onSteer}
            onDequeue={props.onDequeue}
            onStop={props.onStop}
            onOpenSettings={props.onOpenSettings}
            onTyping={onTyping}
          />
        )}
      </div>
    </div>
  );
}
