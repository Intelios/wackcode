import { describe, expect, it } from "vitest";
import type { NormalizedBlock, NormalizedMessage } from "../../types";
import { clusterMessages, dividerLabel, DIVIDER_GAP_MS } from "./chat-clusters";
import { activitiesOf, activityLabel, groupActivities } from "./chat-activity";
import { resolveScratchpadPath, scratchpadFiles } from "./chat-scratchpad";
import { greeting, pickIdeas, PROMPT_IDEAS } from "./chat-hero";

const T = new Date(2025, 4, 10, 14, 2).getTime();
const msg = (id: string, role: NormalizedMessage["role"], at?: number, extra: Partial<NormalizedMessage> = {}): NormalizedMessage =>
  ({ id, role, timestamp: at, blocks: [{ type: "text", text: id }], ...extra });
const call = (id: string, toolName: string, args: Record<string, unknown> = {}): NormalizedBlock =>
  ({ type: "tool-call", toolCallId: id, toolName, arguments: args });
const result = (id: string, extra: Partial<NormalizedBlock> = {}): NormalizedBlock => ({ type: "tool-result", toolCallId: id, ...extra });

describe("clusterMessages", () => {
  it("groups a turn's assistant and tool messages into one reply", () => {
    const entries = clusterMessages([msg("u1", "user", T), msg("a1", "assistant", T + 1000), msg("t1", "tool", T + 2000), msg("a2", "assistant", T + 600_000)]);
    expect(entries.map((entry) => entry.type)).toEqual(["divider", "user", "reply"]);
    const reply = entries[2];
    expect(reply.type === "reply" && reply.messages.map((message) => message.id)).toEqual(["a1", "t1", "a2"]);
  });

  it("marks cluster starts by speaker change and silence", () => {
    const entries = clusterMessages([msg("u1", "user", T), msg("u2", "user", T + 1000), msg("u3", "user", T + 4 * 60_000)]);
    const users = entries.filter((entry) => entry.type === "user");
    expect(users.map((entry) => entry.type === "user" && entry.first)).toEqual([true, false, true]);
  });

  it("adds a divider only after a long pause, and keeps the pending echo in place", () => {
    const entries = clusterMessages([msg("u1", "user", T), msg("a1", "assistant", T + 1000), msg("pending:1", "user", T + DIVIDER_GAP_MS + 5000)]);
    expect(entries.map((entry) => entry.type)).toEqual(["divider", "user", "reply", "divider", "user"]);
  });

  it("keeps compaction boundaries and system notices separate", () => {
    const entries = clusterMessages([msg("u1", "user", T), msg("c", "assistant", T + 1, { compaction: { summary: "s", tokensBefore: 1 } }), msg("s", "system", T + 2)]);
    expect(entries.map((entry) => entry.type)).toEqual(["divider", "user", "compaction", "system"]);
  });
});

describe("dividerLabel", () => {
  it("says today, yesterday or a date", () => {
    expect(dividerLabel(T, T + 1000)).toMatch(/^Today · /);
    expect(dividerLabel(T, T + 24 * 3600_000)).toMatch(/^Yesterday · /);
    expect(dividerLabel(T, T + 5 * 24 * 3600_000)).not.toMatch(/^(Today|Yesterday)/);
  });
});

describe("activities", () => {
  it("speaks in plain words, present while live and past once done", () => {
    expect(activityLabel(call("1", "web_fetch", { url: "https://www.example.com/a?b" }), true).label).toBe("Looking up example.com/a");
    expect(activityLabel(call("1", "write", { path: "notes/plan.md" }), false).label).toBe("Jotted down plan.md");
    expect(activityLabel(call("1", "mcp__fs__read_thing"), false).label).toBe("Used read thing (fs)");
    expect(activityLabel(call("1", "memory_save", { title: "Likes tea" }), false).label).toBe("Remembered “Likes tea”");
  });

  it("derives status from results and the reply's liveness", () => {
    const results = new Map([["a", result("a")], ["b", result("b", { isError: true })]]);
    const list = activitiesOf([call("a", "read"), call("b", "read"), call("c", "read")], results, true);
    expect(list.map((item) => item.status)).toEqual(["done", "failed", "live"]);
    expect(activitiesOf([call("c", "read")], results, false)[0].status).toBe("done");
  });

  it("folds settled runs of three or more but leaves live, failed and screenshots out", () => {
    const results = new Map(["a", "b", "c", "e"].map((id) => [id, result(id)]));
    results.set("s", result("s", { images: [{ imageId: "i" }] }));
    const groups = groupActivities(activitiesOf(["a", "b", "c", "s", "e", "live"].map((id) => call(id, "read")), results, true));
    expect(groups.map((group) => group.type === "fold" ? group.label : group.activity.key)).toEqual(["Did 3 things", "s", "e", "live"]);
  });
});

describe("scratchpad files", () => {
  const pad = "/Users/me/Library/wackcode/chats/abc";
  it("resolves inside the scratchpad only", () => {
    expect(resolveScratchpadPath("notes.md", pad)).toBe(`${pad}/notes.md`);
    expect(resolveScratchpadPath("./a/../b/c.md", pad)).toBe(`${pad}/b/c.md`);
    expect(resolveScratchpadPath(`${pad}/x.txt`, pad)).toBe(`${pad}/x.txt`);
    expect(resolveScratchpadPath("../escape.md", pad)).toBeUndefined();
    expect(resolveScratchpadPath("/etc/hosts", pad)).toBeUndefined();
    expect(resolveScratchpadPath("~/x", pad)).toBeUndefined();
    expect(resolveScratchpadPath(".", pad)).toBeUndefined();
  });

  it("lists each successful write or edit once", () => {
    const results = new Map([["1", result("1")], ["2", result("2")], ["3", result("3", { isError: true })], ["4", result("4")]]);
    const files = scratchpadFiles([
      call("1", "write", { path: "notes.md" }), call("2", "edit", { path: `${pad}/notes.md` }),
      call("3", "write", { path: "bad.md" }), call("4", "write", { path: "drafts/a.txt" }), call("5", "read", { path: "x" })
    ], results, pad);
    expect(files).toEqual([
      { path: `${pad}/notes.md`, name: "notes.md", folder: "" },
      { path: `${pad}/drafts/a.txt`, name: "a.txt", folder: "drafts" }
    ]);
  });
});

describe("hero", () => {
  it("greets by the hour", () => {
    expect([3, 9, 13, 20].map(greeting)).toEqual(["Up late", "Good morning", "Good afternoon", "Good evening"]);
  });

  it("picks distinct ideas deterministically", () => {
    const one = pickIdeas(42);
    expect(one).toHaveLength(3);
    expect(new Set(one.map((idea) => idea.label)).size).toBe(3);
    expect(pickIdeas(42)).toEqual(one);
    expect(pickIdeas(7, 50)).toHaveLength(PROMPT_IDEAS.length);
  });
});
