import { useState } from "react";
import {
  MCP_DEFAULT_TIMEOUT_MS,
  MCP_MAX_TIMEOUT_MS,
  MCP_MIN_TIMEOUT_MS,
  type McpServerRecord,
  type McpTestResult,
  type McpTransport,
  type SaveMcpServerInput
} from "../types";
import { DuckMark } from "./DuckMark";
import { Icon, type IconName } from "./Icons";
import { SettingsHero, stagger } from "./SettingsHero";
import { ConfirmDialog } from "./ui/ConfirmDialog";

export interface McpActions {
  /** Rejects with a user-facing message. */
  onSaveMcpServer: (input: SaveMcpServerInput) => Promise<McpServerRecord>;
  onDeleteMcpServer: (serverId: string) => Promise<void>;
  onSetMcpServerEnabled: (serverId: string, enabled: boolean) => Promise<void>;
  /** The server's own tool names that are switched off. */
  onSetMcpServerTools: (serverId: string, disabledTools: string[]) => Promise<void>;
  onTestMcpServer: (serverId: string) => Promise<McpTestResult>;
}

interface Props extends McpActions {
  servers: McpServerRecord[];
  /** The agent's name in the app's own copy (Settings › Appearance). */
  agentName?: string;
}

const TRANSPORTS: { value: McpTransport; label: string; hint: string }[] = [
  { value: "stdio", label: "stdio", hint: "A local command WackCode starts, talking over stdin and stdout." },
  { value: "http", label: "HTTP", hint: "A server at a URL, using Streamable HTTP." },
  { value: "sse", label: "SSE", hint: "A server at a URL, using the older HTTP + Server-Sent Events transport." }
];

const transportLabel = (transport: McpTransport) => TRANSPORTS.find((item) => item.value === transport)?.label ?? transport;

/** The empty state's two ways in; each opens the editor with its transport chosen. */
const STARTS: { transport: McpTransport; icon: IconName; title: string; detail: string; kind: string }[] = [
  { transport: "stdio", icon: "terminal", title: "A command on your Mac", detail: "npx, uvx, docker or any program that speaks MCP over stdin and stdout.", kind: "stdio" },
  { transport: "http", icon: "globe", title: "A server at a URL", detail: "A hosted server over Streamable HTTP or SSE, with headers for its token.", kind: "HTTP · SSE" }
];

/**
 * The hero's little stage: the duck on the left, cabled to two servers. Each switched-on server
 * (up to two) lights up and draws a stream of dots along its cable; with none, the cables and
 * racks rest dashed. Pure decoration; the pill says the same in words. The flow lives in
 * styles.css, which stills it under reduced motion.
 */
function McpStage({ servers, lit }: { servers: number; lit: number }) {
  const racks = [22, 62];
  return (
    <svg className={`settings-stage mcp-stage ${lit > 0 ? "live" : ""}`} viewBox="0 0 160 110" aria-hidden="true">
      {racks.map((y, index) => {
        const cable = `M58 55C82 55 80 ${y + 13} 102 ${y + 13}`;
        return (
          <g key={y} className={`mcp-stage-link ${index < servers ? "present" : ""} ${index < lit ? "on" : ""}`} style={{ "--k": index } as React.CSSProperties}>
            <path className="mcp-stage-cable" d={cable} />
            <path className="mcp-stage-flow" d={cable} />
            <rect className="mcp-stage-rack" x="102" y={y} width="46" height="26" rx="7" />
            <path className="mcp-stage-slots" d={`M110 ${y + 9}h18M110 ${y + 17}h12`} />
            <circle className="mcp-stage-led" cx="140" cy={y + 13} r="3" />
          </g>
        );
      })}
      <rect className="mcp-stage-app" x="14" y="33" width="44" height="44" rx="12" />
      <DuckMark className="mcp-stage-duck" x="24" y="43" width="24" height="24" />
    </svg>
  );
}

let nextEntryKey = 0;

