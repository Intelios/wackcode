/**
 * Built-in MCP client: the user's MCP servers (Settings › MCP servers) as tools named
 * `mcp__<server>__<tool>`.
 *
 * - Servers connect lazily, when a run starts (`prepare`), never when a chat merely opens: every
 *   chat the user clicks gets a worker, and each would otherwise start every stdio server. A
 *   connection then lasts as long as the worker.
 * - Tool sets change only between runs: `prepare` runs before the prompt, and `configure`
 *   (from `set_mcp`) waits in the worker's command queue like every other setting.
 * - Pi can't unregister a tool, so a tool whose server is off, gone, failed, or that the user
 *   switched off stays registered but inactive (`inactiveTools`), and refuses if called.
 * - A server that fails to connect says so once in the chat and isn't retried for a few
 *   minutes, so a broken server can't add its timeout to every message.
 * - Connections live out here, not in the factory, because `/init` reloads the session and
 *   re-runs every factory; the new registry gets the same tools straight away.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { Tool } from "@modelcontextprotocol/client";
import type { McpServerSpec } from "../../protocol.js";
import type { BuiltinHost } from "../host.js";
import { connectServer, type McpConnection } from "./client.js";
import { convertToolResult, mcpToolName, secretValues, toolParameters } from "./tools.js";

const RETRY_FAILED_AFTER_MS = 5 * 60_000;
const MAX_DESCRIPTION_CHARS = 2_000;

type ServerStatus =
  /** Not connected yet, or the connection dropped: connects at the next run. */
  | { kind: "idle" }
  | { kind: "connecting"; abort: AbortController }
  | { kind: "connected"; connection: McpConnection }
  | { kind: "failed"; error: string; at: number };

interface Server {
  spec: McpServerSpec;
  /** What the connection itself depends on. A change reconnects; a new name or timeout doesn't. */
  key: string;
  status: ServerStatus;
}

interface Registration {
  serverId: string;
  /** The server's own name for the tool. */
  tool: string;
  /** Re-registering an unchanged tool would still rebuild the system prompt, costing the prompt cache. */
  signature: string;
}

export interface McpToolDetails {
  server: string;
  tool: string;
  truncated: boolean;
}

export interface McpController {
  /** Apply the enabled servers from Settings. Removed, disabled or changed servers disconnect. */
  configure(specs: McpServerSpec[]): void;
  /**
   * Connect every server that isn't connected yet, then register their tools. `onConnecting`
   * runs first when anything actually has to connect. Resolves to whether the set of usable
   * tools may have changed, so the caller re-applies the active tools.
   */
  prepare(signal: AbortSignal, onConnecting?: () => void): Promise<boolean>;
  /** Registered MCP tools the model must not be offered right now. */
  inactiveTools(): string[];
  /** The id of the server a registered tool belongs to. */
  serverOf(toolName: string): string | undefined;
  /** The server marks this tool read-only, so Plan mode may let it through. */
  isReadOnlyTool(toolName: string): boolean;
  /** Header and environment values, for redaction. */
  credentials(): string[];
  closeAll(): Promise<void>;
}

