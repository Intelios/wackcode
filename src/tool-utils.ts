import type { NormalizedBlock, NormalizedMessage, SubagentDetails, SubagentResult, SubagentStatus, ToolCatalogEntry } from "./types";
import { displayPath } from "./chat-utils";

/** The built-in sub-agents tool. Its calls render as SubAgent chips rather than a tool row. */
export const SUBAGENT_TOOL_NAME = "subagent";
/** The one WackCode built-in tool the user can switch off, through the tool denylist. */
export const WEB_FETCH_TOOL_NAME = "web_fetch";
/** Browser preview is one grouped switch even though the extension exposes five tools. */
export const BROWSER_TOOL_NAMES = ["browser_open", "browser_snapshot", "browser_act", "browser_screenshot", "browser_console"] as const;
/** Computer use: its own setting (Settings › Packages), never the tool denylist. */
export const COMPUTER_TOOL_NAMES = ["computer_apps", "computer_open", "computer_snapshot", "computer_screenshot", "computer_act"] as const;
/** Memory: its own setting (Settings › Memory), never the tool denylist. */
export const MEMORY_TOOL_NAMES = ["memory_save", "memory_recall", "memory_forget"] as const;

/** An app as the model named it: a `.app` path shows as its name. */
function appName(value: string): string {
  const match = /([^/]+)\.app\/?$/i.exec(value);
  return match ? match[1] : value;
}

/** A one-line description of a `computer_act` batch: its only action, or how many. */
function describeActions(actions: unknown): string {
  if (!Array.isArray(actions) || actions.length === 0) return "";
  if (actions.length > 1) return `${actions.length} actions`;
  const action = actions[0] as Record<string, unknown>;
  const text = (value: unknown, max = 40) => {
    const raw = typeof value === "string" ? value.replace(/\s+/g, " ") : "";
    return raw.length > max ? `${raw.slice(0, max)}…` : raw;
  };
  switch (action.kind) {
    case "typeText":
    case "setText":
      return `typed "${text(action.text)}"`;
    case "keypress":
      return `pressed ${text(action.keys)}`;
    case "menu":
      return Array.isArray(action.path) ? `chose ${action.path.map((item) => text(item, 30)).join(" › ")}` : "chose a menu item";
    case "click":
      return action.ref ? `clicked ${text(action.ref)}` : "clicked";
    case "press":
      return `pressed ${text(action.ref)}`;
    default:
      return typeof action.kind === "string" ? action.kind : "";
  }
}

export interface ToolSummary {
  /** Label while the tool is executing, e.g. "Editing". */
  activeVerb: string;
  /** Label once finished, e.g. "Edited". */
  doneVerb: string;
  /** Path or command shown after the verb. */
  subject: string;
  additions?: number;
  deletions?: number;
  /** Kind controls what the expanded body shows. */
  kind: "read" | "edit" | "write" | "bash" | "search" | "other";
}

function args(block: NormalizedBlock): Record<string, unknown> {
  const value = block.arguments;
  if (value && typeof value === "object" && !Array.isArray(value)) return value as Record<string, unknown>;
  if (typeof value === "string") {
    try {
      const parsed = JSON.parse(value) as unknown;
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) return parsed as Record<string, unknown>;
    } catch { /* still streaming */ }
  }
  return {};
}

function str(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function countLines(text: string): number {
  if (!text) return 0;
  return text.split("\n").filter((line) => line.length > 0).length;
}

export function diffStats(diff: unknown): { additions: number; deletions: number } | undefined {
  if (typeof diff !== "string" || !diff) return undefined;
  let additions = 0;
  let deletions = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) additions += 1;
    else if (line.startsWith("-") && !line.startsWith("---")) deletions += 1;
  }
  return additions || deletions ? { additions, deletions } : undefined;
}

function detailsOf(block?: NormalizedBlock): Record<string, unknown> {
  return (block?.details ?? {}) as Record<string, unknown>;
}

export function editStats(call: NormalizedBlock, result?: NormalizedBlock): { additions: number; deletions: number } | undefined {
  const fromDiff = diffStats(detailsOf(result).diff);
  if (fromDiff) return fromDiff;
  const edits = args(call).edits;
  if (!Array.isArray(edits)) return undefined;
  let additions = 0;
  let deletions = 0;
  for (const edit of edits) {
    const entry = edit as Record<string, unknown>;
    deletions += countLines(str(entry.oldText));
    additions += countLines(str(entry.newText));
  }
  return additions || deletions ? { additions, deletions } : undefined;
}

