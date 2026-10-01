import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const cleanup: string[] = [];
afterEach(async () => {
  while (cleanup.length) await rm(cleanup.pop() as string, { recursive: true, force: true });
});

async function relist(providers: { providerId: string; authPath: string }[]): Promise<Record<string, { id: string; thinkingLevels: string[] }[]>> {
  const child = spawn(process.execPath, [resolve("dist/subscription-models.js")], { env: { PATH: process.env.PATH }, stdio: ["pipe", "pipe", "ignore"] });
  const chunks: Buffer[] = [];
  child.stdout.on("data", (chunk: Buffer) => chunks.push(chunk));
  const exited = new Promise<number | null>((resolvePromise) => child.once("exit", resolvePromise));
  child.stdin.end(JSON.stringify({ providers }));
  expect(await exited).toBe(0);
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

describe("subscription model re-list (launch)", () => {
  it("lists each signed-in account's models offline, without a lock or touching an expired token", async () => {
    const root = await mkdtemp(join(tmpdir(), "wackcode-subscription-models-"));
    cleanup.push(root);
    const codex = join(root, "codex.json");
    const copilot = join(root, "copilot.json");
    const torn = join(root, "torn.json");
    const codexAuth = JSON.stringify({ "openai-codex": { type: "oauth", access: "access-secret", refresh: "refresh-secret", expires: 0 } });
    await writeFile(codex, codexAuth, { mode: 0o600 });
    await writeFile(copilot, JSON.stringify({ "github-copilot": { type: "oauth", access: "a", refresh: "r", expires: 0, availableModelIds: ["not-a-real-model"] } }), { mode: 0o600 });
    await writeFile(torn, codexAuth.slice(0, 20), { mode: 0o600 });
    // A killed process's lock. Pi's own store would wait 30 s for it to go stale.
    await mkdir(`${codex}.lock`);

    const listed = await relist([
      { providerId: "openai-codex", authPath: codex },
      { providerId: "github-copilot", authPath: copilot },
      { providerId: "openrouter", authPath: codex },
      // Caught mid-write: left out, so Rust keeps the last list rather than storing none.
      { providerId: "xai", authPath: torn }
    ]);

    expect(Object.keys(listed).sort()).toEqual(["github-copilot", "openai-codex"]);
    expect(listed["openai-codex"].length).toBeGreaterThan(0);
    expect(listed["openai-codex"].every((model) => model.id && model.thinkingLevels.length > 0)).toBe(true);
    // Copilot lists only what the account's credential enables.
    expect(listed["github-copilot"]).toEqual([]);
    expect(JSON.stringify(listed)).not.toContain("secret");
    expect(await readFile(codex, "utf8")).toBe(codexAuth);
  });
});
