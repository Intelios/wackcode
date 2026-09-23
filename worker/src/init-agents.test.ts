import { mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { inspectInitAgentsResult, prepareInitAgents } from "./init-agents.js";

const folders: string[] = [];
afterEach(async () => {
  for (const folder of folders.splice(0)) await rm(folder, { recursive: true, force: true });
});

async function workspace(): Promise<string> {
  const folder = await mkdtemp(join(tmpdir(), "wackcode-init-"));
  folders.push(folder);
  return folder;
}

describe("/init project instructions", () => {
  it("asks for verified, concise guidance and reports a missing result", async () => {
    const cwd = await workspace();
    const target = await prepareInitAgents(cwd);
    expect(target.prompt).toContain("Verify every factual claim");
    expect(target.prompt).toContain("change no other files");
    expect(target.prompt).toContain("leave it absent and explain why");
    expect(await inspectInitAgentsResult(target)).toBe("absent");
    await writeFile(target.path, "# Project\n\nRun `pnpm test`.\n");
    expect(await inspectInitAgentsResult(target)).toBe("created");
  });

  it("preserves an existing file and accounts for CLAUDE.md precedence", async () => {
    const cwd = await workspace();
    await writeFile(join(cwd, "AGENTS.md"), "# Keep this rule\n");
    await writeFile(join(cwd, "CLAUDE.md"), "# Existing context\n");
    const target = await prepareInitAgents(cwd);
    expect(target.prompt).toContain("Conservatively refine the existing AGENTS.md");
    expect(target.prompt).toContain("Carry over applicable guidance from CLAUDE.md");
    expect(await inspectInitAgentsResult(target)).toBe("unchanged");
    await writeFile(target.path, "# Keep this rule\n\nRun `pnpm test`.\n");
    expect(await inspectInitAgentsResult(target)).toBe("updated");
    expect(await readFile(join(cwd, "CLAUDE.md"), "utf8")).toBe("# Existing context\n");
  });

  it("refuses a shadowing override, a symlink, and an empty result", async () => {
    const cwd = await workspace();
    await writeFile(join(cwd, "AGENTS.override.md"), "# Override\n");
    await expect(prepareInitAgents(cwd)).rejects.toThrow("Pi would ignore AGENTS.md");
    await rm(join(cwd, "AGENTS.override.md"));
    await writeFile(join(cwd, "elsewhere.md"), "# Elsewhere\n");
    await symlink(join(cwd, "elsewhere.md"), join(cwd, "AGENTS.md"));
    await expect(prepareInitAgents(cwd)).rejects.toThrow("regular file");
    await rm(join(cwd, "AGENTS.md"));
    const target = await prepareInitAgents(cwd);
    await writeFile(target.path, " \n");
    await expect(inspectInitAgentsResult(target)).rejects.toThrow("empty");
  });
});