/** A header or environment variable row. `saved`: a value is stored already, so blank keeps it. */
interface EntryDraft {
  key: number;
  name: string;
  value: string;
  saved: boolean;
}

interface ServerDraft {
  id?: string;
  name: string;
  transport: McpTransport;
  timeoutMs: string;
  command: string;
  /** One argument per line. */
  args: string;
  url: string;
  headers: EntryDraft[];
  env: EntryDraft[];
}

type TestState = { state: "testing" } | { state: "ok"; tools: number } | { state: "failed"; error: string };

const savedEntries = (names: string[]): EntryDraft[] => names.map((name) => ({ key: ++nextEntryKey, name, value: "", saved: true }));

function draftFrom(server?: McpServerRecord): ServerDraft {
  return {
    id: server?.id,
    name: server?.name ?? "",
    transport: server?.transport ?? "stdio",
    timeoutMs: String(server?.timeoutMs ?? MCP_DEFAULT_TIMEOUT_MS),
    command: server?.command ?? "",
    args: server?.args.join("\n") ?? "",
    url: server?.url ?? "",
    headers: savedEntries(server?.headers ?? []),
    env: savedEntries(server?.env ?? [])
  };
}

function inputFrom(draft: ServerDraft): SaveMcpServerInput {
  const entries = (list: EntryDraft[]) =>
    list.filter((entry) => entry.name.trim() || entry.value.trim()).map((entry) => ({ name: entry.name.trim(), value: entry.value || undefined }));
  const stdio = draft.transport === "stdio";
  return {
    id: draft.id,
    name: draft.name.trim(),
    transport: draft.transport,
    timeoutMs: Number(draft.timeoutMs),
    command: stdio ? draft.command.trim() : "",
    args: stdio ? draft.args.split("\n").map((arg) => arg.trim()).filter(Boolean) : [],
    url: stdio ? "" : draft.url.trim(),
    headers: stdio ? [] : entries(draft.headers),
    env: stdio ? entries(draft.env) : []
  };
}

function timeoutProblem(value: string): string | undefined {
  const timeout = Number(value);
  if (!value.trim() || !Number.isInteger(timeout) || timeout < MCP_MIN_TIMEOUT_MS || timeout > MCP_MAX_TIMEOUT_MS) {
    return `Between ${MCP_MIN_TIMEOUT_MS.toLocaleString()} and ${MCP_MAX_TIMEOUT_MS.toLocaleString()} ms`;
  }
  return undefined;
}

function summary(server: McpServerRecord): string {
  return server.transport === "stdio" ? [server.command, ...server.args].join(" ") : server.url;
}

interface EntriesProps {
  label: string;
  hint: string;
  noun: string;
  namePlaceholder: string;
  entries: EntryDraft[];
  disabled: boolean;
  onChange: (entries: EntryDraft[]) => void;
}

/** Name/value rows whose values are stored as secrets: never shown again, blank keeps the saved one. */
function SecretEntries({ label, hint, noun, namePlaceholder, entries, disabled, onChange }: EntriesProps) {
  const update = (key: number, patch: Partial<EntryDraft>) =>
    onChange(entries.map((entry) => entry.key === key ? { ...entry, ...patch } : entry));
  return (
    <div className="mcp-entries">
      <span className="subagent-field-label">{label} <small className="subagent-hint">{hint}</small></span>
      {entries.map((entry) => (
        <div className="mcp-entry-row" key={entry.key}>
          <input
            aria-label={`${noun} name`}
            value={entry.name}
            placeholder={namePlaceholder}
            spellCheck={false}
            autoComplete="off"
            disabled={disabled}
            // Renaming a saved entry drops its stored value: the new name needs its own.
            onChange={(event) => update(entry.key, { name: event.target.value, saved: false })}
          />
          <input
            type="password"
            aria-label={`${entry.name || noun} value`}
            value={entry.value}
            placeholder={entry.saved ? "Saved — leave blank to keep" : "Value"}
            autoComplete="off"
            disabled={disabled}
            onChange={(event) => update(entry.key, { value: event.target.value })}
          />
          <button
            type="button"
            className="icon-button"
            aria-label={`Remove ${entry.name || noun}`}
            disabled={disabled}
            onClick={() => onChange(entries.filter((item) => item.key !== entry.key))}
          >
            <Icon name="close" />
          </button>
        </div>
      ))}
      <button
        type="button"
        className="secondary-button mcp-add-entry"
        disabled={disabled}
        onClick={() => onChange([...entries, { key: ++nextEntryKey, name: "", value: "", saved: false }])}
      >
        <Icon name="plus" /> Add {noun}
      </button>
    </div>
  );
}

