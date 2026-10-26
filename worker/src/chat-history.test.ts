import { describe, expect, it } from "vitest";
import type { FileEntry } from "@earendil-works/pi-coding-agent";
import { chatBranch, renderChatHistory, stripTailSections } from "./chat-history.js";
import { normalizeReadChatParams, formatChatHistory, resolvedChat } from "./builtin/chat-refs.js";
import { DEFAULT_MAX_BYTES } from "@earendil-works/pi-coding-agent";

// A tiny builder for the session JSONL shape saved-session.ts also reads.
function session(entries: Array<Record<string, unknown>>): FileEntry[] {
  return [
    { type: "session", version: 1, id: "sess", timestamp: "2026-01-01T00:00:00Z", cwd: "/tmp" } as unknown as FileEntry,
    ...entries.map((entry) => entry as unknown as FileEntry),
  ];
}

function message(id: string, parentId: string | null, role: string, content: unknown, extra: Record<string, unknown> = {}) {
  return {
    type: "message", id, parentId, timestamp: "2026-01-01T00:00:00Z",
    message: { role, content, timestamp: 1, ...(role === "assistant" ? { api: "openai-completions", provider: "p", model: "m", usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 }, stopReason: "stop" } : {}), ...extra },
  };
}

describe("chatBranch", () => {
  it("follows the last entry back to the root, hiding sibling retries", () => {
    const entries = session([
      message("u1", null, "user", "first"),
      message("a1", "u1", "assistant", [{ type: "text", text: "answer one" }]),
      message("u2", "a1", "user", "second"),
      message("a2", "u2", "assistant", [{ type: "text", text: "wrong try" }]),
      message("a3", "u2", "assistant", [{ type: "text", text: "right try" }]),
    ]);
    expect(chatBranch(entries).map((entry) => entry.id)).toEqual(["u1", "a1", "u2", "a3"]);
  });

  it("refuses a circular branch instead of looping", () => {
    const entries = session([
      { type: "message", id: "a", parentId: "b", message: { role: "user", content: "x" } },
      { type: "message", id: "b", parentId: "a", message: { role: "user", content: "y" } },
    ]);
    expect(() => chatBranch(entries)).toThrow(/circular/);
  });
});

describe("renderChatHistory", () => {
  const entries = session([
    message("u1", null, "user", [{ type: "text", text: "Continue the auth work." }, { type: "image", mimeType: "image/png", data: "AAAA" }]),
    message("a1", "u1", "assistant", [
      { type: "thinking", thinking: "secret reasoning" },
      { type: "text", text: "On it." },
      { type: "toolCall", id: "t1", name: "read", arguments: { path: "src/auth.ts" } },
    ]),
    { type: "message", id: "r1", parentId: "a1", message: { role: "toolResult", toolCallId: "t1", toolName: "read", content: [{ type: "text", text: "export const login = 1;" }], timestamp: 1 } },
    { type: "compaction", id: "c1", parentId: "r1", timestamp: "2026-01-01T00:00:00Z", summary: "We built the login form." },
    message("u2", "c1", "user", "thanks"),
  ]);

  it("renders turns, tool markers and compaction, and leaves thinking out", () => {
    const text = renderChatHistory(entries, { name: "Auth", project: "WackCode", model: "Gateway · sees" });
    expect(text).toContain("# Chat: Auth");
    expect(text).toContain("Project: WackCode");
    expect(text).toContain("Model: Gateway · sees");
    expect(text).toContain("Messages: 3 (2 user, 1 assistant)");
    expect(text).toContain("## User\n\nContinue the auth work.\n[image]");
    expect(text).toContain('## Assistant\n\nOn it.\n\n[tool: read({"path":"src/auth.ts"})]');
    expect(text).toContain("## Compaction summary\n\nWe built the login form.");
    expect(text).not.toContain("secret reasoning");
    // Tool results only on request.
    expect(text).not.toContain("export const login");
  });

  it("includes truncated tool results when asked", () => {
    const long = "x".repeat(3_000);
    const withResult = session([
      message("u1", null, "user", "run it"),
      message("a1", "u1", "assistant", [{ type: "toolCall", id: "t1", name: "bash", arguments: { command: "ls" } }]),
      { type: "message", id: "r1", parentId: "a1", message: { role: "toolResult", toolCallId: "t1", toolName: "bash", content: [{ type: "text", text: long }], timestamp: 1 } },
    ]);
    const text = renderChatHistory(withResult, { name: "Run" }, { tools: true });
    expect(text).toContain("[tool result: bash]");
    expect(text).toContain("[… result truncated]");
    expect(text.length).toBeLessThan(long.length);
  });

  it("collapses generated tail sections inside a user message", () => {
    const wire = [
      "continue @\"Auth refactor\"",
      "",
      "<attached-files>",
      '<file name="notes.md" lines="1">',
      "hello",
      "</file>",
      "</attached-files>",
    ].join("\n");
    const text = renderChatHistory(session([message("u1", null, "user", wire)]), { name: "A" });
    expect(text).toContain("[attached files]");
    expect(text).not.toContain("<file name=");
    expect(text).not.toContain("hello");
  });

  it("says so for a chat with no messages", () => {
    expect(renderChatHistory(session([]), { name: "Empty" })).toContain("[This chat has no saved messages.]");
  });

  it("describes reply images without carrying another chat's image handles", () => {
    const withImages = session([
      message("u1", null, "user", "Show the result."),
      message("a1", "u1", "assistant", [{ type: "text", text: [
        "Here is the result: ![New layout](wackcode-image://entry/result/0)",
        '![Dark \\] theme](<wackcode-image://entry/result/1> "Expanded")',
        "![](wackcode-image://entry/attachment/0)",
        "[Earlier image](wackcode-image://entry/result/0)",
        "![Normal remote](https://example.com/image.png)",
      ].join("\n") }]),
      { type: "compaction", id: "c1", parentId: "a1", timestamp: "2026-01-01T00:00:00Z", summary: "Image: wackcode-image://entry/result/0" },
    ]);
    const rendered = renderChatHistory(withImages, { name: "Images" });
    expect(rendered).toContain("Here is the result: [image: New layout]");
    expect(rendered).toContain("[image: Dark ] theme]");
    expect(rendered).toContain("\n[image]\n");
    expect(rendered).toContain("![Normal remote](https://example.com/image.png)");
    expect(rendered).toContain("Image: [image]");
    expect(rendered).not.toContain("wackcode-image:");
  });
});

