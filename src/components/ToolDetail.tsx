/**
 * What a tool call opens into: one solid sheet, the same under a chip (`ToolChips`) and a Chat
 * mode chip (`chat/ChatActivity.tsx`), and fused into a work row (`ToolCallRow`), whose bar
 * stands in for the head. A sheet has a head — a round mark like the Explored head's, then the
 * call in full (the whole command, the whole path) — over a body shaped by what came back: a command's output, a file's numbered and highlighted
 * lines, a search's hits grouped by file, a listing's entries, a question with the answer
 * picked, or the arguments as labelled rows. Result text in a shape `tool-output.ts` doesn't
 * know falls back to plain text, never to nothing.
 *
 * Copy is offered only on what the agent authored (commands, written files, diffs, a path),
 * never on results or arguments. Line numbers and the prompt are drawn by CSS, so selecting and
 * copying a body takes the source text alone.
 */
import { createContext, Fragment, useContext, useId, useMemo, useState, type CSSProperties, type ReactNode } from "react";
import { highlightBlock, highlightDiffLines, highlightLines, languageForPath } from "../highlight";
import { editArgumentDiff, focusedEditDiff } from "../edit-preview";
import { displayPath } from "../chat-utils";
import type { NormalizedBlock } from "../types";
import { WEB_SEARCH_TOOL_NAME, callArguments, imageReadNote, mcpToolParts, parseWebSearchDetails, splitPathSubject, summarizeTool, type ToolSummary } from "../tool-utils";
import type { WebSearchDetails } from "../types";
import { useLinkOpener } from "../link-opener";
import { WebResults } from "./web/WebResults";
import { argRows, askedQuestions, grepMatcher, parseGrep, readRange, splitCommandStatus, splitMatches, splitNotice, type Asked, type GrepFile } from "../tool-output";
import { Icon, type IconName } from "./Icons";
import { CopyButton } from "./ui/CopyButton";

/** Stable tool-call keys retain disclosure choices when only one tab is mounted. */
export const ToolExpansion = createContext<{ scope: string; keys: Set<string> } | undefined>(undefined);
export function useToolDisclosure(entryKey?: string) {
  const store = useContext(ToolExpansion);
  const expanded = store?.keys;
  const key = entryKey === undefined ? undefined : JSON.stringify([store?.scope ?? "", entryKey]);
  const [open, setOpen] = useState(() => key !== undefined && expanded?.has(key) === true);
  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (key === undefined) return;
    if (next) expanded?.add(key); else expanded?.delete(key);
  };
  return [open, toggle] as const;
}

const TOOL_ICONS: Record<string, IconName> = {
  read: "file",
  edit: "pencil",
  write: "filePlus",
  bash: "terminal",
  grep: "search",
  find: "search",
  ls: "folder",
  web_fetch: "globe",
  web_search: "search",
  browser_open: "browser",
  browser_snapshot: "browser",
  browser_act: "cursor",
  browser_console: "browser",
  ask_user_question: "question",
  plan_mode_complete: "brain",
  todo: "checklist",
  subagent: "agents",
  computer_apps: "cursor",
  computer_open: "cursor",
  computer_snapshot: "cursor",
  computer_screenshot: "image",
  computer_act: "cursor",
  memory_save: "memory",
  memory_recall: "memory",
  memory_forget: "memory",
  browser_screenshot: "image"
};

export function toolIcon(call: NormalizedBlock, result: NormalizedBlock | undefined, summary: ToolSummary): IconName {
  if (summary.kind === "read" && result?.images?.length) return "image";
  return TOOL_ICONS[call.toolName ?? ""] ?? (mcpToolParts(call.toolName ?? "") ? "plug" : "terminal");
}

const plural = (count: number, noun: string, many = `${noun}s`) => `${count} ${count === 1 ? noun : many}`;

/** The sheet's opening line: the call's mark, the call in full, and what there is to say about it. */
function SheetHead({ icon, meta, children }: { icon: IconName; meta?: ReactNode; children: ReactNode }) {
  return (
    <div className="tool-head">
      <span className="tool-head-mark" aria-hidden="true"><Icon name={icon} /></span>
      <div className="tool-head-title">{children}</div>
      {meta}
    </div>
  );
}

/** A whole path, wrapping rather than truncated, with the file name carrying the weight. */
function PathTitle({ path }: { path: string }) {
  const { dir, name } = splitPathSubject(path);
  return <span className="tool-head-path">{dir && <span className="dir">{dir}</span>}<span className="name">{name}</span></span>;
}

