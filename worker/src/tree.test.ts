import { describe, expect, it } from "vitest";
import { RUN_TIMING_ENTRY_TYPE } from "./run-timing.js";
import {
  CHECKPOINT_ENTRY_TYPE,
  COMMAND_PRESENTATION_ENTRY_TYPE,
  COMMAND_PRESENTATION_VERSION,
  LEAVE_ENTRY_TYPE,
  NAV_ENTRY_TYPE,
  TREE_MARKER_VERSION,
  buildTreeIndex,
  checkpointBefore,
  commandPresentationBefore,
  latestInSubtree,
  leftWith,
  modelSwitchesOnPath,
  turnsOnPath,
  undoTarget,
  versionsOf,
  type EntryLike
} from "./tree.js";

function message(id: string, parentId: string | null, role: string): EntryLike {
  return { type: "message", id, parentId, message: { role, content: [] } };
}

function custom(id: string, parentId: string | null, customType: string, data: unknown = {}): EntryLike {
  return { type: "custom", id, parentId, customType, data };
}

function checkpoint(id: string, parentId: string | null, tree: string | null): EntryLike {
  return custom(id, parentId, CHECKPOINT_ENTRY_TYPE, { version: TREE_MARKER_VERSION, id: tree });
}

function leave(id: string, parentId: string | null, tree: string | null): EntryLike {
  return custom(id, parentId, LEAVE_ENTRY_TYPE, { version: TREE_MARKER_VERSION, checkpoint: tree ? { id: tree } : null });
}

function nav(id: string, parentId: string | null, kind: string, from: string): EntryLike {
  return custom(id, parentId, NAV_ENTRY_TYPE, { version: TREE_MARKER_VERSION, kind, from });
}

function model(id: string, parentId: string | null, provider: string, modelId: string): EntryLike {
  return { type: "model_change", id, parentId, provider, modelId };
}

/**
 * system → [plan] → [ckpt a] → u1 → a1 → [timing] ─┬─ [leave] … (abandoned first version)
 *                                                 │   └ [plan2] → [ckpt c] → u2 → a2
 *                                                 └ [nav] → [ckpt b] → [system] → u2b → a2b
 */
function session(): EntryLike[] {
  return [
    message("sys", null, "system"),
    custom("plan", "sys", "wackcode-plan-state"),
    checkpoint("ck-a", "plan", "tree-a"),
    message("u1", "ck-a", "user"),
    message("a1", "u1", "assistant"),
    custom("timing1", "a1", RUN_TIMING_ENTRY_TYPE),
    custom("plan2", "timing1", "wackcode-plan-state"),
    checkpoint("ck-c", "plan2", "tree-c"),
    message("u2", "ck-c", "user"),
    message("a2", "u2", "assistant"),
    leave("left", "a2", "tree-left"),
    nav("nav", "plan2", "resend", "left"),
    checkpoint("ck-b", "nav", "tree-b"),
    message("sys2", "ck-b", "system"),
    message("u2b", "sys2", "user"),
    message("a2b", "u2b", "assistant")
  ];
}

