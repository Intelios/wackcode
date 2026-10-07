import { describe, expect, it } from "vitest";
import type { NormalizedBlock, NormalizedMessage } from "./types";
import { layoutTranscript } from "./explore-utils";
import { completedPlan, layoutWorkTurns } from "./completed-work";

const text = (value: string): NormalizedBlock => ({ type: "text", text: value });
const user = (id = "u", timestamp = 100): NormalizedMessage => ({ id, entryId: id, role: "user", timestamp, blocks: [text("Do the work")] });
const assistant = (id: string, blocks: NormalizedBlock[], extra: Partial<NormalizedMessage> = {}): NormalizedMessage =>
  ({ id, entryId: id, role: "assistant", blocks, ...extra });
const call = (id = "c", toolName = "bash"): NormalizedBlock => ({ type: "tool-call", toolCallId: id, toolName, arguments: { command: "pnpm test" } });
const result = (id = "c", extra: Partial<NormalizedBlock> = {}): NormalizedMessage =>
  ({ id: `r-${id}`, role: "tool", blocks: [{ type: "tool-result", toolCallId: id, text: "Passed", ...extra }] });
const answer = assistant("a", [text("Finished")], { stopReason: "stop", turn: { userEntryId: "u", endEntryId: "a" } });
const progress = assistant("p", [text("Checking the tests"), call()], { stopReason: "toolUse" });
const history = () => [user(), progress, result(), answer];

function project(messages: NormalizedMessage[], extra: Partial<Parameters<typeof layoutWorkTurns>[1]> = {}) {
  const results = new Map<string, NormalizedBlock>();
  const callIds = new Set<string>();
  messages.forEach((message) => message.blocks.forEach((block) => {
    if (block.type === "tool-call" && block.toolCallId) callIds.add(block.toolCallId);
    if (block.type === "tool-result" && block.toolCallId) results.set(block.toolCallId, block);
  }));
  return layoutWorkTurns(messages, {
    layout: layoutTranscript(messages, undefined, { grouping: false }), results, callIds,
    switchPositions: new Set(), running: false, ...extra
  });
}

function slots(turn: ReturnType<typeof project> extends Map<number, infer T> ? T : never, part: "work" | "outcome") {
  return turn.folded?.[part].flatMap((row) => row.type === "message" ? row.slots.map((slot) => [row.index, slot.type === "block" ? slot.index : "explore", row.actions]) : [row.type]);
}

