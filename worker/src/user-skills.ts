/**
 * The user's own skill folders (Settings › Skills), layered over the package skills Pi's resource
 * loader already holds.
 *
 * Pi's own discovery stays off (`noSkills`), so these folders load only because the host names
 * them: `~/.agents/skills`, then any other tool's folder the user switched on. A project's own
 * `.agents/skills` is never among them. The worker and Settings' scan both go through here, so
 * Settings shows exactly what a chat loads:
 *
 * - Roots are read in order and the first skill with a name wins, like Pi's own `loadSkills`.
 * - A switched-off skill is dropped before names are compared, so switching one off lets the
 *   next skill with that name load instead of leaving a gap.
 * - The same file reached twice (a symlink between two tools' folders) loads once.
 * - A user skill beats a package skill with the same name.
 */
import { readFileSync, realpathSync, statSync } from "node:fs";
import type { LoadSkillsResult, ResourceDiagnostic, Skill } from "@earendil-works/pi-coding-agent";
import type { ScannedDiagnostic, ScannedGroup, ScannedSkill, SkillScanRequest, UserSkillsPayload } from "./protocol.js";

type PiSkillLoaders = Pick<typeof import("@earendil-works/pi-coding-agent"), "loadSkills" | "loadSkillsFromDir" | "parseFrontmatter">;
type ParseFrontmatter = PiSkillLoaders["parseFrontmatter"];

const MAX_SKILLS_PER_GROUP = 500;
const MAX_DESCRIPTION_CHARS = 1_024;
const MAX_DIAGNOSTICS_PER_GROUP = 50;
const MAX_MESSAGE_CHARS = 300;

export const NO_USER_SKILLS: LoadSkillsResult = { skills: [], diagnostics: [] };

function realPath(path: string): string {
  try {
    return realpathSync(path);
  } catch {
    return path;
  }
}

function collision(winner: Skill, loser: Skill): ResourceDiagnostic {
  return {
    type: "collision",
    message: `name "${loser.name}" collision`,
    path: loser.filePath,
    collision: { resourceType: "skill", name: loser.name, winnerPath: winner.filePath, loserPath: loser.filePath }
  };
}

/** What a chat loads from the user's folders. Each skill's source is its folder's label. */
export function loadUserSkills(pi: PiSkillLoaders, payload: UserSkillsPayload | undefined): LoadSkillsResult {
  if (!payload || payload.roots.length === 0) return NO_USER_SKILLS;
  const disabled = new Set(payload.disabled);
  const byName = new Map<string, Skill>();
  const seen = new Set<string>();
  const skills: Skill[] = [];
  const diagnostics: ResourceDiagnostic[] = [];
  for (const root of payload.roots) {
    const loaded = pi.loadSkillsFromDir({ dir: root.path, source: root.label });
    diagnostics.push(...loaded.diagnostics);
    for (const skill of loaded.skills) {
      if (disabled.has(skill.filePath)) continue;
      const real = realPath(skill.filePath);
      if (seen.has(real)) continue;
      const winner = byName.get(skill.name);
      if (winner) {
        diagnostics.push(collision(winner, skill));
        continue;
      }
      byName.set(skill.name, skill);
      seen.add(real);
      skills.push(skill);
    }
  }
  return { skills, diagnostics };
}

/** The user's skills first, then every package skill whose name and file they don't already hold. */
export function mergeSkills(user: LoadSkillsResult, packages: LoadSkillsResult): LoadSkillsResult {
  if (user.skills.length === 0 && user.diagnostics.length === 0) return packages;
  const byName = new Map(user.skills.map((skill) => [skill.name, skill]));
  const seen = new Set(user.skills.map((skill) => realPath(skill.filePath)));
  const skills = [...user.skills];
  const diagnostics = [...user.diagnostics, ...packages.diagnostics];
  for (const skill of packages.skills) {
    if (seen.has(realPath(skill.filePath))) continue;
    const winner = byName.get(skill.name);
    if (winner) {
      diagnostics.push(collision(winner, skill));
      continue;
    }
    skills.push(skill);
  }
  return { skills, diagnostics };
}

/** A cheap fingerprint of what the system prompt and `/` menu show, to skip needless rebuilds. */
export function skillsSignature(result: LoadSkillsResult): string {
  return JSON.stringify(result.skills.map((skill) => [skill.name, skill.description, skill.filePath, skill.disableModelInvocation, skill.sourceInfo.source]));
}

function scanned(skill: Skill, extra: Partial<ScannedSkill> = {}, parseFrontmatter?: ParseFrontmatter): ScannedSkill {
  const hint = parseFrontmatter ? argumentHint(skill, parseFrontmatter) : undefined;
  return {
    name: skill.name,
    description: skill.description.slice(0, MAX_DESCRIPTION_CHARS),
    filePath: skill.filePath,
    baseDir: skill.baseDir,
    manual: skill.disableModelInvocation,
    ...(hint ? { argumentHint: hint } : {}),
    ...extra
  };
}