function DiffLines({ diff, path }: { diff: string; path?: string }) {
  const lines = useMemo(() => focusedEditDiff(diff).split("\n"), [diff]);
  const classes = lines.map((line) =>
    line.startsWith("+") && !line.startsWith("+++") ? "addition"
      : line.startsWith("-") && !line.startsWith("---") ? "deletion"
      : line.startsWith("@@") ? "hunk"
      : line.startsWith("diff ") || line.startsWith("# ") ? "heading" : "");
  // The edit's own file language highlights the code lines; hunk and heading lines stay plain.
  const nodes = highlightDiffLines(lines, classes.map((name) => (name === "hunk" || name === "heading" ? "meta" : "code")), languageForPath(path));
  return (
    <pre className="diff-view tool-diff">
      {lines.map((line, index) => (
        <span className={classes[index]} key={index}>{nodes[index]}{"\n"}</span>
      ))}
    </pre>
  );
}

/** The lines a pane is showing: `first` is where they start in the pane's whole text. */
interface PaneView {
  id: string;
  lines: string[];
  first: number;
}

/**
 * One pane of a sheet's body. Preview limits only affect rendering: expansion and copying
 * always use the source text. `body` draws the shown lines its own way (numbered code, grouped
 * hits); without it, or when it returns nothing, they are plain text.
 */
export function ToolText({ text, label = "output", caption, tone, maxLines = 60, copy = false, expansionKey, body }: {
  text: string; label?: string; caption?: string; tone?: "error"; maxLines?: number; copy?: boolean; expansionKey?: string;
  body?: (view: PaneView) => ReactNode;
}) {
  const [all, toggleAll] = useToolDisclosure(expansionKey ? `${expansionKey}:${label}` : undefined);
  const id = useId();
  const lines = useMemo(() => text.split("\n"), [text]);
  const truncated = lines.length > maxLines;
  const shown = truncated && !all ? lines.slice(-maxLines) : lines;
  // The toolbar exists only for a caption or the truncation controls, and copy (where offered at
  // all) rides in it; a bare pane has no row, so copy floats over its corner on hover instead.
  return (
    <div className={tone ? `tool-text ${tone}` : "tool-text"}>
      {truncated || caption ? (
        <div className="tool-text-toolbar">
          {caption && <span className="tool-text-caption">{caption}</span>}
          {truncated && (
            <>
              <span className="tool-text-count">{all ? `All ${lines.length} lines` : `Last ${maxLines} of ${lines.length} lines`}</span>
              <button type="button" className="text-button" aria-expanded={all} aria-controls={id} onClick={toggleAll}>
                {all ? "Show less" : "Show all"}
              </button>
            </>
          )}
          {copy && <CopyButton text={text} label={`Copy ${label}`} />}
        </div>
      ) : copy ? (
        <CopyButton text={text} label={`Copy ${label}`} floating />
      ) : null}
      {body?.({ id, lines: shown, first: lines.length - shown.length }) ?? <pre id={id}>{shown.join("\n")}</pre>}
    </div>
  );
}

/** Custom properties a body sets for its line-number gutter. */
const gutter = (lastNumber: number, extra?: CSSProperties) => ({ "--gutter": `${String(lastNumber).length}ch`, ...extra }) as CSSProperties;

/** A file's text: highlighted by its path's language, each line numbered as the file numbers it. */
function CodePane({ text, path, start = 1, ...pane }: { text: string; path: string; start?: number; label?: string; maxLines?: number; copy?: boolean; expansionKey?: string }) {
  const tokens = useMemo(() => highlightLines(text, languageForPath(path)), [text, path]);
  const last = tokens.length - 1;
  return (
    <ToolText text={text} {...pane} body={({ id, lines, first }) => (
      <pre id={id} className="tool-code" style={gutter(start + last, { counterReset: `line ${start + first - 1}` })}>
        {lines.map((_, index) => <span key={first + index}>{tokens[first + index]}{first + index < last ? "\n" : ""}</span>)}
      </pre>
    )} />
  );
}

