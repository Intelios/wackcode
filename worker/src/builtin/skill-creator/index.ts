/**
 * Built-in skill creator — the working side of `/skill-creator`. The model gets one sequential
 * tool, `skill_creator`, whose two actions both run in the host: `prepare` creates the managed
 * draft workspace (snapshotting an existing skill when improving one), and `preview` validates
 * the draft and returns the versioned details the desktop renders as the review card. The tool
 * exists only while a workflow is active on the current branch (or the /skill-creator run is
 * live), is Build-only, and is withheld entirely while the command is switched off.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { SkillCreatorState, SkillPreviewDetails } from "../../protocol.js";
import type { BuiltinHost } from "../host.js";
import {
  SKILL_CREATOR_ENTRY_TYPE,
  SKILL_CREATOR_STATE_VERSION,
  restoreSkillCreatorState,
  toPersistedSkillCreator,
} from "./state.js";

export const SKILL_CREATOR_TOOL_NAME = "skill_creator";
export const SKILL_CREATOR_TOOL_LABEL = "Skill creator";
export const SKILL_CREATOR_DETAILS_VERSION = 1;

/** Defensive caps mirroring the host's; the host already truncates, these make it structural. */
const MAX_BODY_PREVIEW_CHARS = 16_000;
const MAX_FILES_LISTED = 200;

export const SKILL_CREATOR_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["action"],
  properties: {
    action: {
      type: "string",
      enum: ["prepare", "preview"],
      description: "prepare: create the managed draft for an agreed skill name (and optionally an existing skill to improve). preview: validate the finished draft and show the user the review card.",
    },
    name: {
      type: "string",
      minLength: 1,
      maxLength: 64,
      description: "prepare only: the skill's name — lowercase letters, numbers and single hyphens, at most 64 characters.",
    },
    sourcePath: {
      type: "string",
      description: "prepare only: the absolute SKILL.md path of an existing skill to improve, as the user named it. Omit for a new skill.",
    },
    draftId: {
      type: "string",
      description: "preview only: the draftId the prepare step returned.",
    },
  },
} as const;

export interface SkillCreatorController {
  /** The branch's active workflow, for the snapshot the renderer sees. */
  getState(): SkillCreatorState | undefined;
  /** Tools that must stay out while /skill-creator is switched off in Settings. */
  inactiveTools(): string[];
  /** The /skill-creator run is live: `prepare` is allowed before any draft exists. */
  setCommandRun(active: boolean): void;
}

export interface SkillCreatorDeps {
  /** `/skill-creator` is switched on (its `app:` key is not in the command denylist). */
  commandEnabled(): boolean;
  /** The chat is in Build mode; the workflow never runs under planning restrictions. */
  isBuildMode(): boolean;
}

/** What the host returns for `prepare`. */
export interface PrepareResult {
  draftId: string;
  draftRoot: string;
  skillDir: string;
  evalsDir: string;
  originalDir?: string;
  name: string;
  originLabel?: string;
}

