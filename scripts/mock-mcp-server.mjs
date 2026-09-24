// A deterministic MCP server for tests and manual checks. Plain JSON-RPC, no dependencies.
//
//   node scripts/mock-mcp-server.mjs                  stdio (what an app "stdio" server runs)
//   node scripts/mock-mcp-server.mjs --http 43128     Streamable HTTP at /mcp, legacy SSE at /sse (0: any port)
//   ... --require-auth <token>                        HTTP only: 401 unless "Authorization: Bearer <token>"
//
// Tools: echo (text back), peek (marked read-only), whoami (the Authorization header it saw,
// $MOCK_MCP_SECRET and its working directory), fail (an error result), and hang (never answers,
// for timeout and Stop tests).
import { createServer } from "node:http";
import { randomUUID } from "node:crypto";

const args = process.argv.slice(2);
const option = (name) => {
  const index = args.indexOf(name);
  return index === -1 ? undefined : args[index + 1];
};
const httpPort = option("--http");
const requiredToken = option("--require-auth");

const TOOLS = [
  {
    name: "echo",
    description: "Return the given text unchanged.",
    inputSchema: { type: "object", properties: { text: { type: "string", description: "Text to echo." } }, required: ["text"] },
  },
  {
    name: "peek",
    description: "Read a fixed value without changing anything.",
    inputSchema: { type: "object", properties: {} },
    annotations: { readOnlyHint: true },
  },
  {
    name: "whoami",
    description: "Report the Authorization header, the MOCK_MCP_SECRET variable and the working directory the server sees.",
    inputSchema: { type: "object", properties: {} },
    annotations: { readOnlyHint: true },
  },
  { name: "fail", description: "Always return an error result.", inputSchema: { type: "object", properties: {} } },
  { name: "hang", description: "Never answer.", inputSchema: { type: "object", properties: {} } },
];

/** Resolves to a JSON-RPC response, `null` for a notification, or never (hang). */
function handle(message, context) {
  const { id, method, params } = message;
  if (id === undefined || id === null) return Promise.resolve(null);
  const result = (value) => Promise.resolve({ jsonrpc: "2.0", id, result: value });
  switch (method) {
    case "initialize":
      return result({
        protocolVersion: params?.protocolVersion ?? "2025-06-18",
        capabilities: { tools: { listChanged: false } },
        serverInfo: { name: "wackcode-mock", version: "1.0.0" },
      });
    case "ping":
      return result({});
    case "tools/list":
      return result({ tools: TOOLS });
    case "tools/call": {
      const name = params?.name;
      const input = params?.arguments ?? {};
      if (name === "echo") return result({ content: [{ type: "text", text: String(input.text ?? "") }] });
      if (name === "peek") return result({ content: [{ type: "text", text: "peeked" }] });
      if (name === "whoami") {
        const report = { authorization: context.authorization ?? null, secret: process.env.MOCK_MCP_SECRET ?? null, cwd: process.cwd() };
        return result({ content: [{ type: "text", text: JSON.stringify(report) }] });
      }
      if (name === "fail") return result({ content: [{ type: "text", text: "The mock tool failed on purpose." }], isError: true });
      if (name === "hang") return new Promise(() => {});
      return Promise.resolve({ jsonrpc: "2.0", id, error: { code: -32602, message: `Unknown tool: ${name}` } });
    }
    default:
      return Promise.resolve({ jsonrpc: "2.0", id, error: { code: -32601, message: `Method not found: ${method}` } });
  }
}

if (!httpPort) {
  let buffer = "";
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (chunk) => {
    buffer += chunk;
    let newline;
    while ((newline = buffer.indexOf("\n")) !== -1) {
      const line = buffer.slice(0, newline).trim();
      buffer = buffer.slice(newline + 1);
      if (!line) continue;
      void handle(JSON.parse(line), {}).then((reply) => {
        if (reply) process.stdout.write(`${JSON.stringify(reply)}\n`);
      });
    }
  });
  process.stdin.on("end", () => process.exit(0));
  process.stderr.write("wackcode mock MCP server ready on stdio\n");
} else {
  /** Legacy SSE sessions: id -> the open event stream. */
  const streams = new Map();
  const readBody = async (request) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(Buffer.from(chunk));
    return JSON.parse(Buffer.concat(chunks).toString("utf8") || "null");
  };
  const server = createServer(async (request, response) => {
    const url = new URL(request.url ?? "/", "http://localhost");
    const authorization = request.headers.authorization;
    if (requiredToken && authorization !== `Bearer ${requiredToken}`) {
      response.writeHead(401, { "content-type": "application/json" }).end(JSON.stringify({ error: "unauthorized" }));
      return;
    }
    if (url.pathname === "/mcp") {
      if (request.method !== "POST") {
        response.writeHead(405).end();
        return;
      }
      const message = await readBody(request);
      const reply = await handle(message, { authorization });
      if (!reply) {
        response.writeHead(202).end();
        return;
      }
      response.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(reply));
      return;
    }
    if (url.pathname === "/sse" && request.method === "GET") {
      const id = randomUUID();
      response.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache", connection: "keep-alive" });
      response.write(`event: endpoint\ndata: /messages?sessionId=${id}\n\n`);
      streams.set(id, response);
      request.on("close", () => streams.delete(id));
      return;
    }
    if (url.pathname === "/messages" && request.method === "POST") {
      const stream = streams.get(url.searchParams.get("sessionId") ?? "");
      if (!stream) {
        response.writeHead(404).end();
        return;
      }
      const message = await readBody(request);
      response.writeHead(202).end();
      const reply = await handle(message, { authorization });
      if (reply) stream.write(`event: message\ndata: ${JSON.stringify(reply)}\n\n`);
      return;
    }
    response.writeHead(404).end();
  });
  server.listen(Number(httpPort), "127.0.0.1", () => {
    const { port } = server.address();
    process.stdout.write(`wackcode mock MCP server: http://127.0.0.1:${port}/mcp (Streamable HTTP), /sse (SSE)\n`);
  });
}
