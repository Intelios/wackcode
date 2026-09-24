import { spawn } from "node:child_process";
import { createServer, type Server } from "node:http";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { describeMcpError, sameOriginFetch } from "./builtin/mcp/client.js";
import { convertToolResult, mcpToolName, redactValues, secretValues, toolParameters } from "./builtin/mcp/tools.js";
import type { McpProbeResult, McpServerSpec } from "./protocol.js";

const MOCK_MCP_SERVER = resolve("../scripts/mock-mcp-server.mjs");

const servers: Server[] = [];
afterEach(async () => {
  await Promise.all(servers.splice(0).map((server) => new Promise((done) => server.close(done))));
});

function spec(overrides: Partial<McpServerSpec> = {}): McpServerSpec {
  return { id: "mcp-1", name: "Mock", slug: "mock", transport: "stdio", timeoutMs: 5_000, command: "node", disabledTools: [], ...overrides };
}

describe("MCP tool names", () => {
  it("prefixes the server slug and keeps valid names as they are", () => {
    expect(mcpToolName("github", "create_issue")).toBe("mcp__github__create_issue");
    expect(mcpToolName("github", "get-file")).toBe("mcp__github__get-file");
  });

  it("replaces other characters, adding a hash so similar names can't collide", () => {
    const dotted = mcpToolName("fs", "read.file");
    const underscored = mcpToolName("fs", "read_file");
    expect(dotted).toMatch(/^mcp__fs__read_file_[0-9a-f]{6}$/);
    expect(dotted).not.toBe(underscored);
  });

  it("fits long names into 64 characters, keeping them distinct", () => {
    const first = mcpToolName("a_very_long_server_n", "retrieve_all_of_the_pull_request_review_comments_for_a_repo");
    const second = mcpToolName("a_very_long_server_n", "retrieve_all_of_the_pull_request_review_comments_for_a_user");
    expect(first.length).toBeLessThanOrEqual(64);
    expect(second.length).toBeLessThanOrEqual(64);
    expect(first).not.toBe(second);
    expect(first).toMatch(/^[A-Za-z0-9_-]+$/);
  });
});

describe("MCP tool parameters", () => {
  it("keeps the schema, drops the dialect marker and always has an object root", () => {
    expect(toolParameters({ $schema: "https://json-schema.org/draft/2020-12/schema", type: "object", properties: { q: { type: "string" } }, required: ["q"] }))
      .toEqual({ type: "object", properties: { q: { type: "string" } }, required: ["q"] });
    expect(toolParameters(undefined)).toEqual({ type: "object", properties: {} });
    expect(toolParameters({ properties: [] })).toEqual({ type: "object", properties: {} });
  });
});

describe("MCP tool results", () => {
  it("passes text and images through and describes what it can't show", () => {
    const { content, truncated } = convertToolResult({
      content: [
        { type: "text", text: "Hello" },
        { type: "image", data: "aGk=", mimeType: "image/png" },
        { type: "resource_link", uri: "file:///a.txt", name: "a.txt" },
        { type: "resource", resource: { uri: "file:///b.txt", text: "B contents" } },
        { type: "resource", resource: { uri: "file:///c.bin", blob: "AAAA" } },
        { type: "audio", data: "AAAA", mimeType: "audio/wav" },
      ],
    });
    expect(truncated).toBe(false);
    expect(content[0]).toEqual({
      type: "text",
      text: "Hello\n\nResource: file:///a.txt (a.txt)\n\nB contents\n\n[Binary resource omitted: file:///c.bin]\n\n[Audio omitted]",
    });
    expect(content[1]).toEqual({ type: "image", data: "aGk=", mimeType: "image/png" });
  });

  it("shows structured content when there is no content, and says when there is nothing", () => {
    expect(convertToolResult({ content: [], structuredContent: { count: 2 } }).content[0]).toEqual({ type: "text", text: '{\n  "count": 2\n}' });
    expect(convertToolResult({ content: [] }).content).toEqual([{ type: "text", text: "(no output)" }]);
  });

  it("throws an error result, so Pi reports the call as failed", () => {
    expect(() => convertToolResult({ content: [{ type: "text", text: "Nope." }], isError: true })).toThrow("Nope.");
    expect(() => convertToolResult({ isError: true })).toThrow("The MCP tool reported an error.");
  });

  it("cuts output beyond Pi's limit and says so", () => {
    const lines = Array.from({ length: 5_000 }, (_, index) => `line ${index}`).join("\n");
    const { content, truncated } = convertToolResult({ content: [{ type: "text", text: lines }] });
    expect(truncated).toBe(true);
    const text = (content[0] as { text: string }).text;
    expect(text.length).toBeLessThan(lines.length);
    expect(text).toMatch(/\[Output truncated to .+\.\]$/);
    const single = convertToolResult({ content: [{ type: "text", text: "x".repeat(200_000) }] });
    expect(single.truncated).toBe(true);
  });
});

