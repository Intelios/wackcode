import { describe, expect, it } from "vitest";
import type { NormalizedMessage } from "./types";
import { changeLabel, defaultSelection, hasVisibleMessages, latestTurn, messageText, userOfTurn, workspacePrefix } from "./tree-utils";

function message(id: string, role: NormalizedMessage["role"], extra: Partial<NormalizedMessage> = {}): NormalizedMessage {
  return { id, entryId: id, role, blocks: [{ type: "text", text: `${id} text` }], ...extra };
}

describe("tree utils", () => {
  it("finds the latest turn and the user message a turn answers", () => {
    const messages = [
      message("u1", "user"),
      message("a1", "assistant", { turn: { userEntryId: "u1", endEntryId: "a1" } }),
      message("u2", "user"),
      message("a2", "assistant"),
      message("t2", "tool"),
      message("a2b", "assistant", { turn: { userEntryId: "u2", endEntryId: "a2b" } })
    ];
    expect(latestTurn(messages)?.user.id).toBe("u2");
    expect(latestTurn(messages)?.answer?.id).toBe("a2b");
    expect(userOfTurn(messages, messages[1])?.id).toBe("u1");
    expect(latestTurn(messages.slice(0, 3))).toEqual({ user: messages[2], answer: undefined });
    expect(latestTurn([{ ...messages[0], entryId: undefined }])).toBeUndefined();
  });

  it("treats a transcript of only system entries as empty", () => {
    expect(hasVisibleMessages([message("s", "system")])).toBe(false);
    expect(hasVisibleMessages([message("s", "system"), message("u", "user")])).toBe(true);
  });

  it("copies only a message's text", () => {
    expect(messageText({ id: "m", role: "assistant", blocks: [{ type: "thinking", text: "hmm" }, { type: "text", text: " Done. " }] })).toBe("Done.");
  });

  it("pre-selects only the chat's own folder when checkpoints cover a whole repository", () => {
    const changes = [
      { path: "app/src/main.ts", status: "revert" as const },
      { path: "other/readme.md", status: "delete" as const }
    ];
    expect(workspacePrefix("/repo", "/repo/app")).toBe("app/");
    expect(workspacePrefix("/repo", "/repo")).toBe("");
    expect(workspacePrefix("/repo", "/elsewhere")).toBe("");
    expect(workspacePrefix("/worktrees/abc", "/worktrees/abc/")).toBe("");
    expect(workspacePrefix("/worktrees/abc/", "/worktrees/abc/app/")).toBe("app/");
    expect(defaultSelection(changes, "app/")).toEqual(["app/src/main.ts"]);
    expect(defaultSelection(changes, "")).toEqual(["app/src/main.ts", "other/readme.md"]);
    expect(changeLabel("recreate")).toBe("Restored");
  });
});