/** A fetched URL as a row subject: host (with any port) and path, without the scheme, "www." or the query. */
export function displayUrl(value: string): string {
  try {
    const url = new URL(value);
    return `${url.host.replace(/^www\./, "")}${url.pathname === "/" ? "" : url.pathname}`;
  } catch {
    return value;
  }
}

/**
 * A path subject (`displayPath` output) as a dim directory tail and the name to anchor on.
 * `dir` keeps its trailing slash; anything without a slash stays a single `name`.
 */
export function splitPathSubject(path: string): { dir: string; name: string } {
  const slash = path.lastIndexOf("/");
  const name = slash >= 0 ? path.slice(slash + 1) : path;
  if (slash < 0 || !name) return { dir: "", name: path };
  return { dir: path.slice(0, slash + 1), name };
}

/** An MCP tool's name, `mcp__<server slug>__<tool>`, split into its parts (see the worker's `mcpToolName`). */
export function mcpToolParts(name: string): { server: string; tool: string } | undefined {
  const match = /^mcp__([a-z0-9_]+?)__(.+)$/.exec(name);
  return match ? { server: match[1], tool: match[2] } : undefined;
}

export function summarizeTool(call: NormalizedBlock, result?: NormalizedBlock): ToolSummary {
  const toolArgs = args(call);
  const name = call.toolName ?? "tool";
  switch (name) {
    case "read":
      return { kind: "read", activeVerb: "Reading", doneVerb: "Read", subject: displayPath(str(toolArgs.path)) };
    case "edit": {
      const stats = editStats(call, result);
      return { kind: "edit", activeVerb: "Editing", doneVerb: "Edited", subject: displayPath(str(toolArgs.path)), additions: stats?.additions, deletions: stats?.deletions };
    }
    case "write": {
      const content = str(toolArgs.content);
      const isNew = detailsOf(result).created === true || detailsOf(result).existed === false;
      return { kind: "write", activeVerb: "Writing", doneVerb: isNew ? "Created" : "Wrote", subject: displayPath(str(toolArgs.path)), additions: content ? countLines(content) : undefined };
    }
    case "bash":
      return { kind: "bash", activeVerb: "Running", doneVerb: "Ran", subject: str(toolArgs.command) };
    case "grep":
      return { kind: "search", activeVerb: "Searching", doneVerb: "Searched", subject: str(toolArgs.pattern) };
    case "find":
      return { kind: "search", activeVerb: "Finding", doneVerb: "Found", subject: str(toolArgs.pattern) };
    case "ls":
      return { kind: "search", activeVerb: "Listing", doneVerb: "Listed", subject: displayPath(str(toolArgs.path) || ".") };
    case "ask_user_question": {
      const questions = toolArgs.questions;
      const first = Array.isArray(questions) ? questions[0] as Record<string, unknown> | undefined : undefined;
      return { kind: "other", activeVerb: "Asking", doneVerb: "Asked", subject: str(first?.question) };
    }
    case "plan_mode_complete":
      return { kind: "other", activeVerb: "Submitting plan", doneVerb: "Plan submitted", subject: "" };
    case "todo":
      return { kind: "other", activeVerb: "Updating todos", doneVerb: "Todos updated", subject: str(toolArgs.subject) };
    case WEB_FETCH_TOOL_NAME:
      return { kind: "other", activeVerb: "Fetching", doneVerb: "Fetched", subject: displayUrl(str(toolArgs.url)) };
    case "memory_save": {
      const updated = typeof toolArgs.name === "string" && toolArgs.name.trim();
      return { kind: "other", activeVerb: updated ? "Updating memory" : "Saving memory", doneVerb: updated ? "Updated memory" : "Saved memory", subject: str(toolArgs.title) };
    }
    case "memory_recall": {
      const names = Array.isArray(toolArgs.names) ? toolArgs.names.map((name) => str(name)) : [];
      return { kind: "other", activeVerb: "Recalling memory", doneVerb: "Recalled memory", subject: names.join(", ") };
    }
    case "memory_forget":
      return { kind: "other", activeVerb: "Forgetting memory", doneVerb: "Forgot memory", subject: str(toolArgs.name) };
    case "computer_apps":
      return { kind: "other", activeVerb: "Listing apps", doneVerb: "Listed apps", subject: "" };
    case "computer_open":
      return { kind: "other", activeVerb: "Opening", doneVerb: "Opened", subject: appName(str(toolArgs.app)) };
    case "computer_snapshot":
      return { kind: "other", activeVerb: "Inspecting", doneVerb: "Inspected", subject: appName(str(toolArgs.app)) };
    case "computer_screenshot":
      return { kind: "other", activeVerb: "Capturing", doneVerb: "Captured", subject: appName(str(toolArgs.app)) };
    case "computer_act": {
      const described = describeActions(toolArgs.actions);
      const app = appName(str(toolArgs.app));
      return { kind: "other", activeVerb: "Using", doneVerb: "Used", subject: described ? `${app} · ${described}` : app };
    }
    case SUBAGENT_TOOL_NAME: {
      const tasks = Array.isArray(toolArgs.tasks) ? toolArgs.tasks.length : 0;
      return {
        kind: "other",
        activeVerb: tasks ? `Running ${tasks} sub-agents` : "Running sub-agent",
        doneVerb: tasks ? `Ran ${tasks} sub-agents` : "Ran sub-agent",
        subject: tasks ? "" : str(toolArgs.agent)
      };
    }
    default: {
      const mcp = mcpToolParts(name);
      if (mcp) return { kind: "other", activeVerb: "Calling", doneVerb: "Called", subject: `${mcp.tool} (${mcp.server})` };
      return { kind: "other", activeVerb: name, doneVerb: name, subject: "" };
    }
  }
}

