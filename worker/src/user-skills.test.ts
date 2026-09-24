import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as pi from "@earendil-works/pi-coding-agent";
import { afterEach, describe, expect, it } from "vitest";
import { loadUserSkills, mergeSkills, scanSkills } from "./user-skills.js";

const cleanup: string[] = [];
afterEach(async () => {
  while (cleanup.length) await rm(cleanup.pop() as string, { recursive: true, force: true });
});

async function tempDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "wackcode-user-skills-"));
  cleanup.push(dir);
  return dir;
}

async function skill(root: string, folder: string, name: string, description: string, extra = ""): Promise<string> {
  await mkdir(join(root, folder), { recursive: true });
  const file = join(root, folder, "SKILL.md");
  await writeFile(file, `---\nname: ${name}\ndescription: ${description}\n${extra}---\nBody of ${name}.\n`);
  return file;
}

describe("loadUserSkills", () => {
  it("reads roots in order, labels each skill by its folder, and keeps the first of a name", async () => {
    const base = await tempDir();
    const mine = join(base, "mine");
    const claude = join(base, "claude");
    await skill(mine, "alpha", "alpha", "Mine.");
    await skill(claude, "alpha", "alpha", "Theirs.");
    await skill(claude, "beta", "beta", "Only theirs.", "disable-model-invocation: true\n");

    const result = loadUserSkills(pi, { roots: [{ path: mine, label: "Your skills" }, { path: claude, label: "Claude Code" }], disabled: [] });
    expect(result.skills.map((entry) => [entry.name, entry.description, entry.sourceInfo.source])).toEqual([
      ["alpha", "Mine.", "Your skills"],
      ["beta", "Only theirs.", "Claude Code"]
    ]);
    expect(result.skills[1].disableModelInvocation).toBe(true);
    expect(result.diagnostics).toEqual([expect.objectContaining({ type: "collision", path: join(claude, "alpha", "SKILL.md") })]);
  });

  it("drops a switched-off skill before names are compared, so the next one of that name loads", async () => {
    const base = await tempDir();
    const mine = join(base, "mine");
    const claude = join(base, "claude");
    const off = await skill(mine, "alpha", "alpha", "Mine.");
    await skill(claude, "alpha", "alpha", "Theirs.");
    const result = loadUserSkills(pi, { roots: [{ path: mine, label: "Your skills" }, { path: claude, label: "Claude Code" }], disabled: [off] });
    expect(result.skills.map((entry) => [entry.name, entry.description])).toEqual([["alpha", "Theirs."]]);
    expect(result.diagnostics).toEqual([]);
  });

  it("loads a file reached through a symlink from a second folder only once", async () => {
    const base = await tempDir();
    const mine = join(base, "mine");
    const claude = join(base, "claude");
    await skill(mine, "shared", "shared", "Shared.");
    await mkdir(claude);
    await symlink(join(mine, "shared"), join(claude, "shared"));
    const result = loadUserSkills(pi, { roots: [{ path: mine, label: "Your skills" }, { path: claude, label: "Claude Code" }], disabled: [] });
    expect(result.skills).toHaveLength(1);
    expect(result.diagnostics).toEqual([]);
  });

  it("is empty without a payload, and a missing folder is not an error", async () => {
    expect(loadUserSkills(pi, undefined).skills).toEqual([]);
    const missing = loadUserSkills(pi, { roots: [{ path: join(await tempDir(), "absent"), label: "Codex" }], disabled: [] });
    expect(missing).toEqual({ skills: [], diagnostics: [] });
  });
});