describe("session tree helpers", () => {
  it("groups versions of a message by the conversation message before them", () => {
    const index = buildTreeIndex(session());
    expect(index.logicalParent.get("u2")).toBe("a1");
    expect(index.logicalParent.get("u2b")).toBe("a1");
    expect(versionsOf(index, "u2")).toEqual({ index: 0, total: 2, previous: undefined, next: "u2b", group: "versions:a1" });
    expect(versionsOf(index, "u2b")).toEqual({ index: 1, total: 2, previous: "u2", next: undefined, group: "versions:a1" });
    expect(versionsOf(index, "u1")).toMatchObject({ index: 0, total: 1, group: "versions:root" });
  });

  it("finds the checkpoint just above each version and never one from another version", () => {
    const entries = session();
    const index = buildTreeIndex(entries);
    expect(checkpointBefore(index, "u1")).toEqual({ id: "tree-a" });
    expect(checkpointBefore(index, "u2")).toEqual({ id: "tree-c" });
    expect(checkpointBefore(index, "u2b")).toEqual({ id: "tree-b" });

    // A version sent while no snapshot was possible must not reach past its own marker.
    const withoutSnapshot = entries.map((entry) => entry.id === "ck-b" ? checkpoint("ck-b", "nav", null) : entry);
    expect(checkpointBefore(buildTreeIndex(withoutSnapshot), "u2b")).toBeUndefined();
  });

  it("reads only a versioned command marker attached to that user message", () => {
    const presentation = { id: "app:goal", name: "goal", arguments: "Ship it", kind: "command" as const };
    const entries = [
      custom("command", null, COMMAND_PRESENTATION_ENTRY_TYPE, { version: COMMAND_PRESENTATION_VERSION, presentation }),
      message("user", "command", "user"),
      message("answer", "user", "assistant"),
      message("plain", "answer", "user")
    ];
    const index = buildTreeIndex(entries);
    expect(commandPresentationBefore(index, "user")).toEqual(presentation);
    expect(commandPresentationBefore(index, "plain")).toBeUndefined();

    const cleared = [
      entries[0],
      custom("ordinary", "command", COMMAND_PRESENTATION_ENTRY_TYPE, { version: COMMAND_PRESENTATION_VERSION, presentation: null }),
      message("edited", "ordinary", "user")
    ];
    expect(commandPresentationBefore(buildTreeIndex(cleared), "edited")).toBeUndefined();

    const invalid = entries.map((entry) => entry.id === "command"
      ? custom("command", null, COMMAND_PRESENTATION_ENTRY_TYPE, { version: 999, presentation })
      : entry);
    expect(commandPresentationBefore(buildTreeIndex(invalid), "user")).toBeUndefined();
  });

  it("finds the newest entry under a message, landing on the marker a branch was left with", () => {
    const index = buildTreeIndex(session());
    expect(latestInSubtree(index, "u2")).toBe("left");
    expect(latestInSubtree(index, "u2b")).toBe("a2b");
    expect(latestInSubtree(index, "a1")).toBe("a2b");
  });

  it("marks where each turn ends and the files a checkpoint recorded after it", () => {
    const entries = session();
    const byId = new Map(entries.map((entry) => [entry.id, entry]));
    const path = ["sys", "plan", "ck-a", "u1", "a1", "timing1", "plan2", "ck-c", "u2", "a2", "left"].map((id) => byId.get(id)!);
    const turns = turnsOnPath(path);
    expect(turns.get("u1")).toEqual({ userEntryId: "u1", endEntryId: "timing1", after: { id: "tree-c" } });
    expect(turns.get("u2")).toEqual({ userEntryId: "u2", endEntryId: "a2", after: { id: "tree-left" } });
    expect(leftWith(path)).toEqual({ id: "tree-left" });
    expect(leftWith(path.slice(0, -1))).toBeUndefined();
  });

  it("keeps an auto-retried failure inside its turn", () => {
    const path = [
      message("u", null, "user"),
      message("failed", "u", "assistant"),
      message("retried", "failed", "assistant"),
      custom("timing", "retried", RUN_TIMING_ENTRY_TYPE)
    ];
    expect(turnsOnPath(path).get("u")).toEqual({ userEntryId: "u", endEntryId: "timing" });
  });

  it("offers undo only while the conversation still ends at a rewind", () => {
    expect(undoTarget(nav("n", "x", "rewind", "left"))).toBe("left");
    expect(undoTarget(nav("n", "x", "switch", "left"))).toBeUndefined();
    expect(undoTarget(message("m", null, "assistant"))).toBeUndefined();
  });

  it("places genuine model switches on the visible active path", () => {
    const path = [
      model("initial", null, "openai", "gpt-4"),
      model("before-chat", "initial", "openai", "gpt-5"),
      message("u1", "before-chat", "user"),
      message("a1", "u1", "assistant"),
      model("same", "a1", "openai", "gpt-5"),
      model("sonnet", "same", "anthropic", "sonnet-4"),
      message("u2", "sonnet", "user"),
      model("opus", "u2", "anthropic", "opus-4"),
      message("a2", "opus", "assistant")
    ];
    const positions = new Map([["u1", 0], ["a1", 1], ["u2", 2], ["a2", 3]]);
    expect(modelSwitchesOnPath(path, positions)).toEqual([
      {
        id: "sonnet", at: 2,
        from: { providerId: "openai", modelId: "gpt-5" },
        to: { providerId: "anthropic", modelId: "sonnet-4" }
      },
      {
        id: "opus", at: 3,
        from: { providerId: "anthropic", modelId: "sonnet-4" },
        to: { providerId: "anthropic", modelId: "opus-4" }
      }
    ]);
    expect(modelSwitchesOnPath(path.slice(0, 4), positions)).toEqual([]);
    // As after compaction, switches above the first still-visible conversation message are a
    // baseline, not a divider stranded at the top of the transcript.
    expect(modelSwitchesOnPath(path, new Map([["u2", 0], ["a2", 1]]))).toEqual([{
      id: "opus", at: 1,
      from: { providerId: "anthropic", modelId: "sonnet-4" },
      to: { providerId: "anthropic", modelId: "opus-4" }
    }]);
  });
});