/**
 * What a read-only tool call looked at, for the transcript's collapsed "Explored" groups
 * (`explore-utils.ts`). Undefined means the call isn't exploration and stays a row of its own.
 */
export type ExploreKind = "file" | "search" | "list" | "command";

export function exploreKind(call: NormalizedBlock): ExploreKind | undefined {
  switch (call.toolName) {
    case "read":
      return "file";
    case "grep":
    case "find":
      return "search";
    case "ls":
      return "list";
    case "bash":
      return bashExploreKind(str(args(call).command));
    default:
      return undefined;
  }
}

/**
 * The exploration kind of a shell command, or undefined unless every part of it is known to
 * only read. This decides what the transcript folds away, never what may run, so it errs
 * towards leaving a command visible: a command that might write stays a row of its own.
 */
export function bashExploreKind(command: string): ExploreKind | undefined {
  const commands = splitShell(command);
  if (!commands?.length) return undefined;
  let kind: ExploreKind | undefined;
  for (const words of commands) {
    const own = simpleCommandKind(words);
    if (own === undefined) return undefined;
    kind ??= own ?? undefined;
  }
  return kind;
}

/**
 * A shell command split into its simple commands (at `|`, `&&`, `||`, `;` and newlines), each
 * a list of words with quoting removed. Undefined for anything not modelled exactly: command
 * substitution, subshells, backgrounding, input redirection and heredocs, and any output
 * redirection except discarding it (`2>/dev/null`, `2>&1`).
 */