/**
 * A skill's optional `argument-hint` frontmatter, shown in the picker and the composer's inline
 * hint (`<arg>` required, `[arg]` optional). Pi's loader keeps it off `Skill`, so the file's
 * frontmatter is read again — the same key Pi reads for prompt templates. Display only: never
 * part of `skillsSignature`, so editing it doesn't churn system-prompt rebuilds.
 */
export function argumentHint(skill: Skill, parseFrontmatter: ParseFrontmatter): string | undefined {
  try {
    const { frontmatter } = parseFrontmatter(readFileSync(skill.filePath, "utf-8"));
    const value = frontmatter["argument-hint"];
    // An unquoted `[files]` parses as a YAML flow sequence; rebuilt as "[files]" so a hand-edited
    // optional hint reads right. Quoted scalars (what the editor writes) arrive as strings.
    const hint = Array.isArray(value)
      ? value.every((item) => typeof item === "string" || typeof item === "number") ? `[${value.join(" ")}]` : undefined
      : typeof value === "string" ? value : undefined;
    if (hint && hint.trim()) return hint.trim().slice(0, MAX_DESCRIPTION_CHARS);
  } catch {
    // An unreadable file just never shows a hint.
  }
  return undefined;
}

function diagnosticsOf(entries: ResourceDiagnostic[]): ScannedDiagnostic[] {
  // Clashes are shown on the losing skill itself (`shadowedBy`), not as a warning.
  return entries
    .filter((entry) => entry.type !== "collision")
    .slice(0, MAX_DIAGNOSTICS_PER_GROUP)
    .map((entry) => ({ type: entry.type, message: entry.message.slice(0, MAX_MESSAGE_CHARS), ...(entry.path ? { path: entry.path } : {}) }));
}

/**
 * Settings' view of every folder (switched on or not) and every trusted package's skills, each
 * skill marked with what beats it when it loses a name clash in the chat's merged set.
 */
export function scanSkills(pi: PiSkillLoaders, request: SkillScanRequest, agentDir: string): { folders: ScannedGroup[]; packages: ScannedGroup[] } {
  const loadPath = (path: string) => pi.loadSkills({ cwd: agentDir, agentDir, skillPaths: [path], includeDefaults: false });

  // What a chat loads, built exactly as the worker builds it.
  const user = loadUserSkills(pi, { roots: request.folders.filter((folder) => folder.enabled), disabled: request.disabled });
  const packageLabels = new Map<string, string>();
  const enabledPackagePaths: string[] = [];
  for (const entry of request.packages) {
    for (const resource of entry.resources) {
      if (!resource.enabled) continue;
      enabledPackagePaths.push(resource.path);
      packageLabels.set(resource.path, entry.label);
    }
  }
  const packaged = enabledPackagePaths.length
    ? pi.loadSkills({ cwd: agentDir, agentDir, skillPaths: enabledPackagePaths, includeDefaults: false })
    : NO_USER_SKILLS;
  const merged = mergeSkills(user, packaged);
  const winners = new Map(merged.skills.map((skill) => [skill.name, skill]));
  const userLabels = new Map(user.skills.map((skill) => [skill.filePath, skill.sourceInfo.source]));
  const packageLabelOf = (skill: Skill) => {
    for (const [path, label] of packageLabels) {
      if (skill.filePath === path || skill.filePath.startsWith(`${path}/`)) return label;
    }
    return skill.sourceInfo.source;
  };
  const labelOf = (skill: Skill) => userLabels.get(skill.filePath) ?? packageLabelOf(skill);
  const shadowedBy = (skill: Skill): string | undefined => {
    const winner = winners.get(skill.name);
    return winner && winner.filePath !== skill.filePath ? labelOf(winner) : undefined;
  };

  const disabled = new Set(request.disabled);
  const isFile = (path: string) => {
    try {
      return statSync(path).isFile();
    } catch {
      return false;
    }
  };
  const folders = request.folders.map((folder): ScannedGroup => {
    // An import can name a single `.md` file rather than a folder.
    const loaded = isFile(folder.path) ? loadPath(folder.path) : pi.loadSkillsFromDir({ dir: folder.path, source: folder.label });
    return {
      id: folder.id,
      skills: loaded.skills.slice(0, MAX_SKILLS_PER_GROUP).map((skill) => scanned(skill, folder.enabled && !disabled.has(skill.filePath) ? { shadowedBy: shadowedBy(skill) } : {}, pi.parseFrontmatter)),
      diagnostics: diagnosticsOf(loaded.diagnostics)
    };
  });

  const packages = request.packages.map((entry): ScannedGroup => {
    const skills: ScannedSkill[] = [];
    const diagnostics: ResourceDiagnostic[] = [];
    for (const resource of entry.resources) {
      const loaded = loadPath(resource.path);
      diagnostics.push(...loaded.diagnostics);
      for (const skill of loaded.skills) {
        if (skills.length >= MAX_SKILLS_PER_GROUP) break;
        skills.push(scanned(skill, { resourceName: resource.name, ...(resource.enabled ? { shadowedBy: shadowedBy(skill) } : {}) }, pi.parseFrontmatter));
      }
    }
    return { id: entry.source, skills, diagnostics: diagnosticsOf(diagnostics) };
  });

  return { folders, packages };
}
