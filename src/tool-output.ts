/**
 * What a tool call's arguments and result text say, taken apart for the expanded sheets
 * (`components/ToolDetail.tsx`). Pi's built-in tools return plain text in known shapes; every
 * parser here gives up (undefined, or the text untouched) on anything else, and the sheet then
 * shows the text whole. Presentation only: nothing here decides what ran or what the model saw.
 */

/** A value as text, or "" for anything else. */
function str(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}

/**
 * The bracketed note Pi appends after a blank line when it cut a result short:
 * "[Showing lines 1-80 of 412. Use offset=81 to continue.]", "[100 matches limit reached. …]".
 * Matched by its wording, so a file that merely ends in a bracketed line stays whole.
 */
const NOTICE = /\n\n\[((?:Showing (?:lines|last) |\d+ more lines in file|[^\]\n]*(?:limit reached|truncated to))[^\]\n]*)\]$/;

/** A result's text and Pi's trailing note, without the sentences telling the model how to go on. */
export function splitNotice(text: string): { body: string; notice?: string } {
  const match = NOTICE.exec(text);
  if (!match) return { body: text };
  const notice = match[1].split(". ").filter((part) => !part.startsWith("Use ")).join(". ").replace(/\.$/, "");
  return { body: text.slice(0, match.index), notice };
}

const COMMAND_STATUS = /(?:^|\n\n)Command (exited with code (\d+)|aborted|timed out after (\d+) seconds|terminated without an exit code)$/;

/**
 * A failed command's output and how it ended ("exit 1", "timed out after 30s"). Pi appends the
 * ending to the output it throws; a failure in any other shape keeps its text whole.
 */
export function splitCommandStatus(text: string): { output: string; status?: string } {
  const match = COMMAND_STATUS.exec(text);
  if (!match) return { output: text };
  const status = match[2] !== undefined ? `exit ${match[2]}`
    : match[3] !== undefined ? `timed out after ${match[3]}s`
    : match[1] === "aborted" ? "stopped" : "no exit code";
  return { output: text.slice(0, match.index), status };
}

/** Lines of text as a file has them: a trailing newline ends the last line rather than starting one. */
function countLines(text: string): number {
  if (!text) return 0;
  const lines = text.split("\n").length;
  return text.endsWith("\n") ? lines - 1 : lines;
}

const plural = (count: number, noun: string, many = `${noun}s`) => `${count} ${count === 1 ? noun : many}`;

/**
 * Which lines of the file a `read` returned: the number its first line carries, and the range
 * in words ("lines 40–119 of 600", or "84 lines" for a whole file). Pi's note supplies the
 * file's length when it cut the read short.
 */
export function readRange(args: Record<string, unknown>, body: string, notice?: string): { first: number; label: string } {
  const first = typeof args.offset === "number" && args.offset > 0 ? Math.floor(args.offset) : 1;
  const count = countLines(body);
  const last = first + Math.max(count, 1) - 1;
  const shown = /^Showing lines (\d+)-(\d+) of (\d+)/.exec(notice ?? "");
  if (shown) return { first: Number(shown[1]), label: `lines ${shown[1]}–${shown[2]} of ${shown[3]}` };
  const more = /^(\d+) more lines in file/.exec(notice ?? "");
  if (more) return { first, label: `lines ${first}–${last} of ${last + Number(more[1])}` };
  if (first > 1) return { first, label: `lines ${first}–${last}` };
  return { first, label: plural(count, "line") };
}

export interface GrepRow {
  line: number;
  text: string;
  /** A matching line; false for the context lines around one. */
  hit: boolean;
}

export interface GrepFile {
  path: string;
  rows: GrepRow[];
}

/**
 * Pi's `grep` output ("path:12: text" for a match, "path-12- text" for context) grouped by
 * file, in order. Undefined unless every non-empty line reads that way. The marker is the first
 * ":12:" or "-12-" that a space (or the line's end) follows, which is what lets a path carry
 * digits between dashes of its own, as a dated file name does.
 */
export function parseGrep(lines: string[]): GrepFile[] | undefined {
  const files: GrepFile[] = [];
  for (const line of lines) {
    if (!line) continue;
    const match = /^(.+?)([:-])(\d+)\2(?: (.*))?$/.exec(line);
    if (!match) return undefined;
    const row: GrepRow = { line: Number(match[3]), text: match[4] ?? "", hit: match[2] === ":" };
    const current = files[files.length - 1];
    if (current?.path === match[1]) current.rows.push(row);
    else files.push({ path: match[1], rows: [row] });
  }
  return files.length ? files : undefined;
}

/**
 * The pattern a `grep` searched for, as a matcher for lighting up its hits. Ripgrep's syntax is
 * not JavaScript's, so a pattern that doesn't compile here simply isn't lit.
 */
export function grepMatcher(args: Record<string, unknown>): RegExp | undefined {
  const pattern = str(args.pattern);
  if (!pattern) return undefined;
  try {
    const source = args.literal === true ? pattern.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") : pattern;
    return new RegExp(source, args.ignoreCase === true ? "gi" : "g");
  } catch {
    return undefined;
  }
}