function splitShell(command: string): string[][] | undefined {
  const commands: string[][] = [];
  let words: string[] = [];
  let word = "";
  let inWord = false;
  // Just after `|`, `&&` or `||`: another command must follow, possibly on the next line.
  let afterOperator = false;
  const append = (text: string) => { word += text; inWord = true; afterOperator = false; };
  const endWord = () => { if (inWord) words.push(word); word = ""; inWord = false; };
  const endCommand = (): boolean => {
    endWord();
    if (!words.length) return false;
    commands.push(words);
    words = [];
    return true;
  };

  let index = 0;
  while (index < command.length) {
    const char = command[index];
    const next = command[index + 1];
    if (char === "'") {
      const close = command.indexOf("'", index + 1);
      if (close < 0) return undefined;
      append(command.slice(index + 1, close));
      index = close + 1;
    } else if (char === "\"") {
      let text = "";
      let cursor = index + 1;
      while (cursor < command.length && command[cursor] !== "\"") {
        const inner = command[cursor];
        if (inner === "`" || (inner === "$" && command[cursor + 1] === "(")) return undefined;
        if (inner === "\\" && cursor + 1 < command.length) {
          const escaped = command[cursor + 1];
          text += "\"\\$`".includes(escaped) ? escaped : `\\${escaped}`;
          cursor += 2;
        } else {
          text += inner;
          cursor += 1;
        }
      }
      if (cursor >= command.length) return undefined;
      append(text);
      index = cursor + 1;
    } else if (char === "\\") {
      // A backslash-newline continues the line; any other escaped character is literal.
      if (next !== "\n") append(next ?? "");
      index += 2;
    } else if (char === " " || char === "\t") {
      endWord();
      index += 1;
    } else if (char === "#" && !inWord) {
      const end = command.indexOf("\n", index);
      index = end < 0 ? command.length : end;
    } else if (char === "\n") {
      if (!afterOperator) endCommand();
      index += 1;
    } else if (char === ";") {
      if (afterOperator) return undefined;
      endCommand();
      index += 1;
    } else if ((char === "|" && next === "|") || (char === "&" && next === "&")) {
      if (!endCommand()) return undefined;
      afterOperator = true;
      index += 2;
    } else if (char === "|") {
      if (next === "&" || !endCommand()) return undefined;
      afterOperator = true;
      index += 1;
    } else if (char === ">" || (char === "&" && next === ">")) {
      // Only discarding output is allowed. A file descriptor number belongs to the redirection.
      if (char === ">" && inWord && /^\d$/.test(word)) { word = ""; inWord = false; }
      let cursor = index + (char === "&" ? 2 : 1);
      if (command[cursor] === ">") return undefined;
      if (command[cursor] === "&" && /[012]/.test(command[cursor + 1] ?? "")) {
        cursor += 2;
      } else {
        while (command[cursor] === " " || command[cursor] === "\t") cursor += 1;
        if (!command.startsWith("/dev/null", cursor)) return undefined;
        cursor += "/dev/null".length;
      }
      if (cursor < command.length && !/[\s;|&]/.test(command[cursor])) return undefined;
      endWord();
      index = cursor;
    } else if (char === "<" || char === "&" || char === "(" || char === ")" || char === "`" || (char === "$" && next === "(")) {
      return undefined;
    } else {
      append(char);
      index += 1;
    }
  }
  if (afterOperator) return undefined;
  endCommand();
  return commands;
}

const GIT_READ_COMMANDS: Record<string, ExploreKind> = {
  status: "command",
  log: "command",
  show: "command",
  diff: "command",
  blame: "command",
  "ls-files": "list",
  grep: "search"
};

/**
 * One simple command's kind: null for a read-only command that isn't itself exploration
 * (`cd`, `echo`, a filter like `sort`), undefined when it might write or run something else.
 */
function simpleCommandKind(words: string[]): ExploreKind | null | undefined {
  const [name, ...rest] = words;
  const has = (...options: string[]) => rest.some((word) => options.some((option) => word === option || word.startsWith(`${option}=`)));
  // A cluster of single-letter options, such as `-rn`, that includes `letter`.
  const cluster = (letter: string) => rest.some((word) => /^-[A-Za-z]+$/.test(word) && word.includes(letter));
  switch (name) {
    case "cd":
    case "echo":
    case "true":
    case "cut":
    case "tr":
      return null;
    case "sort":
      return has("--output") || cluster("o") ? undefined : null;
    case "uniq":
      // `uniq input output` writes its second operand.
      return rest.filter((word) => !word.startsWith("-")).length <= 1 ? null : undefined;
    case "pwd":
    case "ls":
    case "tree":
      return "list";
    case "cat":
    case "head":
    case "tail":
    case "nl":
    case "wc":
      return "file";
    case "grep":
    case "egrep":
    case "fgrep":
      return "search";
    case "rg":
      return has("--pre") ? undefined : "search";
    case "find":
      return has("-exec", "-execdir", "-ok", "-okdir", "-delete", "-fprint", "-fprint0", "-fprintf", "-fls") ? undefined : "search";
    case "fd":
      return has("-x", "--exec", "-X", "--exec-batch") || cluster("x") || cluster("X") ? undefined : "search";
    case "sed":
      return sedPrintsOnly(rest) ? "file" : undefined;
    case "git": {
      const kind = GIT_READ_COMMANDS[rest[0] ?? ""];
      return kind && !has("--output", "-O", "--open-files-in-pager") ? kind : undefined;
    }
    default:
      return undefined;
  }
}

/** `sed -n` whose scripts only print line ranges, such as `sed -n '10,40p' file`. */
function sedPrintsOnly(words: string[]): boolean {
  const scripts: string[] = [];
  let quiet = false;
  let explicit = false;
  for (let index = 0; index < words.length; index += 1) {
    const word = words[index];
    if (word === "-e" || word === "--expression") {
      explicit = true;
      scripts.push(words[index + 1] ?? "");
      index += 1;
    } else if (word.startsWith("--expression=")) {
      explicit = true;
      scripts.push(word.slice("--expression=".length));
    } else if (word === "--quiet" || word === "--silent") {
      quiet = true;
    } else if (/^-[A-Za-z]+$/.test(word)) {
      // In-place editing and script files are out; `-n` may share a cluster with `-E` or `-s`.
      if (/[ifw]/.test(word)) return false;
      if (word.includes("n")) quiet = true;
    } else if (word.startsWith("-")) {
      return false;
    } else if (!explicit && scripts.length === 0) {
      scripts.push(word);
    }
  }
  return quiet && scripts.length > 0 && scripts.every((script) => /^[\d\s,$;p]+$/.test(script) && script.includes("p"));
}

