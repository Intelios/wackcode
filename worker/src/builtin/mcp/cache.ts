/**
 * An advisory tool catalogue shared by chat workers, under the app's agent directory.
 * Never cache results, configuration or credentials. A fingerprint binds schemas to the
 * exact connection (including credentials); every worker still reconnects and checks the
 * current tool before executing it. Missing, old, oversized or damaged files are cache misses.
 */
import { createHash, randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { mkdir, open, rename, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { Tool } from "@modelcontextprotocol/client";

const MAX_BYTES = 4 * 1024 * 1024;
const MAX_AGE_MS = 7 * 24 * 60 * 60_000;

export function cataloguePath(directory: string, connectionKey: string): string {
  return join(directory, `${createHash("sha256").update(connectionKey).digest("hex")}.json`);
}

export async function readCatalogue(path: string): Promise<Tool[] | undefined> {
  try {
    const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
    try {
      const stat = await file.stat();
      if (!stat.isFile() || stat.size > MAX_BYTES || Date.now() - stat.mtimeMs > MAX_AGE_MS) return;
      const buffer = Buffer.alloc(stat.size + 1);
      const { bytesRead } = await file.read(buffer, 0, buffer.length, 0);
      if (bytesRead > stat.size) return;
      const value: unknown = JSON.parse(buffer.subarray(0, bytesRead).toString("utf8"));
      if (!Array.isArray(value) || !value.every((tool) => tool && typeof tool.name === "string"
        && tool.inputSchema && typeof tool.inputSchema === "object" && !Array.isArray(tool.inputSchema))) return;
      return value as Tool[];
    } finally {
      await file.close();
    }
  } catch { return; }
}

export async function writeCatalogue(directory: string, path: string, tools: Tool[], redact: (text: string) => string): Promise<void> {
  const text = JSON.stringify(tools);
  // Even a server that echoes a credential into its schema must not persist it here.
  if (Buffer.byteLength(text) > MAX_BYTES || redact(text) !== text) return;
  const temporary = `${path}.${randomUUID()}.tmp`;
  try {
    await mkdir(directory, { recursive: true, mode: 0o700 });
    await writeFile(temporary, text, { mode: 0o600, flag: "wx" });
    await rename(temporary, path);
  } catch {
    // Optional acceleration: an unwritable cache must never break an MCP connection.
  } finally {
    await rm(temporary, { force: true }).catch(() => undefined);
  }
}
