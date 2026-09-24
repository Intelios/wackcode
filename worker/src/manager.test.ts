import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { afterEach, describe, expect, it } from "vitest";
import { FRAME, type ManagerOutput, type ManagerPackage } from "./manager-protocol.js";

class ManagerHarness {
  readonly child: ChildProcessWithoutNullStreams;
  readonly outputs: ManagerOutput[] = [];
  /** Unframed stdout lines, i.e. whatever npm printed. */
  readonly noise: string[] = [];
  stderr = "";
  private waiters: Array<() => void> = [];

  constructor() {
    this.child = spawn(process.execPath, [resolve("dist/manager.js")], { stdio: ["pipe", "pipe", "pipe"] });
    createInterface({ input: this.child.stdout }).on("line", (line) => {
      if (line.startsWith(FRAME)) this.outputs.push(JSON.parse(line.slice(FRAME.length)) as ManagerOutput);
      else this.noise.push(line);
      for (const notify of this.waiters.splice(0)) notify();
    });
    this.child.stderr.on("data", (chunk: Buffer) => { this.stderr += chunk.toString("utf8"); });
  }

  send(value: unknown): void {
    this.child.stdin.write(`${JSON.stringify(value)}\n`);
  }

  /** Run one command and return the catalog it produced, failing loudly on a refusal. */
  async run(command: Record<string, unknown>): Promise<ManagerPackage[]> {
    const id = crypto.randomUUID();
    const before = this.outputs.length;
    this.send({ ...command, id });
    const response = await this.waitFor((output) => output.type === "response" && output.id === id);
    if (response.type === "response" && !response.success) throw new Error(response.error);
    const catalog = this.outputs.slice(before).find((output) => output.type === "catalog");
    return catalog?.type === "catalog" ? catalog.packages : [];
  }

  async waitFor(predicate: (output: ManagerOutput) => boolean, timeoutMs = 30_000): Promise<ManagerOutput> {
    const started = Date.now();
    while (true) {
      const match = this.outputs.find(predicate);
      if (match) return match;
      if (Date.now() - started >= timeoutMs) {
        throw new Error(`Timed out. stderr: ${this.stderr}\nnoise: ${this.noise.join("\n")}\noutputs: ${JSON.stringify(this.outputs)}`);
      }
      await new Promise<void>((resolvePromise) => {
        const timer = setTimeout(resolvePromise, 50);
        this.waiters.push(() => { clearTimeout(timer); resolvePromise(); });
      });
    }
  }

  async shutdown(): Promise<void> {
    if (this.child.exitCode !== null) return;
    this.send({ id: crypto.randomUUID(), type: "shutdown" });
    await Promise.race([
      new Promise<void>((done) => this.child.once("exit", () => done())),
      new Promise<void>((done) => setTimeout(done, 1_500))
    ]);
    if (this.child.exitCode === null) this.child.kill("SIGKILL");
  }
}

/** A package with two extensions and one skill, plus a load-time console.log to pollute stdout. */
async function writeFixture(root: string): Promise<string> {
  const pkg = join(root, "fixture-pkg");
  await mkdir(join(pkg, "extensions"), { recursive: true });
  await mkdir(join(pkg, "skills"), { recursive: true });
  await writeFile(join(pkg, "package.json"), JSON.stringify({
    name: "fixture-pkg", version: "2.1.0", type: "module", keywords: ["pi-package"],
    pi: { extensions: ["./extensions"], skills: ["./skills"] }
  }));
  await writeFile(join(pkg, "extensions", "echo.ts"),
    `console.log("fixture package noise on stdout");\nexport default function (pi: any) { pi.registerTool({ name: "fixture_echo", label: "e", description: "d", parameters: { type: "object", properties: {} }, async execute() { return { content: [] }; } }); }\n`);
  await writeFile(join(pkg, "extensions", "legacy.ts"), `export default function () {}\n`);
  await writeFile(join(pkg, "skills", "review.md"), `---\nname: review\ndescription: Review code\n---\nReview it.\n`);
  return pkg;
}

