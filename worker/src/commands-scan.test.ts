import { spawn } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { afterEach, describe, expect, it } from "vitest";
import type { CommandScanResult } from "./protocol.js";

const cleanup: string[] = [];
afterEach(async () => {
  while (cleanup.length) await rm(cleanup.pop() as string, { recursive: true, force: true });
});

async function scan(request: Record<string, unknown>): Promise<CommandScanResult> {
  const child = spawn(process.execPath, [resolve("dist/commands-scan.js")], { stdio: ["pipe", "pipe", "pipe"] });
  const lines = createInterface({ input: child.stdout });
  const answer = new Promise<CommandScanResult>((resolvePromise, reject) => {
    lines.on("line", (line) => resolvePromise(JSON.parse(line) as CommandScanResult));
    child.once("exit", () => reject(new Error("commands-scan exited without a result")));
  });
  child.stdin.write(`${JSON.stringify({ request, agentDir: join(tmpdir(), "wackcode-scan-agent") })}\n`);
  try {
    return await answer;
  } finally {
    lines.close();
    child.kill("SIGKILL");
  }
}

describe("commands scan (Settings › Commands)", () => {
  it("lists custom and package commands with the chat's resolved names", async () => {
    const root = await mkdtemp(join(tmpdir(), "wackcode-scan-"));
    cleanup.push(root);
    const packageDir = join(root, "package");
    const commandsDir = join(root, "commands");
    const skillsDir = join(root, "skills");
    const skillDir = join(skillsDir, "fixture-skill");
    await mkdir(packageDir, { recursive: true });
    await mkdir(commandsDir, { recursive: true });
    await mkdir(skillDir, { recursive: true });
    const extension = join(packageDir, "ext.ts");
    await writeFile(extension, `export default function (pi: any) {
      pi.registerCommand("hello", { description: "Says hello", handler: async () => undefined });
      pi.registerCommand("new", { description: "Package new", handler: async () => undefined });
      pi.registerCommand("goal", { description: "Package goal", handler: async () => undefined });
    }\n`);
    const prompt = join(packageDir, "review.md");
    await writeFile(prompt, "---\ndescription: Package review\nargument-hint: <diff>\n---\nReview $ARGUMENTS\n");
    const mine = join(commandsDir, "hello.md");
    await writeFile(mine, "---\ndescription: Mine\n---\nMy hello.\n");
    await writeFile(join(commandsDir, "mine.md"), "Just mine.\n");
    const skillFile = join(skillDir, "SKILL.md");
    await writeFile(skillFile, "---\nname: fixture-skill\ndescription: Fixture skill\nargument-hint: [context]\n---\n\nUse this skill.\n");

    const result = await scan({
      cwd: root,
      packages: [{ source: "npm:fixture", label: "Fixture", installedPath: packageDir, extensions: [{ path: extension, enabled: true }], skills: [], prompts: [{ path: prompt, enabled: true }] }],
      skillRoots: [{ path: skillsDir, label: "Your skills" }],
      skillDisabled: [],
      commandsDir,
      disabled: []
    });
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    // The extension keeps `hello` (extensions resolve first); the user's file gets custom:hello.
    const group = result.packages.find((entry) => entry.id === "npm:fixture");
    expect(group?.commands).toEqual(expect.arrayContaining([
      expect.objectContaining({ key: `extension:${extension}#hello`, name: "hello", kind: "extension", description: "Says hello", enabled: true }),
      expect.objectContaining({ key: `extension:${extension}#new`, name: "extension:new", rawName: "new", kind: "extension" }),
      expect.objectContaining({ key: `extension:${extension}#goal`, name: "extension:goal", rawName: "goal", kind: "extension" }),
      expect.objectContaining({ key: `prompt:${prompt}`, name: "review", kind: "prompt", argumentHint: "<diff>", enabled: true })
    ]));
    expect(result.custom).toEqual(expect.arrayContaining([
      expect.objectContaining({ key: `custom:${mine}`, name: "custom:hello", rawName: "hello", kind: "custom" }),
      expect.objectContaining({ kind: "custom", name: "mine" })
    ]));
    expect(result.catalog).toEqual(expect.arrayContaining([
      expect.objectContaining({ id: `extension:${extension}#hello`, name: "hello", source: "extension" }),
      expect.objectContaining({ id: `skill:${skillFile}`, name: "skill:fixture-skill", source: "skill", sourceLabel: "Your skills", argumentHint: "[context]" })
    ]));

    // A switched-off key frees its name for the command behind it, exactly like a chat.
    const disabled = await scan({
      cwd: root,
      packages: [{ source: "npm:fixture", label: "Fixture", installedPath: packageDir, extensions: [{ path: extension, enabled: true }], skills: [], prompts: [{ path: prompt, enabled: true }] }],
      skillRoots: [{ path: skillsDir, label: "Your skills" }],
      skillDisabled: [],
      commandsDir,
      disabled: [`extension:${extension}#hello`]
    });
    expect(disabled.ok).toBe(true);
    if (!disabled.ok) return;
    const off = disabled.packages[0].commands.find((entry) => entry.key === `extension:${extension}#hello`);
    expect(off).toEqual(expect.objectContaining({ enabled: false, name: "hello" }));
    expect(disabled.custom.find((entry) => entry.key === `custom:${mine}`)).toEqual(expect.objectContaining({ name: "hello" }));
  });
});
