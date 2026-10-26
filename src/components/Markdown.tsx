import { memo, useMemo } from "react";
import ReactMarkdown, { defaultUrlTransform, type UrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import { markdownComponents, openFenceTail } from "../markdown-components";
import { parseReplyImageReference } from "../reply-images";

const replyImageUrlTransform: UrlTransform = (value, key, node) => {
  if (key === "src" && node.tagName === "img") {
    if (parseReplyImageReference(value)) return value;
    if (value.startsWith("wackcode-image:")) return "wackcode-image://unavailable";
  }
  return defaultUrlTransform(value);
};

/**
 * Assistant prose. Code fences are syntax-highlighted (src/highlight.ts). While `streaming`, the
 * one fence still being written renders plain; closed fences highlight as usual. While
 * `freshInk`, the source's final block renders as `.ink-fresh` word spans so the write head
 * can cool to settled ink (used by ThinkingRow's live body; reduced motion skips it there).
 */
export const Markdown = memo(function Markdown({ children, streaming, freshInk, replyImages = false }: { children: string; streaming?: boolean; freshInk?: boolean; replyImages?: boolean }) {
  const components = useMemo(
    () => markdownComponents({
      streamingTail: streaming ? openFenceTail(children) : undefined,
      replyImages,
      // trimEnd: a final block still owns the write head while the source ends in newlines.
      freshInk: freshInk ? { sourceLength: children.trimEnd().length } : undefined
    }),
    [children, streaming, freshInk, replyImages]
  );
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml components={components} urlTransform={replyImages ? replyImageUrlTransform : undefined}>{children}</ReactMarkdown>
  );
});
