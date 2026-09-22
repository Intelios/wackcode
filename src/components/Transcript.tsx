import { memo, useMemo, type ReactNode } from "react";
import type { NormalizedBlock, NormalizedMessage, PlanState } from "../types";
import { useFollowScroll } from "../hooks/useFollowScroll";
import { useSmoothText } from "../hooks/useSmoothText";
import { Icon } from "./Icons";
import { Markdown } from "./Markdown";
import { PlanCard, type PlanAction } from "./PlanCard";
import { ThinkingRow } from "./ThinkingRow";
import { OrphanResult, ToolRow } from "./ToolRow";

interface Props {
  messages: NormalizedMessage[];
  partial?: NormalizedMessage;
  running: boolean;
  activity?: string;
  /** Latest Plan mode state; PlanCards use it to know which proposal is awaiting a decision. */
  planState?: PlanState;
  onPlanAction?: (action: PlanAction) => void;
}

function StreamingText({ text }: { text: string }) {
  const shown = useSmoothText(text, true);
  return <div className="stream-text"><Markdown>{shown}</Markdown></div>;
}

/** The plan text a plan_mode_complete result carries, or undefined if it isn't one. */
function completedPlan(result?: NormalizedBlock): string | undefined {
  const details = result?.details as { plan?: unknown } | undefined;
  return typeof details?.plan === "string" && details.plan.trim() ? details.plan : undefined;
}

function renderBlock(
  block: NormalizedBlock,
  results: Map<string, NormalizedBlock>,
  live: boolean,
  planState: PlanState | undefined,
  onPlanAction: ((action: PlanAction) => void) | undefined,
  running: boolean,
  streaming?: boolean
): ReactNode {
  if (block.type === "thinking") {
    return <ThinkingRow text={block.text ?? ""} streaming={streaming} />;
  }
  if (block.type === "tool-call") {
    const result = block.toolCallId ? results.get(block.toolCallId) : undefined;
    const plan = block.toolName === "plan_mode_complete" ? completedPlan(result) : undefined;
    if (plan !== undefined) {
      const current = planState?.mode === "plan" && planState.phase === "ready" && planState.plan === plan;
      return <PlanCard plan={plan} current={current} busy={running} onAction={onPlanAction} />;
    }
    return <ToolRow call={block} result={result} running={live && !result} />;
  }
  if (block.type === "tool-result") {
    return <OrphanResult block={block} />;
  }
  if (!block.text) return null;
  if (streaming) return <StreamingText text={block.text} />;
  return <Markdown>{block.text}</Markdown>;
}

// Recomputed for every message on every streamed partial, so image blocks stand in as their id
// and readiness rather than their preview data.
function signatureBlock(block: NormalizedBlock): unknown {
  return block.type === "image" ? { image: block.imageId, ready: Boolean(block.thumbnail) } : block;
}

function signature(message: NormalizedMessage, results: Map<string, NormalizedBlock>): string {
  return JSON.stringify([
    message.blocks.map(signatureBlock),
    message.stopReason,
    message.errorMessage,
    message.blocks.map((block) => block.type === "tool-call" ? (block.toolCallId ? results.get(block.toolCallId) ?? null : null) : null)
  ]);
}

interface MessageProps {
  message: NormalizedMessage;
  results: Map<string, NormalizedBlock>;
  live: boolean;
  running: boolean;
  planState?: PlanState;
  onPlanAction?: (action: PlanAction) => void;
  sig: string;
}

const Message = memo(function Message({ message, results, live, running, planState, onPlanAction }: MessageProps) {
  if (message.role === "user") {
    const images = message.blocks.filter((block) => block.type === "image");
    const text = message.blocks.filter((block) => block.type === "text").map((block) => block.text ?? "").join("\n").trim();
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
        {text && <div className="bubble">{text}</div>}
      </div>
    );
  }
  if (message.role === "system") {
    const text = message.blocks.map((block) => block.text ?? "").join("\n").trim();
    return text ? <div className="msg system">{text}</div> : null;
  }
  return (
    <div className="msg assistant">
      {message.blocks.map((block, index) => (
        <div key={block.toolCallId ?? index} className="block-slot">
          {renderBlock(block, results, live, planState, onPlanAction, running)}
        </div>
      ))}
      {message.stopReason === "error" && <div className="message-error">{message.errorMessage || "The provider rejected the request."}</div>}
      {message.stopReason === "aborted" && <span className="aborted-label">Stopped</span>}
    </div>
  );
}, (prev, next) =>
  prev.sig === next.sig && prev.live === next.live && prev.running === next.running
  && prev.planState === next.planState && prev.onPlanAction === next.onPlanAction);

function activityLabel(activity?: string): string {
  if (!activity) return "Working…";
  if (activity.startsWith("tool_execution")) return "";
  if (activity.startsWith("compaction")) return "Compacting context…";
  if (activity.startsWith("auto_retry") || activity.startsWith("summarization_retry")) return "Retrying…";
  return "Working…";
}

export function Transcript({ messages, partial, running, activity, planState, onPlanAction }: Props) {
  const { ref, onScroll, detached, jumpToLatest } = useFollowScroll();

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

  const waiting = running && (!partial || partial.blocks.length === 0);
  const label = activityLabel(activity);

  if (messages.length === 0 && !partial) {
    return (
      <div className="transcript-zone">
        <div className="conversation-scroll">
          <div className="conversation-empty">
            <h2>What should we build?</h2>
            <p>Describe the change, bug, or question — Pi can read this project, run commands, and edit files.</p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="transcript-zone">
      <div className="conversation-scroll" ref={ref} onScroll={onScroll}>
        <div className="transcript">
          {messages.map((message) => {
            if (message.role === "tool") {
              const orphans = message.blocks.filter((block) => !block.toolCallId || !callIds.has(block.toolCallId));
              if (orphans.length === 0) return null;
              return <div key={message.id} className="orphan-group">{orphans.map((block, index) => <OrphanResult key={index} block={block} />)}</div>;
            }
            return (
              <Message
                key={message.id}
                message={message}
                results={results}
                live={running && message.id === lastAssistantId}
                running={running}
                planState={planState}
                onPlanAction={onPlanAction}
                sig={signature(message, results)}
              />
            );
          })}
          {partial && (
            <div className="msg assistant streaming">
              {partial.blocks.map((block, index) => (
                <div key={block.toolCallId ?? index} className="block-slot">
                  {renderBlock(block, results, true, planState, onPlanAction, running, true)}
                </div>
              ))}
            </div>
          )}
          {waiting && label && (
            <div className="agent-working"><span className="thinking-shimmer">{label}</span></div>
          )}
        </div>
      </div>
      {detached && (
        <button type="button" className="jump-latest" onClick={jumpToLatest}>
          <Icon name="chevron" style={{ transform: "rotate(90deg)" }} /> Latest
        </button>
      )}
    </div>
  );
}
