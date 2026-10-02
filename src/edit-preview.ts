/** Chat-only edit previews. Keep saved tool results and the Changes panel untouched. */
const CONTEXT = 3;

/** Hide long unchanged runs, retaining every change and its surrounding context. */
export function focusedEditDiff(diff: string): string {
  const lines = diff.split("\n");
  const changed = (line: string) => /^[+-]/.test(line) && !/^(\+\+\+|---)/.test(line);
  if (!lines.some(changed)) return diff;
  const shown: string[] = [];
  let index = 0;
  while (index < lines.length) {
    // Both unified patches and Pi's numbered display diffs prefix context with a space.
    if (!lines[index].startsWith(" ") || /^\s+\.\.\.$/.test(lines[index])) {
      shown.push(lines[index++]);
      continue;
    }
    const start = index;
    while (index < lines.length && lines[index].startsWith(" ") && !/^\s+\.\.\.$/.test(lines[index])) index++;
    const before = start > 0 && changed(lines[start - 1]);
    const after = index < lines.length && changed(lines[index]);
    const leading = before ? Math.min(CONTEXT, index - start) : 0;
    const trailing = after ? Math.min(CONTEXT, index - start - leading) : 0;
    shown.push(...lines.slice(start, start + leading));
    if (index - start > leading + trailing) shown.push(" …");
    shown.push(...lines.slice(index - trailing, index));
  }
  return shown.join("\n");
}

/** Myers line diff: sparse edits stay cheap even when their context spans a large file. */
function changedLines(oldLines: string[], newLines: string[]): string[] {
  const maxDistance = Math.min(oldLines.length + newLines.length, 512);
  const offset = maxDistance + 1;
  const frontier = new Int32Array(2 * maxDistance + 3);
  const trace: Int32Array[] = [];
  for (let distance = 0; distance <= maxDistance; distance++) {
    trace.push(frontier.slice());
    for (let diagonal = -distance; diagonal <= distance; diagonal += 2) {
      const at = offset + diagonal;
      let x = diagonal === -distance || (diagonal !== distance && frontier[at - 1] < frontier[at + 1])
        ? frontier[at + 1] : frontier[at - 1] + 1;
      let y = x - diagonal;
      while (x < oldLines.length && y < newLines.length && oldLines[x] === newLines[y]) { x++; y++; }
      frontier[at] = x;
      if (x < oldLines.length || y < newLines.length) continue;
      const lines: string[] = [];
      for (let step = distance; step > 0; step--) {
        const previous = trace[step];
        const k = x - y;
        const previousK = k === -step || (k !== step && previous[offset + k - 1] < previous[offset + k + 1]) ? k + 1 : k - 1;
        const previousX = previous[offset + previousK];
        const previousY = previousX - previousK;
        while (x > previousX && y > previousY) { lines.push(` ${oldLines[--x]}`); y--; }
        if (x === previousX) lines.push(`+${newLines[--y]}`);
        else lines.push(`-${oldLines[--x]}`);
      }
      while (x > 0 && y > 0) { lines.push(` ${oldLines[--x]}`); y--; }
      return lines.reverse();
    }
  }
  // Bound diff work for large replacements. A coarse replacement is still truthful:
  // preserve every source line instead of freezing the chat or dropping any changes.
  return oldLines.map((line) => `-${line}`).concat(newLines.map((line) => `+${line}`));
}

function replacementDiff(oldText: string, newText: string): string {
  const oldLines = oldText.replace(/\r\n/g, "\n").split("\n");
  const newLines = newText.replace(/\r\n/g, "\n").split("\n");
  // A trailing newline is a terminator, not an extra source line.
  if (oldLines.at(-1) === "") oldLines.pop();
  if (newLines.at(-1) === "") newLines.pop();
  let start = 0;
  while (start < oldLines.length && start < newLines.length && oldLines[start] === newLines[start]) start++;
  let end = 0;
  while (end < oldLines.length - start && end < newLines.length - start && oldLines[oldLines.length - 1 - end] === newLines[newLines.length - 1 - end]) end++;
  const oldMiddle = oldLines.slice(start, oldLines.length - end);
  const newMiddle = newLines.slice(start, newLines.length - end);
  const lines = oldLines.slice(Math.max(0, start - CONTEXT), start).map((line) => ` ${line}`);
  for (const line of changedLines(oldMiddle, newMiddle)) lines.push(line);
  lines.push(...oldLines.slice(oldLines.length - end, oldLines.length - end + CONTEXT).map((line) => ` ${line}`));
  if (oldText.endsWith("\n") !== newText.endsWith("\n")) lines.push("\\ Final newline changed");
  return focusedEditDiff(lines.join("\n"));
}

/** Saved/extension edits without a result diff still get changes, rather than raw JSON. */
export function editArgumentDiff(args: Record<string, unknown>): string | undefined {
  const edits = Array.isArray(args.edits) ? args.edits : [args];
  if (!edits.length) return undefined;
  const diffs: string[] = [];
  for (const [index, edit] of edits.entries()) {
    if (!edit || typeof edit !== "object") return undefined;
    const { oldText, newText } = edit as Record<string, unknown>;
    if (typeof oldText !== "string" || typeof newText !== "string") return undefined;
    diffs.push(`@@ Edit ${index + 1} @@\n${replacementDiff(oldText, newText)}`);
  }
  return diffs.join("\n");
}