const cleanup: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const action of cleanup.splice(0).reverse()) await action();
});

describe("Pi package manager process", () => {
  it("installs only a package's skills when Settings › Skills asks, leaving its code off", async () => {
    const root = await mkdtemp(join(tmpdir(), "wackcode-manager-skills-"));
    cleanup.push(() => rm(root, { recursive: true, force: true }));
    const piDir = join(root, "pi");
    await mkdir(piDir, { recursive: true });
    const fixture = await writeFixture(root);

    const manager = new ManagerHarness();
    cleanup.push(() => manager.shutdown());
    await manager.run({ type: "init", piDir });
    const installed = await manager.run({ type: "install", source: fixture, onlySkills: true });
    expect(installed[0].extensions.map((resource) => resource.enabled)).toEqual([false, false]);
    expect(installed[0].skills.map((resource) => [resource.name, resource.enabled])).toEqual([["skills/review.md", true]]);
  });

  it("installs a local package, reports its resources, and applies per-resource toggles", async () => {
    const root = await mkdtemp(join(tmpdir(), "wackcode-manager-"));
    const piDir = join(root, "pi");
    await mkdir(piDir, { recursive: true });
    const fixture = await writeFixture(root);

    const manager = new ManagerHarness();
    cleanup.push(() => manager.shutdown());
    await manager.run({ type: "init", piDir });

    expect(await manager.run({ type: "list" })).toEqual([]);

    const installed = await manager.run({ type: "install", source: fixture });
    expect(installed).toHaveLength(1);
    expect(installed[0].displayName).toBe("fixture-pkg");
    expect(installed[0].kind).toBe("local");
    expect(installed[0].version).toBe("2.1.0");
    expect(installed[0].extensions.map((resource) => resource.name)).toEqual(["extensions/echo.ts", "extensions/legacy.ts"]);
    expect(installed[0].skills.map((resource) => resource.name)).toEqual(["skills/review.md"]);
    // Everything a package declares is on until the user says otherwise.
    expect(installed[0].extensions.every((resource) => resource.enabled)).toBe(true);

    // Turning one extension off must leave the other on and must not touch skills.
    // The UI always round-trips the catalog's own source string; Pi may have rewritten it.
    const source = installed[0].source;
    const filtered = await manager.run({ type: "set_resources", source, extensions: ["extensions/echo.ts"] });
    expect(filtered[0].extensions.map((resource) => [resource.name, resource.enabled])).toEqual([
      ["extensions/echo.ts", true],
      ["extensions/legacy.ts", false]
    ]);
    expect(filtered[0].skills.every((resource) => resource.enabled)).toBe(true);

    // The toggle is persisted in Pi's own settings format, so the Pi CLI agrees with us.
    const settings = JSON.parse(await readFile(join(piDir, "settings.json"), "utf8")) as { packages: unknown[] };
    expect(settings.packages).toEqual([{ source, extensions: ["extensions/echo.ts"] }]);

    // An empty array means "none of this kind", which is different from omitting the key.
    // An absolute path still matches the stored relative form.
    const noExtensions = await manager.run({ type: "set_resources", source: fixture, extensions: [] });
    expect(noExtensions[0].extensions.every((resource) => resource.enabled)).toBe(false);

    expect(await manager.run({ type: "remove", source: fixture })).toEqual([]);
  }, 60_000);

  it("reports a refusal instead of crashing, and keeps stray stdout out of the protocol", async () => {
    const root = await mkdtemp(join(tmpdir(), "wackcode-manager-"));
    const piDir = join(root, "pi");
    await mkdir(piDir, { recursive: true });

    const manager = new ManagerHarness();
    cleanup.push(() => manager.shutdown());
    await manager.run({ type: "init", piDir });

    await expect(manager.run({ type: "set_resources", source: "npm:not-installed", extensions: [] }))
      .rejects.toThrow(/not installed/);
    // The process survives a refusal and still answers.
    expect(await manager.run({ type: "list" })).toEqual([]);
    expect(manager.child.exitCode).toBeNull();
  }, 30_000);
});
