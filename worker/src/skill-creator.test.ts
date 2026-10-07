import { describe, expect, it, vi } from "vitest";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { SkillCreatorState } from "./protocol.js";
import type { BuiltinHost } from "./builtin/host.js";
import { buildSkillCreatorPrompt } from "./builtin/skill-creator/guide.js";
import {
  SKILL_CREATOR_ENTRY_TYPE,
  SKILL_CREATOR_STATE_VERSION,
  restoreSkillCreatorState,
  toPersistedSkillCreator,
} from "./builtin/skill-creator/state.js";
import {
  SKILL_CREATOR_TOOL_NAME,
  createSkillCreatorExtension,
  normalizeSkillCreatorParams,
  parseSkillPreviewDetails,
  skillPreviewDetails,
} from "./builtin/skill-creator/index.js";

function fakeHost(overrides: Partial<BuiltinHost> = {}): BuiltinHost {
  return {
    publishTitleResult: vi.fn(),
    askQuestions: vi.fn(),
    publishPlanState: vi.fn(),
    publishSkillCreatorState: vi.fn(),
    publishTodoState: vi.fn(),
    publishGoalState: vi.fn(),
    recordCommandPresentation: vi.fn(),
    runGoalVerification: vi.fn(),
    childToolNames: () => [],
    runSubagent: vi.fn(),
    redact: (text) => text,
    notice: vi.fn(),
    workspace: () => "/workspace",
    taskId: () => "task-1",
    browser: vi.fn(),
    skillCreator: vi.fn(),
    computer: vi.fn(),
    supportsVision: () => false,
    ...overrides,
  } as BuiltinHost;
}

/** The slice of ExtensionAPI the skill-creator extension touches. */
function captureExtension() {
  let registered: { name: string; execute: (id: string, params: unknown, signal?: AbortSignal) => Promise<unknown> } | undefined;
  const entries: Array<{ type: string; data: unknown }> = [];
  const listeners = new Map<string, (event: unknown, ctx: unknown) => void>();
  const api = {
    registerTool: (tool: { name: string; execute: (id: string, params: unknown, signal?: AbortSignal) => Promise<unknown> }) => {
      if (tool.name === SKILL_CREATOR_TOOL_NAME) registered = tool;
    },
    registerCommand: vi.fn(),
    appendEntry: (type: string, data: unknown) => { entries.push({ type, data }); },
    on: (event: string, handler: (event: unknown, ctx: unknown) => void) => {
      listeners.set(event, handler);
      return () => listeners.delete(event);
    },
  };
  return {
    api: api as unknown as ExtensionAPI,
    registered: () => registered,
    entries,
    fire: (event: string, ctx: unknown) => listeners.get(event)?.({}, ctx),
  };
}

const PREPARE_RESULT = {
  draftId: "0f14d0ab-9605-4a62-a9e4-5ed26688389b",
  draftRoot: "/agent/task-1/skill-creator/0f14d0ab",
  skillDir: "/agent/task-1/skill-creator/0f14d0ab/skill",
  evalsDir: "/agent/task-1/skill-creator/0f14d0ab/evals",
  name: "pdf-tools",
};

const PREVIEW_RESULT = {
  draftId: PREPARE_RESULT.draftId,
  revision: "a".repeat(64),
  name: "pdf-tools",
  description: "Extract text from PDFs.",
  manual: false,
  target: "new",
  bodyPreview: "# PDF tools\n\nDo the thing.",
  bodyTruncated: false,
  files: ["scripts/extract.sh"],
  fileCount: 2,
  totalBytes: 2048,
  warnings: [],
};

function harness(deps: { commandEnabled?: () => boolean; isBuildMode?: () => boolean } = {}, host: BuiltinHost = fakeHost()) {
  const capture = captureExtension();
  const extension = createSkillCreatorExtension(host, {
    commandEnabled: deps.commandEnabled ?? (() => true),
    isBuildMode: deps.isBuildMode ?? (() => true),
  });
  extension.factory(capture.api);
  return { capture, controller: extension.controller, host };
}

