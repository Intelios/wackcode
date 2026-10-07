import { memo, useMemo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { markdownComponents, openFenceTail } from "../markdown-components";

/**
 * Assistant prose. Code fences are syntax-highlighted (src/highlight.ts). While `streaming`, the
 * one fence still being written renders plain; closed fences highlight as usual. While
 * `freshInk`, the source's final block renders as `.ink-fresh` word spans so the write head
 * can cool to settled ink (used by ThinkingRow's live body; reduced motion skips it there).
 */
export const Markdown = memo(function Markdown({ children, streaming, freshInk }: { children: string; streaming?: boolean; freshInk?: boolean }) {
  const components = useMemo(
    () => markdownComponents({
      streamingTail: streaming ? openFenceTail(children) : undefined,
      // trimEnd: a final block still owns the write head while the source ends in newlines.
      freshInk: freshInk ? { sourceLength: children.trimEnd().length } : undefined
    }),
    [children, streaming, freshInk]
  );
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml components={components}>{children}</ReactMarkdown>
  );
});
