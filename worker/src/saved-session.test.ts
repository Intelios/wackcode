import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { readSavedSession, streamSavedSession } from "./saved-session.js";
import type { SavedHistoryFrame } from "./protocol.js";

const folders: string[] = [];
afterEach(async () => { await Promise.all(folders.splice(0).map((folder) => rm(folder, { recursive: true, force: true }))); });
const options = { taskId: "offline", mode: "build" as const, thinkingLevel: "off" as const };
async function fixture(entries: unknown[], tail = "") {
  const folder = await mkdtemp(join(tmpdir(), "wackcode-history-"));
  folders.push(folder);
  const sessionFile = join(folder, "session.jsonl");
  await writeFile(sessionFile, entries.map((entry) => JSON.stringify(entry)).join("\n") + "\n" + tail);
  return sessionFile;
}
const header = { type: "session", version: 3, id: "saved-session", timestamp: "2026-09-30T10:00:00Z", cwd: "/workspace-no-longer-exists" };
const user = (id: string, parentId: string | null, text: string) => ({ type: "message", id, parentId, message: { role: "user", content: text, timestamp: 1 } });

describe("saved history without a worker", () => {
  it("opens a new chat without a session or model", async () => {
    expect(await readSavedSession({ ...options, mode: "ultraplan" })).toMatchObject({ messages: [], tree: { leafId: null }, planState: { mode: "ultraplan" } });
  });

  it("reads only the active branch, including versions, and skips an interrupted append", async () => {
    const sessionFile = await fixture([header, user("a", null, "First"), user("b", null, "Edited")], '{"type":"message"');
    const original = await readFile(sessionFile, "utf8");
    const history = await readSavedSession({ ...options, sessionFile });
    expect(history.messages).toEqual([expect.objectContaining({ id: "b", blocks: [{ type: "text", text: "Edited" }], versions: expect.objectContaining({ total: 2, previous: "a" }) })]);
    expect(history.tree?.leafId).toBe("b");
    expect(await readFile(sessionFile, "utf8")).toBe(original);
  });

  it("preserves compacted history and entry ids while honoring context edits", async () => {
    const sessionFile = await fixture([
      header, user("old", null, "Summarized"), user("kept", "old", "Original"),
      { type: "compaction", id: "compact", parentId: "kept", summary: "Earlier work", firstKeptEntryId: "kept", tokensBefore: 10, timestamp: header.timestamp },
      { type: "context_edit", id: "edit", parentId: "compact", targetId: "kept", replacement: { content: "Changed in context" } },
      user("new", "edit", "Continue")
    ]);
    const history = await readSavedSession({ ...options, sessionFile });
    expect(history.messages.flatMap((message) => message.blocks.map((block) => block.text))).toEqual(["Summarized", "Changed in context", "Continue"]);
    expect(history.messages.map((message) => message.id)).toEqual(["old", "kept", "compact", "new"]);
    expect(history.messages[2].compaction).toEqual({ summary: "Earlier work", tokensBefore: 10, estimatedTokensAfter: expect.any(Number) });
    const after = history.messages[2].compaction?.estimatedTokensAfter;
    expect((await readSavedSession({ ...options, sessionFile })).messages[2].compaction?.estimatedTokensAfter).toBe(after);
  });

  it("keeps the context meter unknown after compaction until another answer has usage", async () => {
    const sessionFile = await fixture([
      header, user("u", null, "Earlier work"),
      { type: "compaction", id: "c", parentId: "u", summary: "Summarized", firstKeptEntryId: "u", tokensBefore: 10, timestamp: header.timestamp }
    ]);
    const history = await readSavedSession({ ...options, sessionFile, contextWindow: 16_384 });
    expect(history.stats.contextUsage).toEqual({ tokens: null, contextWindow: 16_384, percent: null });
    expect(history.stats.contextBreakdown).toBeUndefined();
  });

  it("marks a tool result's image that can't be previewed as failed instead of pending", async () => {
    const result = { type: "message", id: "r", parentId: "a", message: { role: "toolResult", toolName: "mcp__design__export", toolCallId: "call", timestamp: 2,
      content: [{ type: "text", text: "Exported" }, { type: "image", data: Buffer.from("not a picture").toString("base64"), mimeType: "image/png" }] } };
    const history = await readSavedSession({ ...options, sessionFile: await fixture([header, user("a", null, "Export it"), result]) });
    const block = history.messages.flatMap((message) => message.blocks).find((candidate) => candidate.type === "tool-result");
    expect(block).toMatchObject({ text: "Exported", toolCallId: "call", images: [{ imageId: expect.any(String), failed: true }] });
    expect(block?.images?.[0]).not.toHaveProperty("thumbnail");
  });

  it("restores ready plans, todos and a paused goal without running extensions", async () => {
    const sessionFile = await fixture([
      header,
      { type: "custom", id: "plan", parentId: null, customType: "wackcode-plan-state", data: { version: 1, enabled: true, ultra: true, plan: "Do the work" } },
      { type: "message", id: "todo", parentId: "plan", message: { role: "toolResult", toolName: "todo", toolCallId: "t", content: [], details: { version: 1, tasks: [{ id: 1, subject: "Read", status: "completed" }], nextId: 2 } } },
      { type: "custom", id: "goal", parentId: "todo", customType: "wackcode-goal", data: { version: 1, objective: "Finish", phase: "active", iteration: 2, maxIterations: 5, noProgress: 0 } }
    ]);
    const history = await readSavedSession({ ...options, sessionFile });
    expect(history.planState).toEqual({ mode: "ultraplan", phase: "ready", plan: "Do the work" });
    expect(history.todoState?.tasks).toHaveLength(1);
    expect(history.goalState).toMatchObject({ phase: "paused", iteration: 2 });
  });

  it("runs the deployed entry with an empty environment and no model/workspace", async () => {
    const sessionFile = await fixture([header, user("u", null, "Available offline")]);
    const history = await new Promise<string>((resolve, reject) => {
      const reader = spawn(process.execPath, [new URL("../dist/session-reader.js", import.meta.url).pathname], { env: {}, stdio: ["pipe", "pipe", "pipe"] });
      let output = "", error = "";
      reader.stdout.on("data", (chunk) => { output += chunk; });
      reader.stderr.on("data", (chunk) => { error += chunk; });
      reader.on("error", reject);
      reader.on("close", (code) => code === 0 ? resolve(output) : reject(new Error(error)));
      reader.stdin.end(JSON.stringify({ ...options, sessionFile }));
    });
    const frames = history.trim().split("\n").map((line) => JSON.parse(line) as SavedHistoryFrame);
    expect(frames[0]).toMatchObject({ type: "snapshot", snapshot: { messages: [{ blocks: [{ text: "Available offline" }] }] } });
    expect(frames.at(-1)).toEqual({ type: "history_previews", previews: [], done: true });
  });

  it("reports missing/damaged sessions instead of showing an empty history", async () => {
    await expect(readSavedSession({ ...options, sessionFile: "/missing/session.jsonl" })).rejects.toThrow();
    await expect(readSavedSession({ ...options, sessionFile: await fixture([user("u", null, "No header")]) })).rejects.toThrow(/header/);
    await expect(readSavedSession({ ...options, sessionFile: await fixture([header, user("u", "u", "Cycle")]) })).rejects.toThrow(/circular/);
  });
});

