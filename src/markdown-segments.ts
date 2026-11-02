/**
 * Splits a still-growing markdown source into segments at boundaries beyond which later text
 * can never change how what came before parses. `Markdown` renders each segment through its own
 * memoized react-markdown instance, so streamed output re-parses only the open tail each
 * smooth-text frame instead of the whole answer — the cost that used to grow with the answer
 * now grows with the block being written.
 *
 * A boundary is stable only where a block is complete for good:
 * - after a blank line outside a fence. Blank lines end every construct that can span lines
 *   (a paragraph can no longer become a setext heading or take a lazy continuation; lists,
 *   block quotes, tables and HTML blocks are all closed);
 * - after a closing fence line. A closed fenced block is a leaf nothing merges into, which
 *   lets "```\nnext para" split even without a blank line.
 * Everything else stays in the open tail: an indented-code line or a lazy continuation belongs
 * to the block it trails until a blank line arrives.
 *
 * Fence detection mirrors `openFenceTail` (markdown-components.tsx): up to three leading
 * spaces (four means indented code, not a fence), a closer needs the same character at least
 * the opener's length with whitespace-only info, and a backtick info string containing a
 * backtick never opens a fence.
 *
 * Contract: `segments.join("")` is exactly `source`; boundaries only move forward as the
 * stream grows; the last segment is the open tail and may be the whole source.
 */

export function streamSegments(source: string): string[] {
  const segments: string[] = [];
  let start = 0;
  // Where the stable boundary sits while blank lines stream in: the first blank of a run.
  let pendingCut = -1;
  let fence = "";
  let fenceChar = "";
  let offset = 0;
  // `end` steps past each line's newline, so `offset` is always the line's start; a final
  // line without a newline makes `end` overshoot by one, which `slice` clamps.
  const lines = source.split("\n");
  for (const line of lines) {
    const end = offset + line.length + 1;
    const match = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line);
    if (fence) {
      if (match && match[1][0] === fenceChar && match[1].length >= fence.length && !/\S/.test(match[2])) {
        fence = "";
        segments.push(source.slice(start, end));
        start = end;
        pendingCut = -1;
      }
    } else if (match && !(match[1][0] === "`" && /`/.test(match[2]))) {
      // A blank line exactly at the boundary leaves nothing to settle.
      if (pendingCut > start) segments.push(source.slice(start, pendingCut));
      if (pendingCut >= 0) start = pendingCut;
      pendingCut = -1;
      fence = match[1];
      fenceChar = match[1][0];
    } else if (/^\s*$/.test(line)) {
      // The boundary sits at the blank line's start: the settled segment keeps its own
      // terminating newline and the rest of the whitespace run leads the tail.
      if (pendingCut < 0) pendingCut = offset;
    } else if (pendingCut >= 0) {
      if (pendingCut > start) segments.push(source.slice(start, pendingCut));
      start = pendingCut;
      pendingCut = -1;
    }
    offset = end;
  }
  const tail = source.slice(start);
  if (/\S/.test(tail)) segments.push(tail);
  return segments;
}
