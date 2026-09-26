import { memo, useMemo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { markdownComponents, openFenceTail } from "../markdown-components";

/**
 * Assistant prose. Code fences are syntax-highlighted (src/highlight.ts). While `streaming`, the
 * one fence still being written renders plain; closed fences highlight as usual.
 */
export const Markdown = memo(function Markdown({ children, streaming }: { children: string; streaming?: boolean }) {
  const components = useMemo(
    () => markdownComponents({ streamingTail: streaming ? openFenceTail(children) : undefined }),
    [children, streaming]
  );
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml components={components}>{children}</ReactMarkdown>
  );
});