export interface ToolGroup {
  /** Stable key: "builtin" for Pi's own tools, otherwise the package source string. */
  id: string;
  label: string;
  tools: ToolCatalogEntry[];
}

/**
 * Group the catalogue for the Tools panel: Pi's own tools first, then one group per package
 * in stable alphabetical order. A package tool with no source string falls into "Other" rather
 * than disappearing. WackCode's built-in extension tools are never listed — they are part of
 * the app itself. The one that can be switched off, web_fetch, has its switch on its Built-ins card.
 * MCP tools aren't listed either: their switches are in Settings › MCP servers.
 */
export function groupTools(catalog: ToolCatalogEntry[]): ToolGroup[] {
  const builtin: ToolCatalogEntry[] = [];
  const byPackage = new Map<string, ToolCatalogEntry[]>();
  for (const tool of [...catalog].sort((left, right) => left.name.localeCompare(right.name))) {
    if (tool.source.kind === "builtin") {
      builtin.push(tool);
      continue;
    }
    if (tool.source.kind === "wackcode" || tool.source.kind === "mcp") {
      continue;
    }
    const id = tool.source.packageId ?? "other";
    const existing = byPackage.get(id);
    if (existing) existing.push(tool);
    else byPackage.set(id, [tool]);
  }
  const groups: ToolGroup[] = [];
  if (builtin.length) groups.push({ id: "builtin", label: "Built-in", tools: builtin });
  for (const id of [...byPackage.keys()].sort()) {
    groups.push({ id, label: id === "other" ? "Other" : id, tools: byPackage.get(id) ?? [] });
  }
  return groups;
}

/**
 * A tool description's opening sentence, for Settings › Tools. Descriptions are written for the
 * model and go on to limits and usage rules; the first sentence is usually what the tool does.
 */
export function firstSentence(text: string): string {
  const trimmed = text.trim().replace(/\s+/g, " ");
  const match = /^.+?[.!?](?=\s|$)/.exec(trimmed);
  return match ? match[0] : trimmed;
}

/**
 * Drop names that are no longer in the catalogue. A denylist referencing a removed package's
 * tool is harmless but accumulates, and showing a stale count in Settings is confusing.
 */
export function pruneDisabledTools(disabled: string[], catalog: ToolCatalogEntry[]): string[] {
  if (!catalog.length) return [...disabled];
  const known = new Set(catalog.map((tool) => tool.name));
  return disabled.filter((name) => known.has(name));
}

/**
 * Snapshots arrive constantly and almost always carry an identical catalogue. Compare before
 * storing so Settings does not re-render on every streamed message.
 */
export function sameToolCatalog(left: ToolCatalogEntry[], right: ToolCatalogEntry[]): boolean {
  if (left.length !== right.length) return false;
  return left.every((tool, index) => {
    const other = right[index];
    return tool.name === other.name
      && tool.available === other.available
      && tool.description === other.description
      && tool.source.kind === other.source.kind
      && tool.source.packageId === other.source.packageId;
  });
}


const SUBAGENT_STATUSES: SubagentStatus[] = ["queued", "running", "done", "failed", "aborted"];

