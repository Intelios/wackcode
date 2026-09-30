import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { readSavedSession } from "./saved-session.js";

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

  it("uses Pi's compaction and context-edit projection", async () => {
    const sessionFile = await fixture([
      header, user("old", null, "Summarized"), user("kept", "old", "Original"),
      { type: "compaction", id: "compact", parentId: "kept", summary: "Earlier work", firstKeptEntryId: "kept", tokensBefore: 10, timestamp: header.timestamp },
      { type: "context_edit", id: "edit", parentId: "compact", targetId: "kept", replacement: { content: "Changed in context" } },
      user("new", "edit", "Continue")
    ]);
    const history = await readSavedSession({ ...options, sessionFile });
    expect(history.messages.flatMap((message) => message.blocks.map((block) => block.text))).toEqual(["Changed in context", "Continue"]);
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
    expect(JSON.parse(history).messages[0].blocks[0].text).toBe("Available offline");
  });

  it("reports missing/damaged sessions instead of showing an empty history", async () => {
    await expect(readSavedSession({ ...options, sessionFile: "/missing/session.jsonl" })).rejects.toThrow();
    await expect(readSavedSession({ ...options, sessionFile: await fixture([user("u", null, "No header")]) })).rejects.toThrow(/header/);
    await expect(readSavedSession({ ...options, sessionFile: await fixture([header, user("u", "u", "Cycle")]) })).rejects.toThrow(/circular/);
  });
});