describe("stripTailSections", () => {
  it("replaces each tail from the outside in and keeps the words", () => {
    const text = "words\n\n<a>\n1\n</a>\n\n<b>\n2\n</b>".replace("<a>", "<referenced-chats-v1>").replace("</a>", "</referenced-chats-v1>")
      .replace("<b>", "<browser-contexts-v1>").replace("</b>", "</browser-contexts-v1>");
    expect(stripTailSections(text)).toBe("words\n\n[referenced chats]\n\n[browser context]");
    expect(stripTailSections("plain words")).toBe("plain words");
  });
});

describe("read_chat's parameters and paging", () => {
  it("normalises and refuses odd input", () => {
    expect(normalizeReadChatParams({ chat: "abc" })).toEqual({ ok: true, chat: "abc", offset: 0, tools: false });
    expect(normalizeReadChatParams({ chat: "abc", offset: 40, tools: true })).toEqual({ ok: true, chat: "abc", offset: 40, tools: true });
    expect(normalizeReadChatParams({ chat: "" }).ok).toBe(false);
    expect(normalizeReadChatParams({ chat: "a", offset: -1 }).ok).toBe(false);
    expect(normalizeReadChatParams({ chat: "a", offset: 1.5 }).ok).toBe(false);
    expect(normalizeReadChatParams({ chat: "a", tools: "yes" }).ok).toBe(false);
    expect(normalizeReadChatParams(null).ok).toBe(false);
  });

  it("accepts only a well-shaped host answer", () => {
    expect(resolvedChat({ id: "a", name: "A", sessionFile: "/tmp/a.jsonl", project: "P", archived: true })).toEqual({
      id: "a", name: "A", sessionFile: "/tmp/a.jsonl", project: "P", workspace: undefined, model: undefined, archived: true, lastActiveAt: undefined,
    });
    expect(resolvedChat({ id: "a", name: "A" })).toBeUndefined();
    expect(resolvedChat("nope")).toBeUndefined();
  });

  it("slices by offset and says how to continue", () => {
    const full = Array.from({ length: 4_000 }, (_, index) => `line ${index} ${"y".repeat(80)}`).join("\n");
    const first = formatChatHistory(full, 0);
    expect(first.truncated).toBe(true);
    expect(first.text).toContain("Use offset=");
    expect(first.text.length).toBeLessThanOrEqual(DEFAULT_MAX_BYTES + 400);
    const second = formatChatHistory(full, 1_000);
    expect(second.text.startsWith(full.slice(1_000).slice(0, 20))).toBe(true);
    const end = formatChatHistory("short", 99);
    expect(end.text).toContain("is past the end of this chat");
    expect(end.truncated).toBe(false);
  });
});