type NormalizeResult<T> = { ok: true; value: T } | { ok: false; error: string };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringField(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

export function normalizeSkillCreatorParams(input: unknown): NormalizeResult<{ action: "prepare" | "preview"; name?: string; sourcePath?: string; draftId?: string }> {
  if (!isRecord(input)) return { ok: false, error: "params must be an object" };
  const action = input.action;
  if (action !== "prepare" && action !== "preview") return { ok: false, error: "action must be \"prepare\" or \"preview\"" };
  const name = stringField(input.name);
  const sourcePath = stringField(input.sourcePath);
  const draftId = stringField(input.draftId);
  if (action === "prepare") {
    if (!name) return { ok: false, error: "prepare requires the agreed skill name" };
    if (name.length > 64) return { ok: false, error: "a skill name can be at most 64 characters" };
  } else if (!draftId) {
    return { ok: false, error: "preview requires the draftId the prepare step returned" };
  }
  return { ok: true, value: { action, ...(name ? { name } : {}), ...(sourcePath ? { sourcePath } : {}), ...(draftId ? { draftId } : {}) } };
}

/** The versioned details a preview result carries; invalid shapes render as an ordinary tool row. */
export function parseSkillPreviewDetails(value: unknown): SkillPreviewDetails | undefined {
  if (!isRecord(value) || value.v !== SKILL_CREATOR_DETAILS_VERSION || value.source !== "skill_creator_preview") return undefined;
  const strings = (key: keyof SkillPreviewDetails) => (typeof value[key] === "string" ? (value[key] as string) : undefined);
  const ownerTaskId = strings("ownerTaskId");
  const draftId = strings("draftId");
  const revision = strings("revision");
  const name = strings("name");
  const description = strings("description");
  const bodyPreview = strings("bodyPreview");
  const target = value.target;
  if (!ownerTaskId || !draftId || !revision || !name || !description || bodyPreview === undefined) return undefined;
  if (target !== "new" && target !== "library-update" && target !== "library-copy") return undefined;
  if (!Array.isArray(value.files) || !value.files.every((file) => typeof file === "string")) return undefined;
  if (!Array.isArray(value.warnings) || !value.warnings.every((warning) => typeof warning === "string")) return undefined;
  return {
    v: SKILL_CREATOR_DETAILS_VERSION,
    source: "skill_creator_preview",
    ownerTaskId,
    draftId,
    revision,
    name,
    description,
    manual: value.manual === true,
    ...(strings("argumentHint") ? { argumentHint: strings("argumentHint") } : {}),
    target,
    ...(strings("originLabel") ? { originLabel: strings("originLabel") } : {}),
    bodyPreview,
    bodyTruncated: value.bodyTruncated === true,
    files: (value.files as string[]).slice(0, MAX_FILES_LISTED),
    fileCount: typeof value.fileCount === "number" ? value.fileCount : (value.files as string[]).length,
    totalBytes: typeof value.totalBytes === "number" ? value.totalBytes : 0,
    warnings: value.warnings as string[],
  };
}

/** Build the card's details from the host's preview payload, re-capping what rides snapshots. */
export function skillPreviewDetails(ownerTaskId: string, preview: Record<string, unknown>): SkillPreviewDetails {
  const body = String(preview.bodyPreview ?? "");
  const truncated = body.length > MAX_BODY_PREVIEW_CHARS;
  const files = Array.isArray(preview.files) ? (preview.files as unknown[]).filter((file): file is string => typeof file === "string") : [];
  return {
    v: SKILL_CREATOR_DETAILS_VERSION,
    source: "skill_creator_preview",
    ownerTaskId,
    draftId: String(preview.draftId ?? ""),
    revision: String(preview.revision ?? ""),
    name: String(preview.name ?? ""),
    description: String(preview.description ?? ""),
    manual: preview.manual === true,
    ...(typeof preview.argumentHint === "string" && preview.argumentHint.trim() ? { argumentHint: preview.argumentHint } : {}),
    target: preview.target === "library-update" || preview.target === "library-copy" ? preview.target : "new",
    ...(typeof preview.originLabel === "string" && preview.originLabel ? { originLabel: preview.originLabel } : {}),
    bodyPreview: truncated ? body.slice(0, MAX_BODY_PREVIEW_CHARS) : body,
    bodyTruncated: preview.bodyTruncated === true || truncated,
    files: files.slice(0, MAX_FILES_LISTED),
    fileCount: typeof preview.fileCount === "number" ? preview.fileCount : files.length,
    totalBytes: typeof preview.totalBytes === "number" ? preview.totalBytes : 0,
    warnings: Array.isArray(preview.warnings) ? (preview.warnings as unknown[]).filter((warning): warning is string => typeof warning === "string") : [],
  };
}

export function createSkillCreatorExtension(host: BuiltinHost, deps: SkillCreatorDeps): {
  factory: (api: ExtensionAPI) => void;
  controller: SkillCreatorController;
} {
  let pi: ExtensionAPI | undefined;
  let state: SkillCreatorState | undefined;
  /** True while the run /skill-creator started is going: prepare needs no restored draft yet. */
  let commandRunActive = false;

  const persist = () => {
    try {
      pi?.appendEntry(SKILL_CREATOR_ENTRY_TYPE, { ...toPersistedSkillCreator(state), version: SKILL_CREATOR_STATE_VERSION });
    } catch { /* Persistence is best-effort; the workflow keeps working without it. */ }
  };
  const emit = () => host.publishSkillCreatorState(state ?? null);

  const describe = (details: SkillPreviewDetails): string => {
    const verb = details.target === "library-update" ? "updates" : details.target === "library-copy" ? "copies into Your skills" : "installs as a new skill";
    return [
      `**Skill draft ready — ${details.name}**`,
      "",
      details.description,
      "",
      `${details.fileCount} file${details.fileCount === 1 ? "" : "s"} · ${Math.max(1, Math.round(details.totalBytes / 1024))} KB · ${verb}.`,
      ...(details.warnings.length ? ["", ...details.warnings.map((warning) => `- ${warning}`)] : []),
      "",
      "The review card above is shown to the user; nothing is installed until they choose Save skill. If they give feedback, revise the draft and preview again.",
    ].join("\n");
  };

  async function execute(
    params: unknown,
    signal: AbortSignal | undefined,
  ): Promise<{ content: Array<{ type: "text"; text: string }>; details?: unknown; terminate?: boolean }> {
    if (!deps.commandEnabled()) throw new Error("That command is switched off in Settings › Commands.");
    if (!deps.isBuildMode()) throw new Error("Switch to Build mode before using the skill creator.");
    const parsed = normalizeSkillCreatorParams(params);
    if (!parsed.ok) throw new Error(parsed.error);
    if (!commandRunActive && !state) {
      throw new Error("The skill_creator tool only works inside a /skill-creator workflow. Start one with /skill-creator, or ask the user to.");
    }

    if (parsed.value.action === "prepare") {
      const result = await host.skillCreator(
        { op: "prepare", name: parsed.value.name!, ...(parsed.value.sourcePath ? { sourcePath: parsed.value.sourcePath } : {}) },
        signal,
      );
      const prepared = isRecord(result) ? result : undefined;
      const draftId = stringField(prepared?.draftId);
      const name = stringField(prepared?.name) ?? parsed.value.name!;
      if (!prepared || !draftId) throw new Error("The host could not prepare a draft workspace.");
      state = { draftId, name };
      persist();
      emit();
      const skillDir = stringField(prepared.skillDir) ?? "(unknown)";
      const lines = [
        `Draft workspace ready: ${stringField(prepared.draftRoot) ?? draftId}`,
        "",
        `- Author the skill in ${skillDir} (SKILL.md plus optional scripts/, references/, assets/).`,
        stringField(prepared.originalDir) ? `- The existing skill's snapshot is in ${prepared.originalDir}; the original itself is never touched.` : "- This is a new skill.",
        `- Optional example runs and their files go under ${stringField(prepared.evalsDir) ?? "evals/"}.`,
        "",
        "Write the files with your ordinary tools, then call skill_creator with action \"preview\" and this draftId:",
        draftId,
      ];
      return { content: [{ type: "text", text: lines.join("\n") }] };
    }

    if (!state || state.draftId !== parsed.value.draftId) {
      throw new Error(state ? "That draftId is not this chat's active draft." : "No active draft in this chat. Start with action \"prepare\".");
    }
    const preview = await host.skillCreator({ op: "preview", draftId: state.draftId }, signal);
    if (!isRecord(preview)) throw new Error("The host could not validate the draft.");
    if (typeof preview.error === "string" && preview.error) throw new Error(preview.error);
    const ownerTaskId = host.taskId();
    if (!ownerTaskId) throw new Error("This chat's draft workspace is no longer available.");
    const details = skillPreviewDetails(ownerTaskId, preview);
    if (!details.revision) throw new Error(String(preview.error ?? "The draft could not be validated."));
    state = { ...state, name: details.name || state.name, revision: details.revision };
    persist();
    emit();
    // The card is the outcome of the workflow's turn: stop rather than narrating past it.
    return { content: [{ type: "text", text: describe(details) }], details, terminate: true };
  }

  const factory = (bound: ExtensionAPI) => {
    pi = bound;
    bound.registerTool({
      name: SKILL_CREATOR_TOOL_NAME,
      label: SKILL_CREATOR_TOOL_LABEL,
      description:
        "Create or improve an Agent Skill (a SKILL.md instructions folder) with the user. Action \"prepare\" sets up the managed draft for an agreed skill name (optionally from an existing skill's path); action \"preview\" validates the finished draft and shows the user a review card they save from. Only available during a /skill-creator workflow in Build mode.",
      parameters: SKILL_CREATOR_PARAMS as never,
      executionMode: "sequential",
      execute: (_toolCallId, params: unknown, signal) => execute(params, signal) as never,
    });
    const restore = (ctx: { sessionManager: { getBranch(): unknown[] } }) => {
      state = restoreSkillCreatorState(ctx.sessionManager.getBranch());
      emit();
    };
    bound.on("session_start", (_event, ctx) => restore(ctx));
    bound.on("session_tree", (_event, ctx) => restore(ctx));
    bound.on("session_compact", (_event, ctx) => restore(ctx));
  };

  const controller: SkillCreatorController = {
    getState: () => state,
    inactiveTools() {
      return deps.commandEnabled() ? [] : [SKILL_CREATOR_TOOL_NAME];
    },
    setCommandRun(active) {
      commandRunActive = active;
    },
  };

  return { factory, controller };
}
