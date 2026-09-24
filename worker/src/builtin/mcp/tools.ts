/**
 * Pure helpers that turn MCP tools and results into Pi's shapes: names the model sees, parameter
 * schemas Pi can validate against, and tool output cut to Pi's usual limit.
 */
import { createHash } from "node:crypto";
import { DEFAULT_MAX_BYTES, formatSize, truncateHead } from "@earendil-works/pi-coding-agent";
import type { McpServerSpec } from "../../protocol.js";

export const MCP_TOOL_PREFIX = "mcp__";

/** Shorter values (flags, ports, "true") are not worth hiding and would mangle ordinary text. */
const MIN_SECRET_CHARS = 8;

/**
 * A server's header and environment values, for exact-value redaction. For "Bearer <token>" the
 * token alone is listed too, since an error may quote just that.
 */
export function secretValues(spec: Pick<McpServerSpec, "headers" | "env">): string[] {
  const values: string[] = [];
  for (const value of [...Object.values(spec.headers ?? {}), ...Object.values(spec.env ?? {})]) {
    values.push(value);
    const token = /^\S+\s+(\S+)$/.exec(value.trim())?.[1];
    if (token) values.push(token);
  }
  return values.filter((value) => value.length >= MIN_SECRET_CHARS);
}

/** `text` with every value in `secrets` replaced, longest first so no fragment survives. */
export function redactValues(text: string, secrets: string[]): string {
  let result = text;
  for (const secret of [...secrets].sort((left, right) => right.length - left.length)) {
    result = result.split(secret).join("[credential redacted]");
  }
  return result;
}

/** Providers cap tool names; 64 characters of `[A-Za-z0-9_-]` is the strictest common limit. */
const MAX_TOOL_NAME = 64;

/**
 * `mcp__<server slug>__<tool>`. A tool name that needs changing to fit the allowed characters or
 * the length limit gets a short hash of the original, so two such tools can never collide.
 */
export function mcpToolName(slug: string, tool: string): string {
  const prefix = `${MCP_TOOL_PREFIX}${slug}__`;
  const clean = tool.replace(/[^A-Za-z0-9_-]/g, "_");
  if (clean === tool && prefix.length + clean.length <= MAX_TOOL_NAME) return prefix + clean;
  const suffix = `_${createHash("sha256").update(tool).digest("hex").slice(0, 6)}`;
  return prefix + clean.slice(0, Math.max(0, MAX_TOOL_NAME - prefix.length - suffix.length)) + suffix;
}

/**
 * The tool's input schema as Pi's `parameters`. Pi validates plain JSON Schema, which MCP input
 * schemas are; the dialect marker is dropped and the root is always an object.
 */
export function toolParameters(schema: unknown): Record<string, unknown> {
  const parameters: Record<string, unknown> =
    schema && typeof schema === "object" && !Array.isArray(schema) ? { ...(schema as Record<string, unknown>) } : {};
  delete parameters.$schema;
  parameters.type = "object";
  if (!parameters.properties || typeof parameters.properties !== "object" || Array.isArray(parameters.properties)) {
    parameters.properties = {};
  }
  return parameters;
}

type TextBlock = { type: "text"; text: string };
type ImageBlock = { type: "image"; data: string; mimeType: string };

export interface ConvertedResult {
  content: Array<TextBlock | ImageBlock>;
  truncated: boolean;
}

/**
 * A `tools/call` result as tool output. Text and images pass through; links and embedded text
 * resources become text; audio and binary resources are named but left out. Text beyond Pi's
 * output limit is cut, because Pi leaves truncation to each tool. An error result throws, which
 * is how a Pi tool reports failure.
 */
export function convertToolResult(result: unknown): ConvertedResult {
  const value = (result ?? {}) as Record<string, unknown>;
  const blocks = Array.isArray(value.content) ? (value.content as Array<Record<string, unknown>>) : [];
  const texts: string[] = [];
  const images: ImageBlock[] = [];
  for (const block of blocks) {
    if (block.type === "text" && typeof block.text === "string") texts.push(block.text);
    else if (block.type === "image" && typeof block.data === "string" && typeof block.mimeType === "string") {
      images.push({ type: "image", data: block.data, mimeType: block.mimeType });
    } else if (block.type === "audio") texts.push("[Audio omitted]");
    else if (block.type === "resource_link") {
      texts.push(`Resource: ${String(block.uri ?? "")}${typeof block.name === "string" && block.name ? ` (${block.name})` : ""}`);
    } else if (block.type === "resource") {
      const resource = (block.resource ?? {}) as Record<string, unknown>;
      texts.push(typeof resource.text === "string" ? resource.text : `[Binary resource omitted: ${String(resource.uri ?? "")}]`);
    }
  }
  if (!blocks.length && value.structuredContent !== undefined) texts.push(JSON.stringify(value.structuredContent, null, 2));

  const text = texts.join("\n\n");
  if (value.isError === true) throw new Error(text.trim() || "The MCP tool reported an error.");

  const cut = truncateHead(text);
  // truncateHead keeps whole lines; one enormous line comes back empty, so fall back to a hard cut.
  const kept = cut.firstLineExceedsLimit ? text.slice(0, DEFAULT_MAX_BYTES / 4) : cut.content;
  const truncated = kept.length < text.length;
  const output = truncated ? `${kept}\n\n[Output truncated to ${formatSize(DEFAULT_MAX_BYTES)}.]` : kept;
  const content: Array<TextBlock | ImageBlock> = [];
  if (output || !images.length) content.push({ type: "text", text: output || "(no output)" });
  content.push(...images);
  return { content, truncated };
}