describe("completed work layout", () => {
  it("partitions at every user, leaves originals intact, and projects actions only on the outcome", () => {
    const messages = [...history(), user("v", 200), assistant("b", [text("Another answer")])];
    const turns = project(messages);
    expect([...turns.keys()]).toEqual([0, 4]);
    const turn = turns.get(0)!;
    expect(turn.endIndex).toBe(4);
    expect(slots(turn, "work")).toEqual([[1, 0, false], [1, 1, false]]);
    expect(slots(turn, "outcome")).toEqual([[3, 0, true]]);
    expect(turns.get(4)?.folded).toBeUndefined();
    expect(messages[1]).toBe(progress);
    expect(messages[3]).toBe(answer);
    expect(progress.blocks[1]).toEqual(call());
  });

  it("folds reasoning and calls in the final message but keeps only trailing answer text", () => {
    const final = assistant("a", [text("Checking"), call(), { type: "thinking", text: "Reasoning" }, text("Finished"), text("Tests passed")]);
    const turn = project([user(), final, result()]).get(0)!;
    expect(slots(turn, "work")).toEqual([[1, 0, false], [1, 1, false], [1, 2, false]]);
    expect(slots(turn, "outcome")).toEqual([[1, 3, true], [1, 4, true]]);
  });

  it("supports legacy metadata/timing and does not create a fold for a direct answer", () => {
    expect(project([user(), assistant("p", [text("Reading")]), assistant("a", [text("Finished")])]).get(0)?.folded).toBeDefined();
    expect(project([user(), answer]).get(0)?.folded).toBeUndefined();
    expect(project([user(), assistant("a", [text(""), { type: "thinking", text: "" }])]).get(0)?.folded).toBeUndefined();
  });

  it("never mistakes a last assistant annotation for completion or folds steering during a live run", () => {
    const messages = [...history(), user("steer", 140), assistant("s", [text("Steering answer")])];
    const turns = project(messages, { running: true, activeRun: { startedAt: 90 } });
    expect(turns.get(0)?.folded).toBeUndefined();
    expect(turns.get(4)?.folded).toBeUndefined();
    expect(project(history(), { running: true }).get(0)?.folded).toBeUndefined();
    expect(project(history(), { running: true, activeRun: { startedAt: 200 } }).get(0)?.folded).toBeUndefined();
  });

  it("keeps finished historical work folded while a later run streams or stops", () => {
    const messages = [...history(), user("new", 300), progress];
    expect(project(messages, { running: true, activeRun: { startedAt: 290 } }).get(0)?.folded).toBeDefined();
    expect(project(messages, { status: "interrupted", partial: assistant("partial", [text("Unfinished")]) }).get(0)?.folded).toBeDefined();
    for (const status of ["interrupted", "error", "stopping"] as const) {
      expect(project(history(), { status }).get(0)?.folded).toBeUndefined();
    }
    expect(project(history(), { partial: assistant("partial", [text("Unfinished")]) }).get(0)?.folded).toBeUndefined();
  });

  it("does not hide failed/aborted or tool-only turns and tolerates delayed results", () => {
    for (const stopReason of ["error", "aborted"]) {
      expect(project([user(), progress, result(), { ...answer, stopReason }]).get(0)?.folded).toBeUndefined();
    }
    expect(project([user(), progress, result()]).get(0)?.folded).toBeUndefined();
    expect(project([user(), progress, answer]).get(0)?.folded).toBeUndefined();
    expect(project([user(), progress, result("c", { isError: true }), answer]).get(0)?.folded).toBeDefined();
  });

  it("recognizes result-based plan-only outcomes, folding old proposals and keeping the terminal footer", () => {
    const messages = [user(), progress, result(), assistant("old", [call("old", "plan_mode_complete")]), result("old", { details: { plan: "Old plan" } }),
      assistant("plan", [{ type: "thinking", text: "Prepare" }, call("plan", "plan_mode_complete")], { turn: { userEntryId: "u", endEntryId: "plan-result" } }),
      result("plan", { details: { plan: "Final plan" } })];
    const turn = project(messages).get(0)!;
    expect(slots(turn, "outcome")).toEqual([[5, 1, true]]);
    expect(slots(turn, "work")).toContainEqual([3, 0, false]);
    expect(slots(turn, "work")).toContainEqual([5, 0, false]);
    expect(completedPlan({ type: "tool-result", details: { plan: "Rejected" }, isError: true })).toBeUndefined();
    expect(completedPlan({ type: "tool-result", details: { plan: " " } })).toBeUndefined();
    expect(project([user(), assistant("plan", [call("plan", "plan_mode_complete")]), result("plan")]).get(0)?.folded).toBeUndefined();
  });

  it("keeps skill-creator previews as the outcome of their turn, not folded work", () => {
    const previewDetails = { v: 1, source: "skill_creator_preview", ownerTaskId: "t", draftId: "d", revision: "r", name: "pdf-tools", description: "Pdfs.", manual: false, target: "new", bodyPreview: "Do it", bodyTruncated: false, files: [], fileCount: 1, totalBytes: 8, warnings: [] };
    const messages = [user(), progress, result(),
      assistant("draft", [call("draft", "skill_creator")], { turn: { userEntryId: "u", endEntryId: "draft-result" } }),
      result("draft", { details: previewDetails })];
    const turn = project(messages).get(0)!;
    expect(turn.folded).toBeDefined();
    expect(slots(turn, "outcome")).toEqual([[3, 0, true]]);
    expect(slots(turn, "work")).toContainEqual([1, 0, false]);
    // A preview without a valid result is ordinary work, never a card outcome.
    expect(project([user(), assistant("draft", [call("draft", "skill_creator")]), result("draft")]).get(0)?.folded).toBeUndefined();
    expect(project([user(), assistant("draft", [call("draft", "skill_creator")]), result("draft", { details: previewDetails, isError: true })]).get(0)?.folded).toBeUndefined();
  });

  it("keeps orphan results and switch positions ordered without duplicating paired results", () => {
    const messages = [user(), progress, result(), result("orphan"), answer];
    const turn = project(messages, { switchPositions: new Set([1, 3, 5]) }).get(0)!;
    expect(turn.folded?.work.map((row) => row.type)).toEqual(["switches", "message", "switches", "orphans"]);
    expect(turn.folded?.work.find((row) => row.type === "orphans")).toMatchObject({ index: 3 });
    expect(turn.body.some((row) => row.type === "switches" && row.position === 5)).toBe(false);
  });

  it("leaves nonempty system notices unfolded, but skips empty notices", () => {
    const notice: NormalizedMessage = { id: "sys", role: "system", blocks: [text("Warning")] };
    expect(project([user(), progress, result(), notice, answer]).get(0)?.folded).toBeUndefined();
    expect(project([user(), progress, result(), { ...notice, blocks: [] }, answer]).get(0)?.folded).toBeDefined();
  });

  it("composes with exploration slots and never mutates their block indices", () => {
    const messages = [user(), assistant("reads", [call("one", "read"), call("two", "read")]), result("one"), result("two"), answer];
    const layout = layoutTranscript(messages, undefined, { grouping: true });
    const turn = project(messages, { layout }).get(0)!;
    const row = turn.folded?.work.find((entry) => entry.type === "message");
    expect(row?.type === "message" && row.slots[0]).toBe(layout.messages.get("reads")?.[0]);
    expect(slots(turn, "outcome")).toEqual([[4, 0, true]]);
  });

  it("gives sibling versions, changed answers and plans distinct disclosure identities", () => {
    const first = project(history()).get(0)!.key;
    expect(project([{ ...user(), id: "other", entryId: "other", versions: { group: "shared", index: 1, total: 2 } }, ...history().slice(1)]).get(0)!.key).not.toBe(first);
    expect(project([...history().slice(0, 3), { ...answer, id: "new", entryId: "new" }]).get(0)!.key).not.toBe(first);
    expect(project([user()]).get(0)?.folded).toBeUndefined();
  });
});