describe("MCP secrets", () => {
  it("lists header and environment values, and a bearer token on its own, skipping short values", () => {
    const values = secretValues({ headers: { Authorization: "Bearer abcdefgh12345", "X-Flag": "1" }, env: { TOKEN: "env-secret-value" } });
    expect(values).toEqual(["Bearer abcdefgh12345", "abcdefgh12345", "env-secret-value"]);
  });

  it("redacts every value, longest first", () => {
    const secrets = secretValues({ headers: { Authorization: "Bearer abcdefgh12345" } });
    expect(redactValues("sent Bearer abcdefgh12345 and abcdefgh12345", secrets)).toBe("sent [credential redacted] and [credential redacted]");
  });
});

describe("MCP errors", () => {
  it("turns common failures into plain sentences", () => {
    expect(describeMcpError(Object.assign(new Error("spawn x ENOENT"), { code: "ENOENT" }), spec({ command: "uvx" })))
      .toBe("Command not found: uvx. Check the command, or give its full path.");
    expect(describeMcpError(Object.assign(new Error("Request timed out"), { code: "REQUEST_TIMEOUT" }), spec({ timeoutMs: 2_500 })))
      .toBe("It did not respond within 2500 ms.");
    expect(describeMcpError(Object.assign(new Error("Error POSTing"), { status: 401 }), spec({ transport: "http", url: "https://x.test/mcp" })))
      .toContain("If it needs a token, add it as a header");
    expect(describeMcpError(new Error("boom"), spec(), "starting…\nfatal: missing API key\n"))
      .toBe("boom Server output: fatal: missing API key");
  });
});

describe("MCP redirects", () => {
  async function listen(handler: Parameters<typeof createServer>[1]): Promise<string> {
    const server = createServer(handler);
    servers.push(server);
    await new Promise<void>((done) => server.listen(0, "127.0.0.1", done));
    return `http://127.0.0.1:${(server.address() as { port: number }).port}`;
  }

  it("follows a redirect within the server's origin, keeping the headers", async () => {
    const origin = await listen((request, response) => {
      if (request.url === "/old") response.writeHead(307, { location: "/new" }).end();
      else response.end(`auth=${request.headers.authorization}`);
    });
    const response = await sameOriginFetch(origin)(`${origin}/old`, { headers: { Authorization: "Bearer abc" } });
    expect(await response.text()).toBe("auth=Bearer abc");
  });

  it("refuses a redirect to another origin, so headers never leave the configured server", async () => {
    let reached = false;
    const elsewhere = await listen((_request, response) => { reached = true; response.end("stolen"); });
    const origin = await listen((_request, response) => response.writeHead(302, { location: `${elsewhere}/collect` }).end());
    await expect(sameOriginFetch(origin)(`${origin}/mcp`, { headers: { Authorization: "Bearer abc" } })).rejects.toThrow(/only follows redirects within/);
    expect(reached).toBe(false);
  });
});

describe("MCP connection test (mcp-probe.js)", () => {
  function probe(server: McpServerSpec): Promise<McpProbeResult> {
    return new Promise((done, fail) => {
      const child = spawn(process.execPath, [resolve("dist/mcp-probe.js")], { stdio: ["pipe", "pipe", "ignore"] });
      let stdout = "";
      child.stdout.on("data", (chunk: Buffer) => { stdout += chunk.toString(); });
      child.on("error", fail);
      child.on("exit", () => done(JSON.parse(stdout.trim().split("\n").at(-1) ?? "null") as McpProbeResult));
      child.stdin.end(`${JSON.stringify({ server })}\n`);
    });
  }

  it("lists a server's tools with their read-only hints", async () => {
    const result = await probe(spec({ command: process.execPath, args: [MOCK_MCP_SERVER] }));
    expect(result.ok).toBe(true);
    const tools = result.ok ? result.tools : [];
    expect(tools.map((tool) => tool.name)).toEqual(["echo", "peek", "whoami", "fail", "hang"]);
    expect(tools.find((tool) => tool.name === "peek")?.readOnly).toBe(true);
    expect(tools.find((tool) => tool.name === "echo")?.readOnly).toBe(false);
  });

  it("reports a failure without the server's secrets", async () => {
    const result = await probe(spec({ command: process.execPath, args: ["-e", "console.error('bad token secret-value-123'); process.exit(1)"], env: { TOKEN: "secret-value-123" } }));
    expect(result.ok).toBe(false);
    const error = result.ok ? "" : result.error;
    expect(error).toContain("[credential redacted]");
    expect(error).not.toContain("secret-value-123");
  });
});