describe("streamed image previews", () => {
  const image = (text: string) => ({ type: "image", mimeType: "image/png", data: Buffer.from(text).toString("base64") });
  const attached = (id: string, parentId: string | null, images: ReturnType<typeof image>[]) => ({
    ...user(id, parentId, ""), message: { role: "user", timestamp: 1, content: [{ type: "text", text: `Text ${id}` }, ...images] }
  });

  it("emits immutable text and pending identities before a blocked resize, then prepares newest first", async () => {
    const sessionFile = await fixture([header, attached("old", null, [image("old")]), attached("new", "old", [image("new")])]);
    const original = await readFile(sessionFile, "utf8");
    let release!: () => void;
    const blocked = new Promise<void>((resolve) => { release = resolve; });
    const calls: string[] = [];
    const frames: SavedHistoryFrame[] = [];
    let initial!: () => void;
    const initialFrame = new Promise<void>((resolve) => { initial = resolve; });
    const complete = streamSavedSession({ ...options, sessionFile }, (frame) => {
      frames.push(frame);
      if (frame.type === "snapshot") initial();
    }, { resizeImage: async (bytes) => { calls.push(Buffer.from(bytes).toString()); await blocked; return null; } });
    await initialFrame;
    const first = frames[0];
    expect(first.type).toBe("snapshot");
    if (first.type !== "snapshot") throw new Error("Expected initial snapshot");
    expect(first.snapshot.messages.map((message) => message.blocks)).toEqual([
      [{ type: "text", text: "Text old" }, { type: "image", mimeType: "image/png", imageId: "image-1" }],
      [{ type: "text", text: "Text new" }, { type: "image", mimeType: "image/png", imageId: "image-2" }]
    ]);
    await vi.waitFor(() => expect(calls).toEqual(["new"]));
    expect(frames).toHaveLength(1);
    expect(JSON.stringify(first)).not.toContain(image("new").data);
    release();
    await complete;
    expect(calls).toEqual(["new", "old"]);
    expect(frames[1]).toEqual({ type: "history_previews", previews: [
      { imageId: "image-2", failed: true }, { imageId: "image-1", failed: true }
    ] });
    // A consumer may retain the initial snapshot while the next frames arrive.
    expect(first.snapshot.messages[1].blocks[1]).not.toHaveProperty("failed");
    expect(frames.at(-1)).toEqual({ type: "history_previews", previews: [], done: true });
    expect(await readFile(sessionFile, "utf8")).toBe(original);
  });

  it("flushes completed previews while another resize is still waiting", async () => {
    const sessionFile = await fixture([header, attached("u", null, [image("slow"), image("fast")])]);
    let release!: () => void;
    const blocked = new Promise<void>((resolve) => { release = resolve; });
    const frames: SavedHistoryFrame[] = [];
    const complete = streamSavedSession({ ...options, sessionFile }, (frame) => { frames.push(frame); }, {
      resizeImage: async (bytes) => { if (Buffer.from(bytes).toString() === "slow") await blocked; return null; }
    });
    try {
      await vi.waitFor(() => expect(frames[1]).toEqual({ type: "history_previews", previews: [{ imageId: "image-2", failed: true }] }), { timeout: 1500 });
      expect(frames.some((frame) => frame.type === "history_previews" && frame.done)).toBe(false);
    } finally { release(); await complete; }
  });

  it("caps batches at eight and settles a throwing decoder without losing text", async () => {
    const sessionFile = await fixture([header, attached("u", null, Array.from({ length: 18 }, (_, index) => image(String(index))))]);
    const frames: SavedHistoryFrame[] = [];
    await streamSavedSession({ ...options, sessionFile }, (frame) => { frames.push(frame); }, {
      resizeImage: async () => { throw new Error("Bad image"); }
    });
    const updates = frames.filter((frame) => frame.type === "history_previews" && !frame.done);
    expect(updates.map((frame) => frame.previews.length)).toEqual([8, 8, 2]);
    expect(updates.flatMap((frame) => frame.previews).every((preview) => preview.failed)).toBe(true);
    expect(frames[0]).toMatchObject({ type: "snapshot", snapshot: { messages: [{ blocks: [expect.objectContaining({ text: "Text u" }), ...Array.from({ length: 18 }, () => expect.any(Object))] }] } });
  });

  it("keeps user/tool image identities and their separate thumbnail limits", async () => {
    const sessionFile = await fixture([header, attached("u", null, [image("attached")]), {
      type: "message", id: "result", parentId: "u", message: { role: "toolResult", toolName: "read", toolCallId: "call", timestamp: 2,
        content: [{ type: "text", text: "Read picture" }, image("tool")] }
    }]);
    const frames: SavedHistoryFrame[] = [];
    const sizes: number[] = [];
    await streamSavedSession({ ...options, sessionFile }, (frame) => { frames.push(frame); }, {
      resizeImage: async (_bytes, _mime, limits) => {
        sizes.push(limits!.maxWidth!);
        return { data: "preview", mimeType: "image/jpeg", width: 1, height: 1, originalWidth: 1, originalHeight: 1, wasResized: true };
      }
    });
    expect(sizes).toEqual([480, 512]);
    expect(frames[0]).toMatchObject({ type: "snapshot", snapshot: { messages: [
      { blocks: [expect.any(Object), { imageId: "image-1", type: "image", mimeType: "image/png" }] },
      { blocks: [{ images: [{ imageId: "image-2" }], toolCallId: "call" }] }
    ] } });
    expect(frames[1]).toEqual({ type: "history_previews", previews: [
      { imageId: "image-2", thumbnail: "data:image/jpeg;base64,preview" },
      { imageId: "image-1", thumbnail: "data:image/jpeg;base64,preview" }
    ] });
  });

  it("terminates the deployed reader when the host closes its output pipe", async () => {
    const sessionFile = await fixture([header, attached("u", null, Array.from({ length: 40 }, () => image("bad image")))]);
    const reader = spawn(process.execPath, [new URL("../dist/session-reader.js", import.meta.url).pathname], { env: {}, stdio: ["pipe", "pipe", "ignore"] });
    const closed = new Promise<number | null>((resolve, reject) => { reader.on("error", reject); reader.on("close", resolve); });
    const deadline = setTimeout(() => reader.kill("SIGKILL"), 5000);
    reader.stdin.end(JSON.stringify({ ...options, sessionFile }));
    try {
      const first = await new Promise<string>((resolve, reject) => {
        let text = "";
        reader.stdout.on("data", (chunk) => { text += chunk; if (text.includes("\n")) resolve(text.split("\n")[0]); });
        reader.on("error", reject);
        reader.on("close", () => reject(new Error("Reader closed before its snapshot")));
      });
      expect(JSON.parse(first).type).toBe("snapshot");
      reader.stdout.destroy();
      expect(await closed).toBe(0);
    } finally { clearTimeout(deadline); reader.kill("SIGKILL"); }
  });
});