/** A command as it was written, then what it printed; a failure wears how it ended as a pill. */
function RunSheet({ command, result, failed, expansionKey }: { command: string; result?: NormalizedBlock; failed: boolean; expansionKey?: string }) {
  const text = result?.text ?? "";
  const ended = failed ? splitCommandStatus(text) : { output: text, status: undefined };
  const { body, notice } = splitNotice(ended.output);
  return (
    <>
      {/* Copy follows what the agent authored — the command — never the output it got back. */}
      <SheetHead icon="terminal" meta={<>{ended.status && <span className="tool-pill failed">{ended.status}</span>}<CopyButton text={command} label="Copy command" /></>}>
        <pre className="tool-command">{highlightBlock(command, "bash")}</pre>
      </SheetHead>
      {body && <ToolText text={body} expansionKey={expansionKey} />}
      {notice && <div className="tool-foot">{notice}</div>}
    </>
  );
}

function FileSheet({ path, args, text, expansionKey }: { path: string; args: Record<string, unknown>; text: string; expansionKey?: string }) {
  const { body, notice } = useMemo(() => splitNotice(text), [text]);
  const range = readRange(args, body, notice);
  return (
    <>
      <SheetHead icon="file" meta={<span className="tool-head-meta">{range.label}</span>}><PathTitle path={path} /></SheetHead>
      <CodePane text={body} path={path} start={range.first} expansionKey={expansionKey} />
    </>
  );
}

/** A grep's hits under the file they were found in, the pattern lit where it matched. */
function Matches({ id, files, matcher }: { id: string; files: GrepFile[]; matcher?: RegExp }) {
  const widest = Math.max(...files.flatMap((file) => file.rows.map((row) => row.line)));
  return (
    <pre id={id} className="tool-matches" style={gutter(widest)}>
      {files.map((file, fileIndex) => {
        const { dir, name } = splitPathSubject(file.path);
        return (
          <Fragment key={fileIndex}>
            <span className="match-file">{dir && <span className="dir">{dir}</span>}<span className="name">{name}</span>{"\n"}</span>
            {file.rows.map((row, rowIndex) => (
              <span key={rowIndex} className={row.hit ? "match-row hit" : "match-row"} data-line={row.line}>
                {splitMatches(row.text, row.hit ? matcher : undefined).map((part, index) => part.hit ? <mark key={index}>{part.text}</mark> : part.text)}{"\n"}
              </span>
            ))}
          </Fragment>
        );
      })}
    </pre>
  );
}

/** A listing's entries, folders marked; `paths` keeps each on its own line with its directory dimmed. */
function Entries({ id, lines, paths }: { id: string; lines: string[]; paths: boolean }) {
  return (
    <div id={id} className={paths ? "tool-entries paths" : "tool-entries"}>
      {lines.filter(Boolean).map((entry, index) => {
        const folder = entry.endsWith("/");
        const shown = folder ? entry.slice(0, -1) : entry;
        const { dir, name } = paths ? splitPathSubject(shown) : { dir: "", name: shown };
        return (
          <span key={index} className={folder ? "tool-entry folder" : "tool-entry"}>
            <Icon name={folder ? "folder" : "file"} />
            <span className="tool-entry-name">{dir && <span className="dir">{dir}</span>}{name}</span>
          </span>
        );
      })}
    </div>
  );
}

const NOTHING_FOUND = /^(No matches found|No files found matching pattern|\(empty directory\))$/;

function SearchSheet({ call, args, result, failed, icon, expansionKey }: { call: NormalizedBlock; args: Record<string, unknown>; result?: NormalizedBlock; failed: boolean; icon: IconName; expansionKey?: string }) {
  const text = result?.text ?? "";
  const tool = call.toolName;
  const { body, notice } = useMemo(() => splitNotice(text), [text]);
  const nothing = !body || NOTHING_FOUND.test(body);
  const structured = !failed && !nothing;
  const hits = useMemo(() => (structured && tool === "grep" ? parseGrep(body.split("\n")) : undefined), [structured, tool, body]);
  const matcher = useMemo(() => grepMatcher(args), [args]);
  const found = useMemo(() => body.split("\n").filter(Boolean).length, [body]);
  const where = typeof args.path === "string" && args.path ? args.path : ".";
  const pattern = typeof args.pattern === "string" ? args.pattern : "";

  const count = !structured ? undefined
    : hits ? `${plural(hits.reduce((sum, file) => sum + file.rows.filter((row) => row.hit).length, 0), "match", "matches")} in ${plural(hits.length, "file")}`
    : tool === "ls" ? plural(found, "entry", "entries")
    : tool === "find" ? plural(found, "file") : undefined;
  return (
    <>
      <SheetHead icon={icon} meta={count && <span className="tool-head-meta">{count}</span>}>
        {tool === "ls" ? <PathTitle path={where} /> : (
          <span className="tool-head-code">
            <span className="pattern">{pattern}</span>
            <span className="where"> in {displayPath(where)}{typeof args.glob === "string" && args.glob ? ` (${args.glob})` : ""}</span>
          </span>
        )}
      </SheetHead>
      {failed ? <ToolText text={text} tone="error" expansionKey={expansionKey} />
        : nothing ? <div className="tool-empty">{tool === "ls" ? "Empty folder" : tool === "find" ? "No files found" : "No matches"}</div>
        : (
          <ToolText text={body} expansionKey={expansionKey} body={({ id, lines }) => {
            if (tool === "ls" || tool === "find") return <Entries id={id} lines={lines} paths={tool === "find"} />;
            // The shown lines are parsed on their own: a preview may begin partway through a file's hits.
            const files = hits ? parseGrep(lines) : undefined;
            return files ? <Matches id={id} files={files} matcher={matcher} /> : undefined;
          }} />
        )}
      {notice && <div className="tool-foot">{notice}</div>}
    </>
  );
}