/** A line cut into the stretches a matcher hit and the text between them. */
export function splitMatches(text: string, matcher: RegExp | undefined): { text: string; hit: boolean }[] {
  if (!matcher || !text) return [{ text, hit: false }];
  const parts: { text: string; hit: boolean }[] = [];
  let from = 0;
  matcher.lastIndex = 0;
  for (let match = matcher.exec(text); match; match = matcher.exec(text)) {
    // An empty match (`a*`) lights nothing and would never advance on its own.
    if (!match[0]) { matcher.lastIndex += 1; continue; }
    if (match.index > from) parts.push({ text: text.slice(from, match.index), hit: false });
    parts.push({ text: match[0], hit: true });
    from = match.index + match[0].length;
  }
  if (from < text.length) parts.push({ text: text.slice(from), hit: false });
  return parts.length ? parts : [{ text, hit: false }];
}

export interface CommitSummary {
  /** The abbreviated hash exactly as git printed it. */
  hash: string;
  /** The commit's subject line; empty for a message with no subject. */
  subject: string;
  /** How much the commit changed, when git printed its stats line. */
  files?: number;
  additions?: number;
  deletions?: number;
}

/** The summary line git prints for a commit it made: "[main a1b2c3d] Fix the upload". */
const COMMIT_LINE = /^\[([^\]]+) ([0-9a-f]{7,40})\] (.*)$/;
/** The stats line that follows it (past the "Date:" line an amend prints): " 2 files changed, 12 insertions(+), 3 deletions(-)". */
const COMMIT_STATS = /^ (\d+) files? changed(?:, (\d+) insertions?\(\+\))?(?:, (\d+) deletions?\(-\))?$/;

/**
 * What `git commit` printed about the commit it made. The ref may carry spaces ("master
 * (root-commit)", "detached HEAD") and the subject may carry brackets of its own, so the
 * hash is the hex word the first closing bracket follows. The LAST summary line wins: a
 * command that commits twice names the newer commit, and its stats are the lines right
 * after it. Undefined when the output shows no summary line — a refused commit, a dry run,
 * a quiet run — and whatever the command printed stays whole.
 */
export function parseCommit(text: string): CommitSummary | undefined {
  const lines = text.split("\n");
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    const match = COMMIT_LINE.exec(lines[index]);
    if (!match) continue;
    const summary: CommitSummary = { hash: match[2], subject: match[3] };
    for (const follow of lines.slice(index + 1, index + 3)) {
      const stats = COMMIT_STATS.exec(follow);
      if (stats) {
        summary.files = Number(stats[1]);
        if (stats[2]) summary.additions = Number(stats[2]);
        if (stats[3]) summary.deletions = Number(stats[3]);
        break;
      }
    }
    return summary;
  }
  return undefined;
}

export interface ArgRow {
  key: string;
  value: string;
  /** How the value reads: prose or a path, a number or switch, or nested data shown as JSON. */
  kind: "text" | "scalar" | "data";
}

/** A call's arguments as labelled rows, in the order the model wrote them. */
export function argRows(args: Record<string, unknown>): ArgRow[] {
  return Object.entries(args).flatMap(([key, value]): ArgRow[] => {
    if (value === undefined) return [];
    if (typeof value === "string") return [{ key, value, kind: "text" }];
    if (value === null || typeof value === "number" || typeof value === "boolean") return [{ key, value: String(value), kind: "scalar" }];
    return [{ key, value: JSON.stringify(value, null, 2), kind: "data" }];
  });
}

export interface AskedOption {
  label: string;
  description: string;
  chosen: boolean;
}

export interface AskedQuestion {
  header: string;
  question: string;
  /** Several options could be picked, which the live card marks with boxes rather than dots. */
  multi: boolean;
  options: AskedOption[];
  /** What the user wrote instead of, or as well as, picking an option. */
  custom?: string;
}

export interface Asked {
  questions: AskedQuestion[];
  /** Where the questions stand: no result yet, answered, put aside, or cut short for the plan. */
  outcome: "waiting" | "answered" | "dismissed" | "wrapped-up";
}

/**
 * An `ask_user_question` call as the questions it asked with the answers given, from the call's
 * arguments and its result's details. Undefined when the arguments aren't the questions the
 * tool takes (still streaming, or refused), which leaves the call to the plain argument sheet.
 */
export function askedQuestions(args: Record<string, unknown>, result?: { details?: unknown; isError?: boolean }): Asked | undefined {
  if (!Array.isArray(args.questions) || !args.questions.length || result?.isError) return undefined;
  const details = record(result?.details);
  const answers = (Array.isArray(details?.answers) ? details.answers : []).map(record).filter((entry) => entry !== undefined);
  const questions: AskedQuestion[] = [];
  for (const entry of args.questions) {
    const raw = record(entry);
    const question = str(raw?.question);
    if (!raw || !question || !Array.isArray(raw.options)) return undefined;
    const answer = answers.find((candidate) => candidate.questionId === raw.id);
    const selected = Array.isArray(answer?.selected) ? answer.selected : [];
    questions.push({
      header: str(raw.header),
      question,
      multi: raw.multiSelect === true,
      options: raw.options.flatMap((option) => {
        const label = str(record(option)?.label);
        return label ? [{ label, description: str(record(option)?.description), chosen: selected.includes(label) }] : [];
      }),
      ...(str(answer?.custom) ? { custom: str(answer?.custom) } : {})
    });
  }
  if (!result) return { questions, outcome: "waiting" };
  if (details?.cancelled === true) return { questions, outcome: details.reason === "wrap_up" ? "wrapped-up" : "dismissed" };
  // A result without its answers (not this app's details) is better read as the text it came with.
  return answers.length ? { questions, outcome: "answered" } : undefined;
}
