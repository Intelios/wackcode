import { mkdir, mkdtemp, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as pi from "@earendil-works/pi-coding-agent";
import { afterEach, describe, expect, it, vi } from "vitest";
import { fileStamp } from "./file-stamps.js";
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
});

describe("loader caching", () => {
  it("loadUserSkills re-reads only after the stamped tree moves", async () => {
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
    // A new top-level folder moves the stamp immediately.
    await mkdir(join(dir, "two"));
    await writeFile(join(dir, "two", "SKILL.md"), "---\nname: two\ndescription: Second.\n---\n");
    expect(loadUserSkills(loaders, payload).skills.map((skill) => skill.name)).toEqual(["one", "two"]);
    const afterAdd = loadSpy.mock.calls.length;
    // An edit to a known skill file moves it too.
    await touch(join(dir, "two", "SKILL.md"));
    expect(loadUserSkills(loaders, payload).skills.find((skill) => skill.name === "two")?.description).toBeDefined();
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