const ASK_OUTCOMES: Record<Asked["outcome"], string> = {
  waiting: "Waiting for you",
  answered: "Answered",
  dismissed: "Dismissed",
  "wrapped-up": "Skipped to the plan"
};

/** The questions as they were put, every option still listed, with what was picked marked. */
function AskSheet({ asked }: { asked: Asked }) {
  return (
    <>
      <SheetHead icon="question" meta={<span className={`tool-pill ${asked.outcome}`}>{ASK_OUTCOMES[asked.outcome]}</span>}>
        <span className="tool-head-words">{asked.questions.length > 1 ? `Asked you ${asked.questions.length} questions` : "Asked you"}</span>
      </SheetHead>
      <div className="tool-questions">
        {asked.questions.map((question, index) => (
          <section key={index} className="tool-question">
            {question.header && <span className="tool-question-tag">{question.header}</span>}
            <div className="tool-question-text">{question.question}</div>
            {/* Not ul/li/p: prose list and paragraph rules reach into an assistant message. */}
            <div className="tool-options" role="list">
              {question.options.map((option, optionIndex) => (
                <div key={optionIndex} role="listitem" className={option.chosen ? "tool-option chosen" : "tool-option"} aria-current={option.chosen ? "true" : undefined}>
                  <span className={question.multi ? "tool-option-mark box" : "tool-option-mark"} aria-hidden="true">{option.chosen && <Icon name="check" />}</span>
                  <span className="tool-option-label">{option.label}</span>
                  {option.description && <span className="tool-option-note">{option.description}</span>}
                </div>
              ))}
              {question.custom && (
                <div role="listitem" className="tool-option chosen written" aria-current="true">
                  <span className="tool-option-mark" aria-hidden="true"><Icon name="pencil" /></span>
                  <span className="tool-option-label">{question.custom}</span>
                  <span className="tool-option-note">Written answer</span>
                </div>
              )}
            </div>
          </section>
        ))}
      </div>
    </>
  );
}

/**
 * web_search: the query in full, the provider and count, then the results as cards. A result
 * opens in the chat's browser preview (⌘-click: the default browser).
 */
function WebSearchSheet({ search, head }: { search: WebSearchDetails; head: boolean }) {
  const open = useLinkOpener();
  const count = search.results.length;
  return (
    <>
      {head && (
        <SheetHead icon="search" meta={<span className="tool-pill">{search.provider.name} · {plural(count, "result")}</span>}>
          <span className="tool-head-query">{search.query}</span>
        </SheetHead>
      )}
      <div className="tool-web-results">
        {count === 0
          ? <p className="tool-web-empty">No results.</p>
          : <WebResults results={search.results} label={`Results for ${search.query}`} onOpen={(url, event) => open(url, { external: event.metaKey })} />}
      </div>
    </>
  );
}

/** web_fetch's sheet adds who read the page, when a search provider did. */
function viaPill(details: Record<string, unknown>): ReactNode {
  return typeof details.via === "string" && details.via ? <span className="tool-pill">via {details.via}</span> : undefined;
}

