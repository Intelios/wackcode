import { memo, useMemo } from "react";
import ReactMarkdown, { defaultUrlTransform, type UrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import { markdownComponents } from "../markdown-components";
import { streamSegments } from "../markdown-segments";
import { parseReplyImageReference } from "../reply-images";

const replyImageUrlTransform: UrlTransform = (value, key, node) => {
  if (key === "src" && node.tagName === "img") {
    if (parseReplyImageReference(value)) return value;
    if (value.startsWith("wackcode-image:")) return "wackcode-image://unavailable";
  }
  return defaultUrlTransform(value);
};

interface MarkdownProps {
  children: string;
  /** The source is still being written: segment it and leave the open tail streaming. */
  streaming?: boolean;
  freshInk?: boolean;
  replyImages?: boolean;
}

/** One markdown source, one react-markdown processor: memoization is what makes segments free. */
const MarkdownBlock = memo(function MarkdownBlock({ children, streamingTail, freshInk, replyImages = false }: Omit<MarkdownProps, "streaming"> & { streamingTail?: string }) {
  const components = useMemo(
    () => markdownComponents({
      streamingTail,
      replyImages,
      // trimEnd: a final block still owns the write head while the source ends in newlines.
      freshInk: freshInk ? { sourceLength: children.trimEnd().length } : undefined
    }),
    [children, streamingTail, freshInk, replyImages]
  );
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml components={components} urlTransform={replyImages ? replyImageUrlTransform : undefined}>{children}</ReactMarkdown>
  );
});

/**
 * Assistant prose. Code fences are syntax-highlighted (src/highlight.ts). While `streaming`, the
 * source is split into segments at stable block boundaries (src/markdown-segments.ts) so a
 * growing answer re-parses only its open tail each smooth-text frame — react-markdown builds a
 * fresh processor and parses everything it's given on every render, and memoized settled
 * segments simply don't render again. Only the tail streams: the one fence still being written
 * renders plain; closed fences highlight as usual. While `freshInk`, the source's final block
 * renders as `.ink-fresh` word spans so the write head can cool to settled ink (used by
 * ThinkingRow's live body; reduced motion skips it there). Settled prose keeps one full-document
 * parse, which stays authoritative: a loose list or a forward reference that streamed as
 * segments always resolves to the document's own reading.
 */
export const Markdown = memo(function Markdown({ children, streaming, freshInk, replyImages = false }: MarkdownProps) {
  const split = useMemo(() => (streaming ? streamSegments(children) : undefined), [children, streaming]);
  if (!split) return <MarkdownBlock freshInk={freshInk} replyImages={replyImages}>{children}</MarkdownBlock>;
  const { segments, fenceTail } = split;
  return (
    <>
      {segments.map((segment, index) => {
        const tail = index === segments.length - 1;
        return (
          <MarkdownBlock key={index} streamingTail={tail ? fenceTail : undefined} freshInk={tail && freshInk} replyImages={replyImages}>
            {segment}
          </MarkdownBlock>
        );
      })}
    </>
  );
});