describe("skill-creator params and details", () => {
  it("normalizes prepare and preview params", () => {
    expect(normalizeSkillCreatorParams({ action: "prepare", name: "pdf-tools" })).toEqual({
      ok: true, value: { action: "prepare", name: "pdf-tools" }
    });
    expect(normalizeSkillCreatorParams({ action: "preview", draftId: "abc" })).toEqual({
      ok: true, value: { action: "preview", draftId: "abc" }
    });
    expect(normalizeSkillCreatorParams({ action: "prepare" }).ok).toBe(false);
    expect(normalizeSkillCreatorParams({ action: "preview" }).ok).toBe(false);
    expect(normalizeSkillCreatorParams({ action: "destroy" }).ok).toBe(false);
    expect(normalizeSkillCreatorParams("nope").ok).toBe(false);
  });

  it("builds versioned details and re-caps what rides snapshots", () => {
    const details = skillPreviewDetails("task-1", {
      ...PREVIEW_RESULT,
      bodyPreview: "x".repeat(20_000),
      files: Array.from({ length: 250 }, (_, index) => `file-${index}`),
    });
    expect(details.v).toBe(1);
    expect(details.ownerTaskId).toBe("task-1");
    expect(details.revision).toBe(PREVIEW_RESULT.revision);
    expect(details.bodyPreview.length).toBe(16_000);
    expect(details.bodyTruncated).toBe(true);
    expect(details.files.length).toBe(200);
    expect(parseSkillPreviewDetails(details)).toEqual(details);
  });

  it("refuses foreign or incomplete details shapes", () => {
    expect(parseSkillPreviewDetails(undefined)).toBeUndefined();
    expect(parseSkillPreviewDetails({ v: 1, source: "something_else" })).toBeUndefined();
    expect(parseSkillPreviewDetails({ ...PREVIEW_RESULT, v: 2, source: "skill_creator_preview" })).toBeUndefined();
    expect(parseSkillPreviewDetails({ ...PREVIEW_RESULT, ownerTaskId: "" })).toBeUndefined();
    expect(parseSkillPreviewDetails({ ...PREVIEW_RESULT, target: "weird" })).toBeUndefined();
    expect(parseSkillPreviewDetails({ ...PREVIEW_RESULT, files: "nope" })).toBeUndefined();
  });
});

describe("skill-creator state persistence", () => {
  it("restores the latest entry on the branch and clears on demand", () => {
    const state: SkillCreatorState = { draftId: "d-1", name: "pdf-tools", revision: "r-1" };
    expect(toPersistedSkillCreator(state)).toEqual({
      version: SKILL_CREATOR_STATE_VERSION, phase: "active", draftId: "d-1", name: "pdf-tools", revision: "r-1"
    });
    expect(toPersistedSkillCreator(undefined)).toMatchObject({ phase: "cleared" });
    const branch = [
      { type: "custom", customType: SKILL_CREATOR_ENTRY_TYPE, data: toPersistedSkillCreator({ draftId: "old", name: "old" }) },
      { type: "message" },
      { type: "custom", customType: SKILL_CREATOR_ENTRY_TYPE, data: toPersistedSkillCreator(state) },
    ];
    expect(restoreSkillCreatorState(branch)).toEqual(state);
    expect(restoreSkillCreatorState([{ type: "custom", customType: SKILL_CREATOR_ENTRY_TYPE, data: toPersistedSkillCreator(undefined) }])).toBeUndefined();
    expect(restoreSkillCreatorState([])).toBeUndefined();
  });
});

describe("skill-creator guide", () => {
  it("carries the request, the workspace and the interview-first fallback", () => {
    const withRequest = buildSkillCreatorPrompt("turn this chat into a skill", { workspace: () => "/w" });
    expect(withRequest).toContain("turn this chat into a skill");
    expect(withRequest).toContain("/w");
    expect(withRequest).toContain("Never write into the shared skill library");
    const empty = buildSkillCreatorPrompt("  ", { workspace: () => undefined });
    expect(empty).toContain("asking what the skill should do");
  });
});

