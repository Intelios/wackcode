import { describe, expect, it } from "vitest";
import {
  CHAT_REFS_GUIDANCE, chatMentionToken, chatRefAllowed, chatRefCandidates, encodeChatRefs, MAX_CHAT_REFS,
  rankChats, rankSuggestions, splitChatRefSection, survivingChatRefs,
} from "./chat-refs";
import { composeFileSection, splitFileSection } from "./attachment-utils";
import { appendContextSection } from "./browser-context";
import type { BrowserContextRecord, ProjectRecord, TaskRecord } from "./types";

function task(id: string, name: string, overrides: Partial<TaskRecord> = {}): TaskRecord {
  return {
    id, name, kind: "code", projectId: null, autoTitleEligible: false, autoTitleAttemptId: null,
    workspacePath: `/tmp/${id}`, worktreePath: null, branch: null, usesWorktree: false,
    providerId: "p", modelId: "m", thinkingLevel: "off", sessionFile: `${id}.jsonl`,
    status: "idle", mode: "build", archived: false, archivedAt: null, lastError: null,
    createdAt: "2026-01-01T00:00:00Z", updatedAt: "2026-01-01T00:00:00Z", lastActivityAt: null,
    originSuggestion: null, dismissedSuggestions: [],
    ...overrides
  };
}

const project = (id: string, name: string): ProjectRecord => ({
  id, name, path: `/tmp/${id}`, gitRoot: null, gitHasHead: false, runCommand: null, branch: null, createdAt: "2026-01-01"
});

describe("encodeChatRefs", () => {
  it("appends one section with the guidance line and leaves simple text alone when empty", () => {
    expect(encodeChatRefs("hello", [])).toBe("hello");
    const encoded = encodeChatRefs("look at @\"Auth refactor\"", [{ id: "a", name: "Auth refactor" }]);
    expect(encoded.startsWith("look at @\"Auth refactor\"\n\n<referenced-chats-v1>\n")).toBe(true);
    expect(encoded).toContain(CHAT_REFS_GUIDANCE);
    expect(encoded).toContain('{"version":1,"chats":[{"id":"a","name":"Auth refactor"}]}');
    expect(encoded.endsWith("</referenced-chats-v1>")).toBe(true);
    // No words at all: the section is the whole message.
    expect(encodeChatRefs("", [{ id: "a", name: "Auth refactor" }]).startsWith("<referenced-chats-v1>")).toBe(true);
  });

  it("de-duplicates by id and caps the list", () => {
    const encoded = encodeChatRefs("x", [{ id: "a", name: "A" }, { id: "a", name: "A again" }]);
    expect(encoded.match(/"id":"a"/g)).toHaveLength(1);
    const many = Array.from({ length: MAX_CHAT_REFS + 4 }, (_, index) => ({ id: `c${index}`, name: `Chat ${index}` }));
    const capped = splitChatRefSection(encodeChatRefs("x", many)).chatRefs;
    expect(capped).toHaveLength(MAX_CHAT_REFS);
    expect(splitChatRefSection(encodeChatRefs("x", [{ id: "", name: "nameless" }])).chatRefs).toEqual([]);
  });
});

describe("splitChatRefSection", () => {
  it("round-trips with and without words around it", () => {
    const refs = [{ id: "a", name: "Auth refactor" }];
    expect(splitChatRefSection(encodeChatRefs("keep going", refs))).toEqual({ text: "keep going", chatRefs: refs });
    expect(splitChatRefSection(encodeChatRefs("", refs))).toEqual({ text: "", chatRefs: refs });
  });

  it("strips every generated tail in splitFileSection, in the order they were appended", () => {
    const chatRefs = [{ id: "a", name: "Auth refactor" }];
    const context: BrowserContextRecord = {
      id: "c", kind: "element", capturedAt: 1, url: "https://example.test", title: "Page",
      viewport: { preset: "desktop", width: 1440, height: 900 }, imageIndex: 0,
      targets: [{ selector: "main", label: "Main", dom: "<main></main>", bounds: { x: 0, y: 0, width: 1, height: 1 }, path: [] }]
    };
    // The wire order: words, browser contexts, referenced chats, attached files.
    const wire = composeFileSection(
      encodeChatRefs(appendContextSection("continue @\"Auth refactor\"", [context]), chatRefs),
      [{ name: "notes.txt", text: "a\nb" }]
    );
    const split = splitFileSection(wire);
    expect(split.text).toBe("continue @\"Auth refactor\"");
    expect(split.chatRefs).toEqual(chatRefs);
    expect(split.browserContexts).toEqual([context]);
    expect(split.files.map((file) => file.name)).toEqual(["notes.txt"]);
  });

  it("leaves a malformed or lookalike section in the text", () => {
    const lookalike = "we discussed <referenced-chats-v1> before";
    expect(splitChatRefSection(lookalike)).toEqual({ text: lookalike, chatRefs: [] });
    const broken = 'words\n\n<referenced-chats-v1>\n{"version":1,"chats":[{"id":"a"}]}\n</referenced-chats-v1>';
    expect(splitChatRefSection(broken)).toEqual({ text: broken, chatRefs: [] });
    const duplicated = 'words\n\n<referenced-chats-v1>\n{"version":1,"chats":[{"id":"a","name":"A"},{"id":"a","name":"B"}]}\n</referenced-chats-v1>';
    expect(splitChatRefSection(duplicated).chatRefs).toEqual([]);
  });
});

