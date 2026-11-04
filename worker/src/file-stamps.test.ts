import { mkdir, mkdtemp, rm, symlink, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as pi from "@earendil-works/pi-coding-agent";
import { afterEach, describe, expect, it, vi } from "vitest";
import { dirStamp, fileStamp, skillsTreeStamp } from "./file-stamps.js";
import { loadUserCommands } from "./user-commands.js";
import { argumentHint, loadUserSkills } from "./user-skills.js";

const cleanup: string[] = [];
afterEach(async () => {
  while (cleanup.length) await rm(cleanup.pop() as string, { recursive: true, force: true });
});

async function tempDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "wackcode-stamps-"));
  cleanup.push(dir);
  return dir;
}

/** Same content, guaranteed-newer mtime. */
async function touch(path: string): Promise<void> {
  const at = new Date(Date.now() + 500);
  await utimes(path, at, at);
}

describe("file stamps", () => {
  it("move when a file changes, stays when nothing does", async () => {
    const dir = await tempDir();
    const file = join(dir, "a.md");
    await writeFile(file, "one");
    const first = fileStamp(file);
    expect(fileStamp(file)).toBe(first);
    await writeFile(file, "two!!");
    const resized = fileStamp(file);
    expect(resized).not.toBe(first);
    await rm(file);
    expect(fileStamp(file)).toBe("!");
  });

  it("a directory stamp covers adds, removes and edits one level deep", async () => {
    const dir = await tempDir();
    await writeFile(join(dir, "a.md"), "one");
    const first = dirStamp(dir);
    expect(dirStamp(dir)).toBe(first);
    await writeFile(join(dir, "b.md"), "two");
    expect(dirStamp(dir)).not.toBe(first);
    await rm(join(dir, "b.md"));
    expect(dirStamp(dir)).toBe(first);
    await writeFile(join(dir, "a.md"), "edited!");
    expect(dirStamp(dir)).not.toBe(first);
    expect(dirStamp(join(dir, "missing"))).toBe("!");
  });

  it("the skills tree stamp ignores node_modules and dot-folders but watches ignore files", async () => {
    const dir = await tempDir();
    await mkdir(join(dir, "skill"), { recursive: true });
    await writeFile(join(dir, "skill", "SKILL.md"), "---\nname: s\ndescription: d\n---\n");
    await mkdir(join(dir, "node_modules", "pkg"), { recursive: true });
    await writeFile(join(dir, "node_modules", "pkg", "x.js"), "x");
    await mkdir(join(dir, ".hidden"));
    await writeFile(join(dir, ".hidden", "y.md"), "y");
    const first = skillsTreeStamp(dir);
    await writeFile(join(dir, "node_modules", "pkg", "x.js"), "changed");
    await writeFile(join(dir, ".hidden", "y.md"), "changed");
    expect(skillsTreeStamp(dir)).toBe(first);
    await writeFile(join(dir, ".gitignore"), "ignored/\n");
    const withIgnore = skillsTreeStamp(dir);
    expect(withIgnore).not.toBe(first);
    await writeFile(join(dir, ".gitignore"), "ignored2/\n");
    expect(skillsTreeStamp(dir)).not.toBe(withIgnore);
    await writeFile(join(dir, "skill", "extra.md"), "a nested file still moves it");
    expect(skillsTreeStamp(dir)).not.toBe(withIgnore);
  });

  it("recurses through a symlinked folder like Pi's loader does", async () => {
    const dir = await tempDir();
    const real = join(dir, "real");
    await mkdir(real);
    await writeFile(join(real, "SKILL.md"), "---\nname: s\ndescription: d\n---\n");
    await symlink(real, join(dir, "linked"));
    const first = skillsTreeStamp(dir);
    await writeFile(join(real, "SKILL.md"), "---\nname: s\ndescription: edited\n---\n");
    expect(skillsTreeStamp(dir)).not.toBe(first);
  });
});

describe("loader caching", () => {
  it("loadUserSkills re-reads only after a stamp change", async () => {
    const dir = await tempDir();
    await mkdir(join(dir, "one"));
    await writeFile(join(dir, "one", "SKILL.md"), "---\nname: one\ndescription: First.\n---\nBody.\n");
    const payload = { roots: [{ path: dir, label: "Bench" }], disabled: [] };
    const loadSpy = vi.fn(pi.loadSkillsFromDir);
    const loaders = { ...pi, loadSkillsFromDir: loadSpy };
    const loaded = loadUserSkills(loaders, payload);
    const calls = loadSpy.mock.calls.length;
    expect(loadUserSkills(loaders, payload)).toBe(loaded);
    expect(loadSpy.mock.calls.length).toBe(calls);
    // A new folder joins the next load.
    await mkdir(join(dir, "two"));
    await writeFile(join(dir, "two", "SKILL.md"), "---\nname: two\ndescription: Second.\n---\n");
    expect(loadUserSkills(loaders, payload).skills.map((skill) => skill.name)).toEqual(["one", "two"]);
    const afterAdd = loadSpy.mock.calls.length;
    // mtime-only churn still reloads, keeping edits visible.
    await touch(join(dir, "two", "SKILL.md"));
    loadUserSkills(loaders, payload);
    expect(loadSpy.mock.calls.length).toBeGreaterThan(afterAdd);
  });

  it("loadUserCommands serves the same folder once until it moves", async () => {
    const dir = await tempDir();
    await writeFile(join(dir, "a.md"), "---\ndescription: A\n---\nDo a.\n");
    const parseSpy = vi.fn(pi.parseFrontmatter);
    const loaders = { parseFrontmatter: parseSpy };
    const loaded = loadUserCommands(loaders, dir);
    const calls = parseSpy.mock.calls.length;
    expect(loadUserCommands(loaders, dir)).toBe(loaded);
    expect(parseSpy.mock.calls.length).toBe(calls);
    await writeFile(join(dir, "b.md"), "---\ndescription: B\n---\nDo b.\n");
    expect(loadUserCommands(loaders, dir).map((template) => template.name)).toEqual(["a", "b"]);
  });

  it("argumentHint reads a file once per stamp", async () => {
    const dir = await tempDir();
    const skillFile = join(dir, "SKILL.md");
    await writeFile(skillFile, "---\nname: s\ndescription: d\nargument-hint: <files>\n---\n");
    const parseSpy = vi.fn(pi.parseFrontmatter);
    const skill = { name: "s", filePath: skillFile } as Parameters<typeof argumentHint>[0];
    expect(argumentHint(skill, parseSpy)).toBe("<files>");
    const calls = parseSpy.mock.calls.length;
    expect(argumentHint(skill, parseSpy)).toBe("<files>");
    expect(parseSpy.mock.calls.length).toBe(calls);
    await writeFile(skillFile, "---\nname: s\ndescription: d\nargument-hint: [maybe]\n---\n");
    expect(argumentHint(skill, parseSpy)).toBe("[maybe]");
  });
});