/** Any other call: its name, its arguments as labelled rows, and what came back. */
function CallSheet({ call, args, result, failed, icon, head, expansionKey }: { call: NormalizedBlock; args: Record<string, unknown>; result?: NormalizedBlock; failed: boolean; icon: IconName; head: boolean; expansionKey?: string }) {
  const rows = argRows(args);
  const mcp = mcpToolParts(call.toolName ?? "");
  const via = viaPill((result?.details ?? {}) as Record<string, unknown>);
  return (
    <>
      {head && (
        <SheetHead icon={icon} meta={mcp ? <span className="tool-head-meta">{mcp.server}</span> : via}>
          <span className="tool-head-code"><span className="pattern">{mcp ? mcp.tool : call.toolName ?? "tool"}</span></span>
        </SheetHead>
      )}
      {rows.length > 0 && (
        <dl className="tool-args">
          {rows.map((row) => (
            <div key={row.key} className={`tool-arg ${row.kind}`}>
              <dt>{row.key}</dt>
              <dd>{row.kind === "data" ? highlightBlock(row.value, "json") : row.value}</dd>
            </div>
          ))}
        </dl>
      )}
      {result?.text && <ToolText text={result.text} caption={failed ? "Error" : "Result"} tone={failed ? "error" : undefined} expansionKey={expansionKey} />}
    </>
  );
}

/**
 * A call's sheet. `live` marks a call still running (its output grows a waiting bead). `head`
 * is off inside a work row, whose bar is the head: the sheet then opens straight into its body,
 * except that a command still leads with itself in full.
 */
export function ToolDetail({ call, result, live = false, head = true }: { call: NormalizedBlock; result?: NormalizedBlock; live?: boolean; head?: boolean }) {
  const summary = summarizeTool(call, result);
  const args = callArguments(call);
  const details = (result?.details ?? {}) as Record<string, unknown>;
  const failed = result?.isError === true;
  const path = typeof args.path === "string" ? args.path : "";
  const key = call.toolCallId;
  const icon = toolIcon(call, result, summary);

  const editDiff = useMemo(() => summary.kind === "edit" && !failed
    ? (typeof details.diff === "string" && details.diff) || editArgumentDiff(args)
    : undefined, [summary.kind, failed, details.diff, args]);
  const asked = useMemo(() => (call.toolName === "ask_user_question" ? askedQuestions(args, result) : undefined), [call.toolName, args, result]);
  const search = useMemo(() => (call.toolName === WEB_SEARCH_TOOL_NAME && !failed ? parseWebSearchDetails(result?.details) : undefined), [call.toolName, failed, result?.details]);

  let sheet: ReactNode;
  if (editDiff) {
    sheet = (
      <>
        {head && <SheetHead icon={icon}><PathTitle path={path} /></SheetHead>}
        <div className="tool-text">
          <CopyButton text={editDiff} label="Copy diff" floating />
          <DiffLines diff={editDiff} path={path || undefined} />
        </div>
      </>
    );
  } else if (summary.kind === "bash") {
    sheet = <RunSheet command={typeof args.command === "string" ? args.command : ""} result={result} failed={failed} expansionKey={key} />;
  } else if (summary.kind === "write" && typeof args.content === "string") {
    sheet = (
      <>
        {head && <SheetHead icon={icon}><PathTitle path={path} /></SheetHead>}
        <CodePane text={args.content} path={path} label="file content" maxLines={80} copy expansionKey={key} />
        {failed && result?.text && <ToolText text={result.text} label="error" caption="Error" tone="error" expansionKey={key} />}
      </>
    );
  } else if (summary.kind === "read" && result?.images?.length) {
    // The picture is under the row already; what's left to say is where it is and Pi's notes.
    const note = imageReadNote(result.text);
    sheet = (
      <>
        <SheetHead icon="image" meta={<CopyButton text={path} label="Copy path" />}><pre className="tool-head-pre">{path}</pre></SheetHead>
        {note && <ToolText text={note} label="note" expansionKey={key} />}
      </>
    );
  } else if (summary.kind === "read" && result && !failed) {
    sheet = <FileSheet path={path} args={args} text={result.text ?? ""} expansionKey={key} />;
  } else if (summary.kind === "search") {
    sheet = <SearchSheet call={call} args={args} result={result} failed={failed} icon={icon} expansionKey={key} />;
  } else if (asked) {
    sheet = <AskSheet asked={asked} />;
  } else if (search) {
    sheet = <WebSearchSheet search={search} head={head} />;
  } else {
    sheet = <CallSheet call={call} args={args} result={result} failed={failed} icon={icon} head={head} expansionKey={key} />;
  }
  return <div className={`tool-detail${failed ? " failed" : ""}${live ? " live" : ""}`}>{sheet}</div>;
}