describe("survivingChatRefs", () => {
  it("keeps only the refs whose token is still in the text", () => {
    const refs = [{ id: "a", name: "Auth refactor" }, { id: "b", name: "Design" }];
    const text = `continue @${chatMentionToken("Auth refactor").slice(1)} please`;
    expect(survivingChatRefs(text, refs)).toEqual([{ id: "a", name: "Auth refactor" }]);
    expect(survivingChatRefs("nothing here", refs)).toEqual([]);
  });
});

describe("chatRefAllowed", () => {
  it("restricts Chat mode to Chat mode chats and lets Code read either", () => {
    expect(chatRefAllowed("chat", "code")).toBe(false);
    expect(chatRefAllowed("chat", "chat")).toBe(true);
    expect(chatRefAllowed("code", "code")).toBe(true);
    expect(chatRefAllowed("code", "chat")).toBe(true);
  });
});

describe("chatRefCandidates", () => {
  const projects = [project("p1", "WackCode"), project("p2", "Docs")];
  const tasks = [
    task("a", "Auth refactor", { projectId: "p1", lastActivityAt: "2026-02-03T00:00:00Z" }),
    task("b", "Design notes", { projectId: "p2", lastActivityAt: "2026-02-04T00:00:00Z" }),
    task("c", "Plain chat", { kind: "chat", lastActivityAt: "2026-02-05T00:00:00Z" }),
    task("d", "Old one", { archived: true, updatedAt: "2026-01-01T00:00:00Z" })
  ];

  it("excludes the current chat and applies the kind rule, newest activity first", () => {
    const code = chatRefCandidates(tasks, projects, "code", "a");
    expect(code.map((chat) => chat.id)).toEqual(["c", "b", "d"]);
    expect(code[0]).toMatchObject({ kind: "chat", name: "Plain chat", label: "No project" });
    expect(code[1].label).toBe("Docs");
    expect(code[2]).toMatchObject({ archived: true, label: "No project · Archived" });

    const chat = chatRefCandidates(tasks, projects, "chat", "c");
    expect(chat.map((entry) => entry.id)).toEqual([]);
  });
});

describe("rankChats", () => {
  const chats = chatRefCandidates([
    task("a", "Auth refactor", { lastActivityAt: "2026-02-01T00:00:00Z" }),
    task("b", "Auth cleanup", { lastActivityAt: "2026-02-05T00:00:00Z" }),
    task("c", "Design", { lastActivityAt: "2026-02-09T00:00:00Z" }),
    task("d", "Reauthorize", { lastActivityAt: "2026-02-10T00:00:00Z" })
  ], [], "code");

  it("ranks exact, prefix, contains, then fuzzy, with newer activity breaking ties", () => {
    // Both prefixes score the same, so the newer chat leads; the contains and fuzzy matches follow.
    expect(rankChats(chats, "auth").map((chat) => chat.id)).toEqual(["b", "a", "d"]);
    // An exact name outranks a prefix, whichever ran last.
    expect(rankChats(chats, "AUTH cleanup").map((chat) => chat.id)).toEqual(["b"]);
    expect(rankChats(chats, "Auth refactor").at(0)?.name).toBe("Auth refactor");
    expect(rankChats(chats, "dsgn").map((chat) => chat.id)).toEqual(["c"]);
    expect(rankChats(chats, "zzz")).toEqual([]);
  });

  it("lists nothing for an empty query and caps the list", () => {
    expect(rankChats(chats, "")).toEqual([]);
    expect(rankChats(chats, "a", 2)).toHaveLength(2);
  });
});

describe("rankSuggestions", () => {
  const files = ["README.md", "src/App.tsx"];
  const chats = chatRefCandidates([task("a", "README rewrite")], [], "code");

  it("keeps the files' order and appends matching chats under them", () => {
    const entries = rankSuggestions(files, chats, "rea");
    expect(entries.map((entry) => entry.kind)).toEqual(["file", "chat"]);
    expect(entries[0]).toMatchObject({ kind: "file", path: "README.md" });
    expect(entries[1]).toMatchObject({ kind: "chat", id: "a" });
  });

  it("works before the file list has loaded, and with no query", () => {
    expect(rankSuggestions(undefined, chats, "read").map((entry) => entry.kind)).toEqual(["chat"]);
    expect(rankSuggestions(files, chats, "")).toEqual([
      { kind: "file", path: "src/", directory: true },
      { kind: "file", path: "README.md", directory: false }
    ]);
  });
});
