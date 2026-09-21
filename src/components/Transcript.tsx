import { memo, useMemo, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { NormalizedBlock, NormalizedMessage } from "../types";
import { useFollowScroll } from "../hooks/useFollowScroll";
import { useSmoothText } from "../hooks/useSmoothText";
import { Icon } from "./Icons";
import { ThinkingRow } from "./ThinkingRow";
import { OrphanResult, ToolRow } from "./ToolRow";

interface Props {
  messages: NormalizedMessage[];
  partial?: NormalizedMessage;
  running: boolean;
  activity?: string;
}

function Markdown({ children }: { children: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      skipHtml
      components={{
        img: ({ alt }) => <span className="blocked-image">[Remote image blocked{alt ? `: ${alt}` : ""}]</span>,
        a: ({ href, children: linkChildren }) => <a href={href} target="_blank" rel="noreferrer">{linkChildren}</a>
      }}
    >{children}</ReactMarkdown>
  );
}

function StreamingText({ text }: { text: string }) {
  const shown = useSmoothText(text, true);
  return <div className="stream-text"><Markdown>{shown}</Markdown></div>;
}

function renderBlock(block: NormalizedBlock, results: Map<string, NormalizedBlock>, live: boolean, streaming?: boolean): ReactNode {
  if (block.type === "thinking") {
    return <ThinkingRow text={block.text ?? ""} streaming={streaming} />;
  }
  if (block.type === "tool-call") {
    const result = block.toolCallId ? results.get(block.toolCallId) : undefined;
    return <ToolRow call={block} result={result} running={live && !result} />;
  }
  if (block.type === "tool-result") {
    return <OrphanResult block={block} />;
  }
  if (!block.text) return null;
  if (streaming) return <StreamingText text={block.text} />;
  return <Markdown>{block.text}</Markdown>;
}

function signature(message: NormalizedMessage, results: Map<string, NormalizedBlock>): string {
  return JSON.stringify([
    message.blocks,
    message.stopReason,
    message.errorMessage,
    message.blocks.map((block) => block.type === "tool-call" ? (block.toolCallId ? results.get(block.toolCallId) ?? null : null) : null)
  ]);
}

interface MessageProps {
  message: NormalizedMessage;
  results: Map<string, NormalizedBlock>;
  live: boolean;
  sig: string;
}

const Message = memo(function Message({ message, results, live }: MessageProps) {
  if (message.role === "user") {
    return (
      <div className="msg user">
        <div className="bubble">{message.blocks.map((block) => block.text ?? "").join("\n").trim()}</div>
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
          {renderBlock(block, results, live)}
        </div>
      ))}
      {message.stopReason === "error" && <div className="message-error">{message.errorMessage || "The provider rejected the request."}</div>}
      {message.stopReason === "aborted" && <span className="aborted-label">Stopped</span>}
    </div>
  );
}, (prev, next) => prev.sig === next.sig && prev.live === next.live);

function activityLabel(activity?: string): string {
  if (!activity) return "Working…";
  if (activity.startsWith("tool_execution")) return "";
  if (activity.startsWith("compaction")) return "Compacting context…";
  if (activity.startsWith("auto_retry") || activity.startsWith("summarization_retry")) return "Retrying…";
  return "Working…";
}

export function Transcript({ messages, partial, running, activity }: Props) {
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
                sig={signature(message, results)}
              />
            );
          })}
          {partial && (
            <div className="msg assistant streaming">
              {partial.blocks.map((block, index) => (
                <div key={block.toolCallId ?? index} className="block-slot">
                  {renderBlock(block, results, true, true)}
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