describe("skill_creator tool", () => {
  it("is withheld while /skill-creator is switched off", () => {
    const { controller } = harness({ commandEnabled: () => false });
    expect(controller.inactiveTools()).toEqual([SKILL_CREATOR_TOOL_NAME]);
    const { controller: on } = harness();
    expect(on.inactiveTools()).toEqual([]);
  });

  it("refuses outside a workflow, outside Build mode, and while switched off", async () => {
    const off = harness({ commandEnabled: () => false });
    await expect(off.capture.registered()!.execute("t1", { action: "prepare", name: "x" })).rejects.toThrow("switched off");

    const planning = harness({ isBuildMode: () => false });
    await expect(planning.capture.registered()!.execute("t1", { action: "prepare", name: "x" })).rejects.toThrow("Build mode");

    const idle = harness();
    await expect(idle.capture.registered()!.execute("t1", { action: "prepare", name: "x" })).rejects.toThrow("/skill-creator workflow");
  });

  it("prepares a draft during the command's run even before any state exists", async () => {
    const host = fakeHost({ skillCreator: vi.fn().mockResolvedValue(PREPARE_RESULT) });
    const { capture, controller } = harness({}, host);
    controller.setCommandRun(true);
    const result = (await capture.registered()!.execute("t1", { action: "prepare", name: "pdf-tools" })) as {
      content: Array<{ text: string }>;
    };
    controller.setCommandRun(false);
    expect(result.content[0]?.text).toContain(PREPARE_RESULT.skillDir);
    expect(host.skillCreator).toHaveBeenCalledWith({ op: "prepare", name: "pdf-tools" }, undefined);
    expect(controller.getState()).toEqual({ draftId: PREPARE_RESULT.draftId, name: "pdf-tools" });
    // The workflow entry persisted, so the state survives later turns and restarts.
    expect(capture.entries.at(-1)).toMatchObject({ type: SKILL_CREATOR_ENTRY_TYPE });
  });

  it("previews the active draft, ends the turn, and publishes the revision", async () => {
    const host = fakeHost({
      skillCreator: vi.fn()
        .mockResolvedValueOnce(PREPARE_RESULT)
        .mockResolvedValueOnce(PREVIEW_RESULT),
    });
    const { capture, controller } = harness({}, host);
    controller.setCommandRun(true);
    await capture.registered()!.execute("t1", { action: "prepare", name: "pdf-tools" });
    controller.setCommandRun(false);

    // A feedback turn (no command run) still previews: the workflow entry carries the draft.
    const preview = (await capture.registered()!.execute("t2", { action: "preview", draftId: PREPARE_RESULT.draftId })) as {
      content: Array<{ text: string }>;
      details: unknown;
      terminate: boolean;
    };
    expect(preview.terminate).toBe(true);
    expect(parseSkillPreviewDetails(preview.details)?.name).toBe("pdf-tools");
    expect(controller.getState()).toMatchObject({ revision: PREVIEW_RESULT.revision });

    // Another draft's id is refused.
    await expect(capture.registered()!.execute("t3", { action: "preview", draftId: "other" })).rejects.toThrow("active draft");
  });

  it("turns host failures into failed tool results (no card)", async () => {
    const host = fakeHost({
      skillCreator: vi.fn()
        .mockResolvedValueOnce(PREPARE_RESULT)
        .mockRejectedValueOnce(new Error("The draft has no SKILL.md yet.")),
    });
    const { capture, controller } = harness({}, host);
    controller.setCommandRun(true);
    await capture.registered()!.execute("t1", { action: "prepare", name: "pdf-tools" });
    controller.setCommandRun(false);
    await expect(capture.registered()!.execute("t2", { action: "preview", draftId: PREPARE_RESULT.draftId })).rejects.toThrow("no SKILL.md");
  });

  it("restores the branch state so navigation and reloads re-derive the workflow", async () => {
    const host = fakeHost({ skillCreator: vi.fn().mockResolvedValue(PREPARE_RESULT) });
    const { capture, controller } = harness({}, host);
    controller.setCommandRun(true);
    await capture.registered()!.execute("t1", { action: "prepare", name: "pdf-tools" });
    controller.setCommandRun(false);
    // A session_tree event (rewind to before the draft) clears it again.
    capture.fire("session_tree", { sessionManager: { getBranch: () => [] } });
    expect(controller.getState()).toBeUndefined();
    await expect(capture.registered()!.execute("t2", { action: "preview", draftId: PREPARE_RESULT.draftId })).rejects.toThrow("/skill-creator workflow");
  });
});
