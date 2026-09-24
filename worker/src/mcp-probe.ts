/**
 * Settings' "Test connection": a short-lived process the host starts for one MCP server. It
 * reads `{ server }` (one JSON line, secrets included, exactly as a chat worker receives them)
 * on stdin, connects, lists the server's tools, prints one `McpProbeResult` line and exits.
 *
 * It never loads Pi and never holds a provider key. The host runs it with the same environment
 * a chat worker gets and kills it at the server's timeout plus a margin.
 */
import { connectServer } from "./builtin/mcp/client.js";
import { redactValues, secretValues } from "./builtin/mcp/tools.js";
import { JsonLineDecoder } from "./framing.js";
import type { McpProbeResult, McpServerSpec } from "./protocol.js";

const MAX_DESCRIPTION_CHARS = 1_000;

function finish(result: McpProbeResult): never {
  process.stdout.write(`${JSON.stringify(result)}\n`);
  process.exit(0);
}

async function probe(spec: McpServerSpec): Promise<never> {
  const secrets = secretValues(spec);
  try {
    const connection = await connectServer(spec, { cwd: process.cwd() });
    const tools = connection.tools().map((tool) => ({
      name: tool.name,
      description: redactValues((tool.description ?? "").slice(0, MAX_DESCRIPTION_CHARS), secrets),
      readOnly: tool.annotations?.readOnlyHint === true,
    }));
    await connection.close();
    finish({ ok: true, tools });
  } catch (error) {
    finish({ ok: false, error: redactValues(error instanceof Error ? error.message : String(error), secrets) });
  }
}

const decoder = new JsonLineDecoder();
let started = false;
process.stdin.on("data", (chunk: Buffer) => {
  const [line] = decoder.push(chunk);
  if (line === undefined || started) return;
  started = true;
  process.stdin.pause();
  let spec: McpServerSpec;
  try {
    spec = (JSON.parse(line) as { server: McpServerSpec }).server;
  } catch {
    finish({ ok: false, error: "The desktop app sent an invalid request." });
  }
  void probe(spec);
});
process.stdin.on("end", () => {
  if (!started) finish({ ok: false, error: "The desktop app sent no server to test." });
});
