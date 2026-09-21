import { useEffect, useRef } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { NormalizedBlock, NormalizedMessage } from "../types";
import { Icon } from "./Icons";

interface Props {
  messages: NormalizedMessage[];
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

function ToolBlock({ block }: { block: NormalizedBlock }) {
  const isResult = block.type === "tool-result";
  return (
    <details className={`tool-block ${block.isError ? "error" : ""}`}>
      <summary>
        <span className="tool-icon">{isResult ? "↳" : "⌘"}</span>
        <strong>{block.toolName || (isResult ? "Tool result" : "Tool call")}</strong>
        {block.isError && <span className="tool-error-label">failed</span>}
        <Icon name="chevron" />
      </summary>
      <pre>{isResult ? block.text : JSON.stringify(block.arguments ?? {}, null, 2)}</pre>
    </details>
  );
}

function Message({ message }: { message: NormalizedMessage }) {
  if (message.role === "tool") return <div className="tool-message">{message.blocks.map((block, index) => <ToolBlock key={index} block={block} />)}</div>;
  return (
    <article className={`message ${message.role}`}>
      <div className="message-label">{message.role === "user" ? "You" : message.role === "assistant" ? "WackCode" : "System"}</div>
      <div className="message-body">
        {message.blocks.map((block, index) => {
          if (block.type === "thinking") return <details className="thinking-block" key={index}><summary><Icon name="spark" /> Reasoning <Icon name="chevron" /></summary><div><Markdown>{block.text ?? ""}</Markdown></div></details>;
          if (block.type === "tool-call" || block.type === "tool-result") return <ToolBlock block={block} key={index} />;
          return <Markdown key={index}>{block.text ?? ""}</Markdown>;
        })}
        {message.stopReason === "error" && <div className="message-error">{message.errorMessage || "The provider rejected the request."}</div>}
        {message.stopReason === "aborted" && <span className="aborted-label">Stopped</span>}
      </div>
    </article>
  );
}

export function Transcript({ messages, running, activity }: Props) {
  const endRef = useRef<HTMLDivElement>(null);
  const previousCount = useRef(0);
  useEffect(() => {
    if (messages.length !== previousCount.current || running) {
      endRef.current?.scrollIntoView({ block: "end", behavior: previousCount.current ? "smooth" : "auto" });
      previousCount.current = messages.length;
    }
  }, [messages, running]);

  if (messages.length === 0) {
    return (
      <div className="conversation-empty">
        <div className="empty-mark"><span>W</span></div>
        <h2>What should we build?</h2>
        <p>Pi can read this project, run commands, and edit files. Its work will appear in the changes panel as it happens.</p>
      </div>
    );
  }

  return (
    <div className="transcript">
      {messages.map((message) => <Message key={message.id} message={message} />)}
      {running && <div className="agent-working"><span className="pulse-dot" /> {activity ? activity.replaceAll("_", " ") : "Pi is working"}</div>}
      <div ref={endRef} />
    </div>
  );
}
