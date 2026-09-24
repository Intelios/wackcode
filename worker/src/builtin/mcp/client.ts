/**
 * One connection to one MCP server, over whichever transport the user chose. Shared by the chat
 * worker and by Settings' "Test connection" probe (`mcp-probe.ts`), so both reach a server the
 * same way and fail with the same messages.
 *
 * - stdio servers are ordinary child processes of this one (never detached), so they sit in the
 *   worker's process group and the host's `killpg` ends them with the worker. They get the SDK's
 *   minimal environment (HOME, PATH, SHELL, …) plus the variables the user set for them — not
 *   the worker's own environment. Their stderr is kept, briefly, for error messages only: it
 *   must never reach this process's stdout, which carries the host protocol.
 * - http and sse servers get the user's headers on every request. Redirects are followed only
 *   within the configured origin, so a header value (often a token) never leaves it.
 */
import { Client, SSEClientTransport, StreamableHTTPClientTransport, type Tool } from "@modelcontextprotocol/client";
import { StdioClientTransport } from "@modelcontextprotocol/client/stdio";
import type { McpServerSpec } from "../../protocol.js";

const STDERR_TAIL_CHARS = 4_000;
const MAX_REDIRECTS = 5;
const CLIENT_INFO = { name: "WackCode", version: "0.1.0" };

export interface McpConnection {
  /** The server's tools as of the last listing. */
  tools(): Tool[];
  callTool(name: string, args: Record<string, unknown>, signal: AbortSignal | undefined): Promise<unknown>;
  close(): Promise<void>;
}

export interface ConnectOptions {
  /** stdio servers start here: the chat's workspace. */
  cwd?: string;
  signal?: AbortSignal;
  /** The server said its tool list changed; `tools()` already has the new list. */
  onToolsChanged?: () => void;
  /** The connection ended without `close()`: the process exited or the stream dropped. */
  onClosed?: (reason: string) => void;
}

/** Start or reach the server, complete the MCP handshake and list its tools, within `timeoutMs`. */
export async function connectServer(spec: McpServerSpec, options: ConnectOptions = {}): Promise<McpConnection> {
  let tools: Tool[] = [];
  let closing = false;
  let stderrTail = "";

  const client = new Client(CLIENT_INFO, {
    listChanged: {
      tools: {
        onChanged: (error, next) => {
          if (error || !next) return;
          tools = next;
          options.onToolsChanged?.();
        },
      },
    },
  });

  let transport: StdioClientTransport | StreamableHTTPClientTransport | SSEClientTransport;
  if (spec.transport === "stdio") {
    if (!spec.command) throw new Error("No command is set.");
    const stdio = new StdioClientTransport({
      command: spec.command,
      args: spec.args ?? [],
      env: spec.env ?? {},
      cwd: options.cwd,
      stderr: "pipe",
    });
    stdio.stderr?.on("data", (chunk: Buffer | string) => {
      stderrTail = (stderrTail + chunk.toString()).slice(-STDERR_TAIL_CHARS);
    });
    transport = stdio;
  } else {
    if (!spec.url) throw new Error("No URL is set.");
    const url = new URL(spec.url);
    const requestInit: RequestInit = { headers: spec.headers ?? {} };
    const fetch = sameOriginFetch(url.origin);
    transport = spec.transport === "sse"
      ? new SSEClientTransport(url, { requestInit, fetch })
      : new StreamableHTTPClientTransport(url, { requestInit, fetch });
  }

  const request = { timeout: spec.timeoutMs, signal: options.signal };
  try {
    await client.connect(transport, request);
    tools = (await client.listTools(undefined, request)).tools;
  } catch (error) {
    closing = true;
    await client.close().catch(() => undefined);
    throw new Error(describeMcpError(error, spec, stderrTail));
  }

  client.onclose = () => {
    if (!closing) options.onClosed?.(lastLine(stderrTail) ?? "The connection closed.");
  };

  return {
    tools: () => tools,
    async callTool(name, args, signal) {
      try {
        return await client.callTool({ name, arguments: args }, { timeout: spec.timeoutMs, signal });
      } catch (error) {
        throw new Error(describeMcpError(error, spec, stderrTail));
      }
    },
    async close() {
      closing = true;
      await client.close().catch(() => undefined);
    },
  };
}

/**
 * `fetch` that follows redirects itself, only within `origin`. The user's headers (often a bearer
 * token) are meant for the configured server alone, and Node's fetch keeps custom headers on a
 * cross-origin redirect.
 */
export function sameOriginFetch(origin: string) {
  return async (input: string | URL, init?: RequestInit): Promise<Response> => {
    let url = new URL(String(input));
    for (let hop = 0; ; hop += 1) {
      const response = await fetch(url, { ...init, redirect: "manual" });
      const location = response.headers.get("location");
      if (response.status < 300 || response.status >= 400 || !location) return response;
      if (hop >= MAX_REDIRECTS) throw new Error("The server redirected too many times.");
      const next = new URL(location, url);
      if (next.origin !== origin) {
        throw new Error(`The server redirected to ${next.origin}. WackCode only follows redirects within ${origin}, so your headers stay with that server.`);
      }
      url = next;
    }
  };
}

/** A plain sentence for a failed connect or call, ending with the server's last stderr line. */
export function describeMcpError(error: unknown, spec: McpServerSpec, stderrTail = ""): string {
  const value = error as { code?: unknown; status?: unknown; name?: unknown; message?: unknown; cause?: { code?: unknown } } | undefined;
  const code = value?.code ?? value?.cause?.code;
  const status = typeof value?.status === "number" ? value.status : undefined;
  let message: string;
  if (value?.name === "AbortError" || (error instanceof Error && /aborted/i.test(error.message) && !status)) {
    message = "Stopped.";
  } else if (code === "ENOENT" && spec.transport === "stdio") {
    message = `Command not found: ${spec.command}. Check the command, or give its full path.`;
  } else if (code === "EACCES" && spec.transport === "stdio") {
    message = `${spec.command} can't be run (permission denied).`;
  } else if (code === "REQUEST_TIMEOUT") {
    message = `It did not respond within ${spec.timeoutMs} ms.`;
  } else if (status === 401 || status === 403 || value?.name === "UnauthorizedError") {
    message = `The server refused the request${status ? ` (HTTP ${status})` : ""}. If it needs a token, add it as a header, e.g. Authorization: Bearer <token>.`;
  } else if (status !== undefined) {
    message = `The server answered HTTP ${status}.`;
  } else if (code === "ECONNREFUSED") {
    message = `Nothing is answering at ${spec.url}.`;
  } else if (code === "ENOTFOUND" || code === "EAI_AGAIN") {
    message = `${spec.url ? new URL(spec.url).hostname : "The server"} could not be found.`;
  } else if (code === "CONNECTION_CLOSED") {
    message = spec.transport === "stdio" ? "The server process exited." : "The server closed the connection.";
  } else {
    message = error instanceof Error ? error.message : String(error);
  }
  const detail = spec.transport === "stdio" ? lastLine(stderrTail) : undefined;
  return detail && message !== "Stopped." ? `${message} Server output: ${detail}` : message;
}

function lastLine(text: string): string | undefined {
  const line = text.trim().split("\n").at(-1)?.trim();
  return line ? line.slice(0, 500) : undefined;
}