function num(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function subagentResult(value: unknown): SubagentResult | undefined {
  if (!value || typeof value !== "object") return undefined;
  const entry = value as Record<string, unknown>;
  if (typeof entry.agent !== "string" || typeof entry.task !== "string") return undefined;
  const status = SUBAGENT_STATUSES.includes(entry.status as SubagentStatus) ? entry.status as SubagentStatus : undefined;
  if (!status) return undefined;
  const usage = (entry.usage && typeof entry.usage === "object" ? entry.usage : {}) as Record<string, unknown>;
  const activity = Array.isArray(entry.activity)
    ? entry.activity.flatMap((item) => {
        const call = item as Record<string, unknown> | null;
        return call && typeof call.tool === "string" ? [{ tool: call.tool, subject: str(call.subject) }] : [];
      })
    : [];
  return {
    agent: entry.agent,
    task: entry.task,
    readOnly: entry.readOnly === true,
    model: typeof entry.model === "string" ? entry.model : undefined,
    status,
    activity,
    output: typeof entry.output === "string" ? entry.output : undefined,
    outputTruncated: entry.outputTruncated === true,
    error: typeof entry.error === "string" ? entry.error : undefined,
    usage: {
      input: num(usage.input),
      output: num(usage.output),
      cacheRead: num(usage.cacheRead),
      cacheWrite: num(usage.cacheWrite),
      cost: num(usage.cost),
      turns: num(usage.turns)
    },
    startedAt: typeof entry.startedAt === "number" ? entry.startedAt : undefined,
    endedAt: typeof entry.endedAt === "number" ? entry.endedAt : undefined
  };
}

/**
 * The card's data from a `subagent` result or live update. Details are stored in session
 * files, so anything that isn't a version this app understands is treated as absent and the
 * call falls back to a plain tool row.
 */
export function parseSubagentDetails(value: unknown): SubagentDetails | undefined {
  if (!value || typeof value !== "object") return undefined;
  const details = value as Record<string, unknown>;
  if (details.v !== 1 || !Array.isArray(details.results) || details.results.length === 0) return undefined;
  const results = details.results.map(subagentResult);
  if (results.some((result) => !result)) return undefined;
  return { v: 1, mode: details.mode === "parallel" ? "parallel" : "single", results: results as SubagentResult[] };
}

/** A card for a call that has not reported progress yet, built from its arguments. */
export function pendingSubagentDetails(call: NormalizedBlock): SubagentDetails | undefined {
  const toolArgs = args(call);
  const queued = (agent: unknown, task: unknown): SubagentResult | undefined =>
    typeof agent === "string" && agent && typeof task === "string"
      ? { agent, task, readOnly: false, status: "queued", activity: [], usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 0 } }
      : undefined;
  if (Array.isArray(toolArgs.tasks)) {
    const results = toolArgs.tasks.map((item) => {
      const task = item as Record<string, unknown> | null;
      return queued(task?.agent, task?.task);
    });
    if (!results.length || results.some((result) => !result)) return undefined;
    return { v: 1, mode: "parallel", results: results as SubagentResult[] };
  }
  const single = queued(toolArgs.agent, toolArgs.task);
  return single ? { v: 1, mode: "single", results: [single] } : undefined;
}

/** Whether a `subagent` call is still in the conversation: rewinding past it removes it. */
export function hasSubagentCall(messages: NormalizedMessage[], partial: NormalizedMessage | undefined, toolCallId: string): boolean {
  const holds = (message: NormalizedMessage) =>
    message.blocks.some((block) => block.type === "tool-call" && block.toolCallId === toolCallId && block.toolName === SUBAGENT_TOOL_NAME);
  return messages.some(holds) || (partial !== undefined && holds(partial));
}

/** A sub-agent's name as a title: "scout" → "Scout", "code-reviewer" → "Code Reviewer". */
export function displayAgentName(name: string): string {
  const words = name.split(/[-_\s]+/).filter(Boolean);
  return words.length ? words.map((word) => word[0].toUpperCase() + word.slice(1)).join(" ") : name;
}

/**
 * A `subagent` call's details wherever they stand now, by the precedence its chips use: the
 * final result once there is one; while the chat runs, the latest live update, or the call's own
 * arguments until the first update. `finished` says the result has landed. Undefined when the
 * call isn't in the conversation, or was cut off without a result.
 */
export function subagentDetailsFor(
  messages: NormalizedMessage[],
  partial: NormalizedMessage | undefined,
  liveToolDetails: Record<string, unknown> | undefined,
  toolCallId: string,
  running: boolean
): { details: SubagentDetails; finished: boolean } | undefined {
  let call: NormalizedBlock | undefined;
  for (const message of partial ? [...messages, partial] : messages) {
    for (const block of message.blocks) {
      if (block.toolCallId !== toolCallId || block.toolName !== SUBAGENT_TOOL_NAME) continue;
      if (block.type === "tool-result") {
        const details = parseSubagentDetails(block.details);
        return details ? { details, finished: true } : undefined;
      }
      if (block.type === "tool-call") call = block;
    }
  }
  if (!call || !running) return undefined;
  const details = parseSubagentDetails(liveToolDetails?.[toolCallId]) ?? pendingSubagentDetails(call);
  return details ? { details, finished: false } : undefined;
}
