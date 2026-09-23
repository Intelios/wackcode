import { lstat, readFile } from "node:fs/promises";
import { join } from "node:path";

export interface InitAgentsTarget {
  path: string;
  before: string | undefined;
  prompt: string;
}

async function readAgentsFile(path: string): Promise<string | undefined> {
  const details = await lstat(path).catch((error: NodeJS.ErrnoException) => {
    if (error.code === "ENOENT") return undefined;
    throw error;
  });
  if (!details) return undefined;
  if (!details.isFile()) throw new Error("AGENTS.md must be a regular file, not a folder or symlink.");
  return readFile(path, "utf8");
}

export async function prepareInitAgents(cwd: string): Promise<InitAgentsTarget> {
  const override = await lstat(join(cwd, "AGENTS.override.md")).catch((error: NodeJS.ErrnoException) => {
    if (error.code === "ENOENT") return undefined;
    throw error;
  });
  if (override) {
    throw new Error("This workspace has AGENTS.override.md. Pi would ignore AGENTS.md here; edit or remove the override first.");
  }
  const path = join(cwd, "AGENTS.md");
  const before = await readAgentsFile(path);
  const claude = await readFile(join(cwd, "CLAUDE.md"), "utf8").then(() => true, (error: NodeJS.ErrnoException) => {
    if (error.code === "ENOENT") return false;
    throw error;
  });
  const prompt = `Initialize project instructions in ./AGENTS.md at this workspace root. This is a focused documentation task: change no other files.

Inspect the repository before writing. Read the existing AGENTS.md if present, ${claude ? "and read CLAUDE.md because Pi will prefer the new AGENTS.md over it, " : ""}then consult only the relevant README, manifests, CI configuration, and source files needed to verify useful guidance. Include project-specific setup and test commands, non-obvious conventions or constraints, and short pointers to deeper documentation. Verify every factual claim against the repository. Do not copy large sections of existing docs, list the whole tree, include secrets, or add generic coding advice. Aim for well under 100 lines; a small project may need only a few bullets. Omit sections with no useful verified guidance.

${before === undefined ? "Create AGENTS.md only if there is defensible project-specific guidance. If there is none, leave it absent and explain why." : "Conservatively refine the existing AGENTS.md: preserve useful intentional instructions, correct unsupported or stale claims, and trim clear redundancy. If it is already sufficient, leave it unchanged."} ${claude ? "Carry over applicable guidance from CLAUDE.md without editing that file. " : ""}Do not delete AGENTS.md. When finished, briefly state whether you created, updated, or left it unchanged and why.`;
  return { path, before, prompt };
}

export async function inspectInitAgentsResult(target: InitAgentsTarget): Promise<"created" | "updated" | "unchanged" | "absent"> {
  const after = await readAgentsFile(target.path);
  if (after === undefined) {
    if (target.before !== undefined) throw new Error("AGENTS.md was removed during /init. Restore it from the chat checkpoint before continuing.");
    return "absent";
  }
  if (!after.trim()) throw new Error("AGENTS.md is empty after /init. Add verified project guidance or restore the previous version.");
  if (target.before === undefined) return "created";
  return after === target.before ? "unchanged" : "updated";
}
