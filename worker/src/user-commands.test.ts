import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as pi from "@earendil-works/pi-coding-agent";
import { afterEach, describe, expect, it } from "vitest";
import { commandsSignature, loadUserCommands, mergePrompts } from "./user-commands.js";

const cleanup: string[] = [];
afterEach(async () => {
  while (cleanup.length) await rm(cleanup.pop() as string, { recursive: true, force: true });
});

async function tempDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "wackcode-user-commands-"));
  cleanup.push(dir);
  return dir;
}

describe("loadUserCommands", () => {
  it("reads .md files as templates, Pi-style: filename, frontmatter, body fallback", async () => {
    const dir = await tempDir();
    await writeFile(join(dir, "review.md"), "---\ndescription: Review the work\nargument-hint: <files>\n---\nReview $ARGUMENTS\n");
    await writeFile(join(dir, "plain.md"), "Summarize the diff.\n");
    await writeFile(join(dir, ".hidden.md"), "never\n");
    await writeFile(join(dir, "notes.txt"), "not a command\n");

    const templates = loadUserCommands(pi, dir);
    expect(templates.map((entry) => entry.name)).toEqual(["plain", "review"]);
    const review = templates[1];
    expect(review.description).toBe("Review the work");
    expect(review.argumentHint).toBe("<files>");
    expect(review.content).toContain("Review $ARGUMENTS");
    expect(review.filePath).toBe(join(dir, "review.md"));
    expect(review.sourceInfo.source).toBe("Your commands");
    // No description: the first body line stands in, like Pi's own loader.
    expect(templates[0].description).toBe("Summarize the diff.");
  });

  it("returns nothing for a missing or empty folder", async () => {
    expect(loadUserCommands(pi, join(tmpdir(), "wackcode-no-such-commands-dir"))).toEqual([]);
    expect(loadUserCommands(pi, undefined)).toEqual([]);
  });
});

describe("mergePrompts", () => {
  it("puts the user's commands ahead of package templates of the same name", () => {
    const user = [{ name: "review" }, { name: "mine" }] as never[];
    const packages = [{ name: "review" }, { name: "theirs" }] as never[];
    expect(mergePrompts(user, packages).map((template) => template.name)).toEqual(["review", "mine", "review", "theirs"]);
    expect(mergePrompts([], packages)).toBe(packages);
  });
});

describe("commandsSignature", () => {
  it("changes when the folder contents or the denylist do", () => {
    const dir = "unused";
    const a = loadUserCommands(pi, dir);
    const base = commandsSignature(a, []);
    expect(commandsSignature(a, [])).toBe(base);
    expect(commandsSignature(a, ["custom:/x"])).not.toBe(base);
    const edited = [{ name: "n", description: "d", content: "body", filePath: "/f", sourceInfo: {} }] as never[];
    expect(commandsSignature(edited, [])).not.toBe(base);
  });
});