describe("mergeSkills", () => {
  it("puts the user's skills first and drops a package skill with a taken name", async () => {
    const base = await tempDir();
    const mine = join(base, "mine");
    const pkg = join(base, "pkg", "skills");
    await skill(mine, "review", "review", "My review.");
    await skill(pkg, "review", "review", "Package review.");
    await skill(pkg, "lint", "lint", "Package lint.");
    const user = loadUserSkills(pi, { roots: [{ path: mine, label: "Your skills" }], disabled: [] });
    const packages = pi.loadSkills({ cwd: base, agentDir: base, skillPaths: [pkg], includeDefaults: false });
    const merged = mergeSkills(user, packages);
    expect(merged.skills.map((entry) => [entry.name, entry.description])).toEqual([["review", "My review."], ["lint", "Package lint."]]);
    expect(merged.diagnostics).toEqual([expect.objectContaining({ type: "collision", path: join(pkg, "review", "SKILL.md") })]);
    // No user skills: the loader's own result passes through untouched.
    expect(mergeSkills({ skills: [], diagnostics: [] }, packages)).toBe(packages);
  });
});

describe("scanSkills", () => {
  it("lists every folder, switched on or not, marking what a clash hides behind", async () => {
    const base = await tempDir();
    const mine = join(base, "mine");
    const claude = join(base, "claude");
    const codex = join(base, "codex");
    const pkg = join(base, "pkg", "skills");
    const offFile = await skill(mine, "gamma", "gamma", "Switched off.");
    await skill(mine, "alpha", "alpha", "Mine.");
    await skill(claude, "alpha", "alpha", "Theirs.");
    await skill(codex, "alpha", "alpha", "Codex's, folder off.");
    await skill(pkg, "alpha", "alpha", "Package alpha.");
    await skill(pkg, "delta", "delta", "Package delta.");
    await mkdir(join(mine, "broken"));
    await writeFile(join(mine, "broken", "SKILL.md"), "---\nname: broken\n---\nNo description.\n");

    const result = scanSkills(pi, {
      folders: [
        { id: "library", path: mine, label: "Your skills", enabled: true },
        { id: "claude", path: claude, label: "Claude Code", enabled: true },
        { id: "codex", path: codex, label: "Codex", enabled: false }
      ],
      packages: [{
        source: "npm:pkg",
        label: "pkg",
        resources: [
          { path: join(pkg, "alpha"), name: "skills/alpha", enabled: true },
          { path: join(pkg, "delta"), name: "skills/delta", enabled: false }
        ]
      }],
      disabled: [offFile]
    }, base);

    const byName = (group: { skills: Array<{ name: string }> }) => Object.fromEntries(group.skills.map((entry) => [entry.name, entry]));
    const library = byName(result.folders[0]);
    expect(library.alpha).toEqual(expect.not.objectContaining({ shadowedBy: expect.anything() }));
    expect(library.gamma).toEqual(expect.not.objectContaining({ shadowedBy: expect.anything() }));
    expect(result.folders[0].diagnostics).toEqual([expect.objectContaining({ type: "warning", path: join(mine, "broken", "SKILL.md") })]);
    expect(byName(result.folders[1]).alpha).toEqual(expect.objectContaining({ shadowedBy: "Your skills" }));
    // A folder that is off loads nothing, so nothing in it is "hidden".
    expect(byName(result.folders[2]).alpha).toEqual(expect.not.objectContaining({ shadowedBy: expect.anything() }));
    const packaged = byName(result.packages[0]);
    expect(packaged.alpha).toEqual(expect.objectContaining({ resourceName: "skills/alpha", shadowedBy: "Your skills" }));
    expect(packaged.delta).toEqual(expect.objectContaining({ resourceName: "skills/delta" }));
    expect(packaged.delta).toEqual(expect.not.objectContaining({ shadowedBy: expect.anything() }));
  });

  it("reads a single .md file for an import", async () => {
    const base = await tempDir();
    const file = join(base, "loose.md");
    await writeFile(file, "---\nname: loose\ndescription: A loose skill.\n---\nBody.\n");
    const result = scanSkills(pi, { folders: [{ id: "import", path: file, label: "Import", enabled: false }], packages: [], disabled: [] }, base);
    expect(result.folders[0].skills).toEqual([expect.objectContaining({ name: "loose", filePath: file })]);
  });
});