interface EditorProps {
  draft: ServerDraft;
  busy: boolean;
  onChange: (draft: ServerDraft) => void;
  onSave: () => void;
  onCancel: () => void;
  onDelete?: () => void;
}

function ServerEditor({ draft, busy, onChange, onSave, onCancel, onDelete }: EditorProps) {
  const stdio = draft.transport === "stdio";
  const timeoutIssue = timeoutProblem(draft.timeoutMs);
  const incomplete = !draft.name.trim() || Boolean(timeoutIssue) || (stdio ? !draft.command.trim() : !draft.url.trim());
  return (
    <div className="subagent-editor mcp-editor">
      <div className="form-grid">
        <label>
          <span>Name <small>Shown in chats and in the server's tool names</small></span>
          <input value={draft.name} onChange={(event) => onChange({ ...draft, name: event.target.value })} placeholder="GitHub" maxLength={40} disabled={busy} />
        </label>
        <label>
          <span>Timeout (ms) <small className={timeoutIssue ? "mcp-invalid" : ""}>{timeoutIssue ?? "For starting up and for each tool call"}</small></span>
          <input
            type="number"
            inputMode="numeric"
            min={MCP_MIN_TIMEOUT_MS}
            max={MCP_MAX_TIMEOUT_MS}
            step={1000}
            value={draft.timeoutMs}
            onChange={(event) => onChange({ ...draft, timeoutMs: event.target.value })}
            aria-invalid={Boolean(timeoutIssue)}
            disabled={busy}
          />
        </label>
        <div className="wide-field mcp-transport-field">
          <span className="subagent-field-label">Type</span>
          <div className="mode-toggle" role="radiogroup" aria-label="Type">
            {TRANSPORTS.map((transport) => (
              <button
                key={transport.value}
                type="button"
                role="radio"
                aria-checked={draft.transport === transport.value}
                className={`mode-option ${draft.transport === transport.value ? "active" : ""}`}
                disabled={busy}
                onClick={() => onChange({ ...draft, transport: transport.value })}
              >
                {transport.label}
              </button>
            ))}
          </div>
          <small className="subagent-hint">{TRANSPORTS.find((item) => item.value === draft.transport)?.hint}</small>
        </div>
        {stdio ? (
          <>
            <label className="wide-field">
              <span>Command <small>The program to run, such as npx, uvx or docker</small></span>
              <input className="mcp-mono" value={draft.command} onChange={(event) => onChange({ ...draft, command: event.target.value })} placeholder="npx" spellCheck={false} autoComplete="off" disabled={busy} />
            </label>
            <label className="wide-field">
              <span>Arguments <small>One per line</small></span>
              <textarea
                className="subagent-prompt-input mcp-args-input"
                value={draft.args}
                onChange={(event) => onChange({ ...draft, args: event.target.value })}
                placeholder={"-y\n@modelcontextprotocol/server-filesystem\n/Users/me/Documents"}
                rows={3}
                spellCheck={false}
                disabled={busy}
              />
            </label>
          </>
        ) : (
          <label className="wide-field">
            <span>URL</span>
            <input className="mcp-mono" value={draft.url} onChange={(event) => onChange({ ...draft, url: event.target.value })} placeholder="https://example.com/mcp" spellCheck={false} autoComplete="off" disabled={busy} />
          </label>
        )}
      </div>
      {stdio ? (
        <SecretEntries
          label="Environment variables"
          hint="Optional. Only this server gets them; values are stored on this device."
          noun="variable"
          namePlaceholder="API_TOKEN"
          entries={draft.env}
          disabled={busy}
          onChange={(env) => onChange({ ...draft, env })}
        />
      ) : (
        <SecretEntries
          label="Headers"
          hint="Optional, e.g. Authorization: Bearer <token>. Sent only to this server; values are stored on this device."
          noun="header"
          namePlaceholder="Authorization"
          entries={draft.headers}
          disabled={busy}
          onChange={(headers) => onChange({ ...draft, headers })}
        />
      )}
      {stdio && <small className="subagent-hint">This command runs on your Mac, with your permissions, in the chat's project folder whenever a chat uses this server.</small>}
      <div className="subagent-editor-actions">
        {onDelete && <button type="button" className="danger-button" disabled={busy} onClick={onDelete}>Delete</button>}
        <span className="subagent-editor-spacer" />
        <button type="button" className="secondary-button" disabled={busy} onClick={onCancel}>Cancel</button>
        <button type="button" className="primary-button" disabled={busy || incomplete} onClick={onSave}>{draft.id ? "Save" : "Add server"}</button>
      </div>
    </div>
  );
}

/**
 * Settings › MCP servers. Switches save at once; a server's settings are edited as a draft and
 * saved explicitly, after which the connection is tested so its tools can be listed. Header and
 * environment values never come back from the host: a saved one shows as "Saved".
 */
export function McpSection({ servers, agentName = "WackCode", onSaveMcpServer, onDeleteMcpServer, onSetMcpServerEnabled, onSetMcpServerTools, onTestMcpServer }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [expanded, setExpanded] = useState<string>();
  /** The server being edited (by id), or "new" for one not yet saved. */
  const [editing, setEditing] = useState<{ key: string; draft: ServerDraft }>();
  const [deleting, setDeleting] = useState<McpServerRecord>();
  const [tests, setTests] = useState<Record<string, TestState>>({});

  async function run(action: () => Promise<unknown>): Promise<boolean> {
    setBusy(true);
    setError(undefined);
    try {
      await action();
      return true;
    } catch (reason) {
      setError(String(reason));
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function test(serverId: string) {
    setTests((current) => ({ ...current, [serverId]: { state: "testing" } }));
    let next: TestState;
    try {
      const result = await onTestMcpServer(serverId);
      next = result.ok ? { state: "ok", tools: result.server.tools.length } : { state: "failed", error: result.error ?? "The server could not be reached." };
    } catch (reason) {
      next = { state: "failed", error: String(reason) };
    }
    setTests((current) => ({ ...current, [serverId]: next }));
  }

  async function saveDraft() {
    if (!editing) return;
    let saved: McpServerRecord | undefined;
    if (await run(async () => { saved = await onSaveMcpServer(inputFrom(editing.draft)); })) {
      setEditing(undefined);
      if (saved) {
        setExpanded(saved.id);
        void test(saved.id);
      }
    }
  }

  function toggleTool(server: McpServerRecord, tool: string) {
    const disabled = server.disabledTools.includes(tool)
      ? server.disabledTools.filter((name) => name !== tool)
      : [...server.disabledTools, tool];
    void run(() => onSetMcpServerTools(server.id, disabled));
  }

  function renderStatus(server: McpServerRecord) {
    const status = tests[server.id];
    if (!status) return null;
    if (status.state === "testing") return <p className="mcp-status">Testing the connection…</p>;
    if (status.state === "ok") return <p className="mcp-status ok">Connected · {status.tools} {status.tools === 1 ? "tool" : "tools"}</p>;
    return <p className="mcp-status failed" role="status">{status.error}</p>;
  }

  function renderServer(server: McpServerRecord) {
    const open = expanded === server.id;
    if (editing?.key === server.id) {
      return (
        <article className="subagent-setting mcp-server open" key={server.id}>
          <ServerEditor
            draft={editing.draft}
            busy={busy}
            onChange={(draft) => setEditing({ key: server.id, draft })}
            onSave={() => void saveDraft()}
            onCancel={() => { setEditing(undefined); setError(undefined); }}
            onDelete={() => setDeleting(server)}
          />
        </article>
      );
    }
    const testing = tests[server.id]?.state === "testing";
    return (
      <article className={`subagent-setting mcp-server ${open ? "open" : ""} ${server.enabled ? "" : "off"}`} key={server.id}>
        <div className="subagent-setting-head">
          <button type="button" className={`package-disclosure ${open ? "open" : ""}`} aria-expanded={open} onClick={() => setExpanded(open ? undefined : server.id)}>
            <Icon name="chevron" />
            <span className="subagent-setting-name">{server.name}</span>
            <span className="subagent-badge">{transportLabel(server.transport)}</span>
          </button>
          <span className="subagent-model-summary mcp-summary" title={summary(server)}>{summary(server)}</span>
          <button
            type="button"
            role="switch"
            aria-checked={server.enabled}
            aria-label={`Use ${server.name}`}
            className={`toggle ${server.enabled ? "on" : ""}`}
            disabled={busy}
            onClick={() => void run(() => onSetMcpServerEnabled(server.id, !server.enabled))}
          >
            <span />
          </button>
        </div>
        {renderStatus(server)}
        {open && (
          <div className="subagent-setting-body">
            <dl className="mcp-details">
              {server.transport === "stdio" ? (
                <>
                  <dt>Command</dt><dd className="mcp-mono">{server.command}</dd>
                  {server.args.length > 0 && <><dt>Arguments</dt><dd className="mcp-mono">{server.args.join(" ")}</dd></>}
                  {server.env.length > 0 && <><dt>Environment</dt><dd className="mcp-mono">{server.env.join(", ")}</dd></>}
                </>
              ) : (
                <>
                  <dt>URL</dt><dd className="mcp-mono">{server.url}</dd>
                  {server.headers.length > 0 && <><dt>Headers</dt><dd className="mcp-mono">{server.headers.join(", ")}</dd></>}
                </>
              )}
              <dt>Timeout</dt><dd>{server.timeoutMs.toLocaleString()} ms</dd>
            </dl>
            <div className="subagent-tools-field">
              <span className="subagent-field-label">Tools</span>
              {server.tools.length === 0 ? (
                <small className="subagent-hint">Test the connection to list this server's tools. They're all on until you switch one off.</small>
              ) : (
                <section className="resource-group mcp-tools" aria-label={`${server.name} tools`}>
                  {server.tools.map((tool) => {
                    const on = !server.disabledTools.includes(tool.name);
                    return (
                      <div className="resource-row mcp-tool-row" key={tool.name}>
                        <div className="mcp-tool-text">
                          <span className="resource-name">{tool.name}</span>
                          {tool.readOnly && <span className="subagent-badge" title="The server marks it read-only, so it also works in Plan mode">Read-only</span>}
                          {tool.description && <small className="mcp-tool-description">{tool.description}</small>}
                        </div>
                        <button
                          type="button"
                          role="switch"
                          aria-checked={on}
                          aria-label={`Use ${tool.name}`}
                          className={`toggle ${on ? "on" : ""}`}
                          disabled={busy}
                          onClick={() => toggleTool(server, tool.name)}
                        >
                          <span />
                        </button>
                      </div>
                    );
                  })}
                </section>
              )}
            </div>
            <div className="subagent-editor-actions">
              <button type="button" className="danger-button" disabled={busy} onClick={() => setDeleting(server)}>Delete</button>
              <span className="subagent-editor-spacer" />
              <button type="button" className="secondary-button" disabled={busy || testing} onClick={() => void test(server.id)}>
                <Icon name="refresh" /> {testing ? "Testing…" : "Test connection"}
              </button>
              <button type="button" className="secondary-button" disabled={busy} onClick={() => setEditing({ key: server.id, draft: draftFrom(server) })}>
                <Icon name="pencil" /> Edit
              </button>
            </div>
          </div>
        )}
      </article>
    );
  }

  function startNew(transport: McpTransport = "stdio") {
    setEditing({ key: "new", draft: { ...draftFrom(), transport } });
    setExpanded(undefined);
    setError(undefined);
  }

  const enabled = servers.filter((server) => server.enabled).length;
  const pill = servers.length === 0 ? "No servers"
    : enabled === 0 ? "All off"
    : enabled === servers.length ? `${enabled} ${enabled === 1 ? "server" : "servers"} on`
    : `${enabled} of ${servers.length} on`;
  const creating = editing?.key === "new";

  return (
    <div className="settings-scroll subagents-settings mcp-settings">
      <div className="settings-page">
        <SettingsHero
          label="MCP servers overview"
          stage={<McpStage servers={servers.length} lit={enabled} />}
          live={enabled > 0}
          pill={pill}
          title={`Plug more tools into ${agentName}`}
          action={(
            <button type="button" className="secondary-button compact" disabled={busy || creating} onClick={() => startNew()}>
              <Icon name="plus" /> New server
            </button>
          )}
        >
          <p>
            An MCP server hands the agent tools of its own: your issue tracker, a database, a design file. Switched-on servers
            join every chat when it sends its first message.
          </p>
        </SettingsHero>
        {error && <div className="error-banner" role="alert">{error}</div>}

        {editing?.key === "new" && (
          <section className="settings-block mcp-new" style={stagger(1)} aria-labelledby="mcp-new-title">
            <h3 className="settings-block-title" id="mcp-new-title">New server</h3>
            <ServerEditor
              draft={editing.draft}
              busy={busy}
              onChange={(draft) => setEditing({ key: "new", draft })}
              onSave={() => void saveDraft()}
              onCancel={() => { setEditing(undefined); setError(undefined); }}
            />
          </section>
        )}

        {servers.length === 0 && !creating && (
          <section className="settings-block" style={stagger(1)} aria-labelledby="mcp-empty-title">
            <h3 className="settings-block-title" id="mcp-empty-title">No servers yet</h3>
            <p className="settings-block-sub">A server&rsquo;s README usually gives a command to run or a URL to connect to. Start with whichever yours has.</p>
            <div className="mcp-starts">
              {STARTS.map((start) => (
                <button type="button" className="mcp-start" key={start.transport} disabled={busy} onClick={() => startNew(start.transport)}>
                  <span className="mcp-start-mark" aria-hidden="true"><Icon name={start.icon} /></span>
                  <span className="mcp-start-text">
                    <strong>{start.title}</strong>
                    <span>{start.detail}</span>
                  </span>
                  <code aria-hidden="true">{start.kind}</code>
                </button>
              ))}
            </div>
          </section>
        )}

        {servers.length > 0 && (
          <section className="settings-block mcp-servers" style={stagger(creating ? 2 : 1)} aria-labelledby="mcp-servers-title">
            <h3 className="settings-block-title" id="mcp-servers-title">Servers</h3>
            <p className="settings-block-sub">
              Changes apply to running chats from their next message. Tools a server marks read-only also work in Plan mode.
            </p>
            <div className="subagent-group">{servers.map(renderServer)}</div>
          </section>
        )}
      </div>

      {deleting && (
        <ConfirmDialog
          title={`Delete ${deleting.name}?`}
          body="Its tools stop being offered to the agent, and its saved headers and environment values are deleted from this device."
          confirmLabel="Delete"
          danger
          onConfirm={async () => {
            await onDeleteMcpServer(deleting.id);
            if (editing?.key === deleting.id) setEditing(undefined);
            setDeleting(undefined);
          }}
          onCancel={() => setDeleting(undefined)}
        />
      )}
    </div>
  );
}
