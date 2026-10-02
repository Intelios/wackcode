/**
 * Shared react-markdown component overrides, used by `Markdown.tsx` for the transcript, plans,
 * sub-agent output and thinking rows. `pre` is where syntax highlighting happens: the fenced
 * block's `code` child is re-rendered from highlight.js tokens (src/highlight.ts), with a
 * hover-revealed copy action using the original text. Inline code needs no override at all. The img override is
 * the original from Markdown.tsx; the a override intercepts clicks (the webview cannot open
 * links itself) and routes http(s) URLs
 * through `api.revealPath`.
 *
 * `streamingTail` gates streaming: it carries the content of a fence that is still being
 * written (openFenceTail), and any block matching it renders plain so a growing fence neither
 * re-tokenises on every smooth-text frame nor pops into colour mid-write. Settled text has no
 * tail and always highlights.
 */

import type { ReactNode } from "react";
import type { Element, ElementContent } from "hast";
import type { Components } from "react-markdown";
import { api } from "./api";
import { highlightBlock } from "./highlight";
import { CopyButton } from "./components/ui/CopyButton";

interface ComponentsOptions {
  /** Content of the trailing unterminated fence while streaming; undefined once settled. */
  streamingTail?: string;
}

/**
 * The content of a trailing unterminated fenced block in `source`, or undefined when every
 * fence is closed. Mirrors CommonMark's fence rules closely enough for gating: up to three
 * leading spaces, a closing fence needs the same character, at least the opening length, and
 * nothing but whitespace after it. Backtick fences also close on any info string containing a
 * backtick, which CommonMark forbids in openers.
 */
export function openFenceTail(source: string): string | undefined {
  const lines = source.split("\n");
  let open = -1;
  let fence = "";
  let fenceChar = "";
  for (const [index, line] of lines.entries()) {
    const match = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line);
    if (!fence) {
      if (match && !(match[1][0] === "`" && /`/.test(match[2]))) {
        fence = match[1];
        fenceChar = match[1][0];
        open = index;
      }
    } else if (match && match[1][0] === fenceChar && match[1].length >= fence.length && !/\S/.test(match[2])) {
      fence = "";
    }
  }
  if (!fence) return undefined;
  return lines.slice(open + 1).join("\n");
}

/** The plain text a hast code element renders to. */
function codeText(node: Element): string {
  let text = "";
  const walk = (children: ElementContent[]): void => {
    for (const child of children) {
      if (child.type === "text") text += child.value;
      else if (child.type === "element") walk(child.children);
    }
  };
  walk(node.children);
  return text;
}

/** Streaming reveals the fence content a line at a time; allow the newline bookkeeping to differ. */
function matchesOpenTail(text: string, tail: string): boolean {
  return text === tail || text === tail + "\n" || text + "\n" === tail;
}

interface CodePreProps {
  node?: Element;
  children?: ReactNode;
  streamingTail?: string;
}

function CodePre({ node, children, streamingTail }: CodePreProps) {
  const code = node?.children.find((child): child is Element => child.type === "element" && child.tagName === "code");
  // A pre without a code child (unknown shape) keeps the default rendering.
  if (!code) return <pre>{children}</pre>;
  const className = classNameOfCode(code);
  const text = codeText(code);
  const tag = /language-([\w+#.-]+)/.exec(className ?? "")?.[1];
  const plain = streamingTail !== undefined && matchesOpenTail(text, streamingTail);
  return (
    <div className="code-block">
      <div className="code-block-toolbar">
        <span className="code-block-language">{tag ?? "Code"}</span>
        <CopyButton text={text} label="Copy code" />
      </div>
      <pre>
        <code className={plain ? className : className ? `hljs ${className}` : "hljs"}>
          {plain ? text : highlightBlock(text, tag)}
        </code>
      </pre>
    </div>
  );
}

function classNameOfCode(node: Element): string | undefined {
  const value = node.properties?.className;
  const joined = (Array.isArray(value) ? value : [])
    .filter((entry): entry is string => typeof entry === "string").join(" ");
  return joined || undefined;
}

export function markdownComponents(options: ComponentsOptions = {}): Components {
  const streamingTail = options.streamingTail;
  return {
    pre: ({ node, children }) => <CodePre node={node} streamingTail={streamingTail}>{children}</CodePre>,
    img: ({ alt }) => <span className="blocked-image">[Remote image blocked{alt ? `: ${alt}` : ""}]</span>,
    a: ({ href, children: linkChildren }) => (
      <a
        href={href}
        target="_blank"
        rel="noreferrer"
        onClick={(event) => {
          // The webview has nothing to open a new window with, and a link left to navigate would
          // replace the app itself, so every click is intercepted. http(s) URLs go to Rust,
          // which opens them in the default browser and refuses every other scheme.
          event.preventDefault();
          if (href !== undefined && /^https?:\/\//i.test(href)) {
            void api.revealPath(href).catch(() => undefined);
          }
        }}
      >
        {linkChildren}
      </a>
    )
  };
}