export function createMcpExtension(host: BuiltinHost): { factory: (pi: ExtensionAPI) => void; controller: McpController } {
  const servers = new Map<string, Server>();
  const registrations = new Map<string, Registration>();
  let api: ExtensionAPI | undefined;
  let liveKey = "";

  const connectionKey = (spec: McpServerSpec) =>
    JSON.stringify([spec.transport, spec.command ?? "", spec.args ?? [], spec.env ?? {}, spec.url ?? "", spec.headers ?? {}]);

  /** Tools the model may use right now, by registered name. */
  function liveTools(): Map<string, { server: Server; tool: Tool }> {
    const live = new Map<string, { server: Server; tool: Tool }>();
    for (const server of servers.values()) {
      if (server.status.kind !== "connected") continue;
      for (const tool of server.status.connection.tools()) {
        if (server.spec.disabledTools.includes(tool.name)) continue;
        live.set(mcpToolName(server.spec.slug, tool.name), { server, tool });
      }
    }
    return live;
  }

  function disconnect(server: Server): void {
    if (server.status.kind === "connecting") server.status.abort.abort();
    if (server.status.kind === "connected") void server.status.connection.close();
    server.status = { kind: "idle" };
  }

  async function connect(server: Server, runSignal: AbortSignal): Promise<void> {
    const { spec } = server;
    const abort = new AbortController();
    const onRunAbort = () => abort.abort();
    runSignal.addEventListener("abort", onRunAbort, { once: true });
    server.status = { kind: "connecting", abort };
    try {
      const connection = await connectServer(spec, {
        cwd: host.workspace(),
        signal: abort.signal,
        onClosed: (reason) => {
          if (servers.get(spec.id) !== server || server.status.kind !== "connected") return;
          server.status = { kind: "idle" };
          host.notice(`MCP server "${spec.name}" stopped: ${host.redact(reason)} It reconnects with your next message.`, "warning");
        },
      });
      // Removed or changed while connecting.
      if (servers.get(spec.id) !== server || abort.signal.aborted) {
        void connection.close();
        return;
      }
      server.status = { kind: "connected", connection };
    } catch (error) {
      if (servers.get(spec.id) !== server) return;
      // Stopped by the user: not the server's fault, so the next run simply tries again.
      if (abort.signal.aborted) {
        server.status = { kind: "idle" };
        return;
      }
      const message = host.redact(error instanceof Error ? error.message : String(error));
      server.status = { kind: "failed", error: message, at: Date.now() };
      host.notice(`MCP server "${spec.name}" isn't available: ${message} Check it in Settings › MCP servers.`, "warning");
    } finally {
      runSignal.removeEventListener("abort", onRunAbort);
    }
  }

  function register(name: string, server: Server, tool: Tool): boolean {
    if (!api) return false;
    const description = `(MCP server "${server.spec.name}") ${tool.description ?? ""}`.trim().slice(0, MAX_DESCRIPTION_CHARS);
    const label = `${server.spec.name}: ${tool.title ?? tool.annotations?.title ?? tool.name}`;
    const parameters = toolParameters(tool.inputSchema);
    const signature = JSON.stringify([server.spec.id, tool.name, label, description, parameters]);
    if (registrations.get(name)?.signature === signature) return false;
    registrations.set(name, { serverId: server.spec.id, tool: tool.name, signature });
    api.registerTool({
      name,
      label,
      description,
      parameters,
      async execute(_toolCallId, params: unknown, signal) {
        const registration = registrations.get(name);
        const owner = registration && servers.get(registration.serverId);
        if (!registration || !owner || owner.spec.disabledTools.includes(registration.tool)) {
          throw new Error(`${name} is switched off in Settings › MCP servers.`);
        }
        if (owner.status.kind !== "connected") throw new Error(`MCP server "${owner.spec.name}" is not connected.`);
        const args = params && typeof params === "object" && !Array.isArray(params) ? (params as Record<string, unknown>) : {};
        try {
          const { content, truncated } = convertToolResult(await owner.status.connection.callTool(registration.tool, args, signal));
          return { content, details: { server: owner.spec.name, tool: registration.tool, truncated } satisfies McpToolDetails };
        } catch (error) {
          throw new Error(host.redact(error instanceof Error ? error.message : String(error)));
        }
      },
    });
    return true;
  }

  /** Register every live tool that's new or changed. */
  function sync(): boolean {
    let changed = false;
    for (const [name, { server, tool }] of liveTools()) changed = register(name, server, tool) || changed;
    const key = [...liveTools().keys()].sort().join("\n");
    if (key !== liveKey) changed = true;
    liveKey = key;
    return changed;
  }

  const controller: McpController = {
    configure(specs) {
      const wanted = new Map(specs.map((spec) => [spec.id, spec]));
      for (const [id, server] of servers) {
        const next = wanted.get(id);
        if (!next || connectionKey(next) !== server.key) {
          disconnect(server);
          servers.delete(id);
        }
      }
      for (const spec of specs) {
        const existing = servers.get(spec.id);
        if (existing) existing.spec = spec;
        else servers.set(spec.id, { spec, key: connectionKey(spec), status: { kind: "idle" } });
      }
      // A renamed server's tools get their new names now; nothing connects until a run.
      sync();
    },

    async prepare(signal, onConnecting) {
      const now = Date.now();
      const pending = [...servers.values()].filter(
        (server) => server.status.kind === "idle" || (server.status.kind === "failed" && now - server.status.at >= RETRY_FAILED_AFTER_MS)
      );
      if (pending.length && !signal.aborted) {
        onConnecting?.();
        await Promise.all(pending.map((server) => connect(server, signal)));
      }
      return sync();
    },

    inactiveTools() {
      const live = liveTools();
      return [...registrations.keys()].filter((name) => !live.has(name));
    },

    serverOf: (toolName) => registrations.get(toolName)?.serverId,

    isReadOnlyTool: (toolName) => liveTools().get(toolName)?.tool.annotations?.readOnlyHint === true,

    credentials: () => [...servers.values()].flatMap(({ spec }) => secretValues(spec)),

    async closeAll() {
      const closing: Promise<void>[] = [];
      for (const server of servers.values()) {
        if (server.status.kind === "connected") closing.push(server.status.connection.close());
        else if (server.status.kind === "connecting") server.status.abort.abort();
      }
      await Promise.allSettled(closing);
    },
  };

  return {
    factory(pi) {
      // A fresh registry: the chat's first load, or `/init` reloading the session.
      api = pi;
      registrations.clear();
      liveKey = "";
      sync();
    },
    controller,
  };
}
