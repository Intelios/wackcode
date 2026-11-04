import { mkdtemp, readFile, rm, stat, symlink, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, it } from "vitest";
import { cataloguePath, readCatalogue, writeCatalogue } from "./cache.js";

const directories: string[] = [];
afterEach(async () => { await Promise.all(directories.splice(0).map((directory) => rm(directory, { recursive: true, force: true }))); });
async function fixture() {
  const directory = await mkdtemp(join(tmpdir(), "wc-mcp-catalogue-"));
  directories.push(directory);
  return { directory, path: cataloguePath(directory, "connection-with-secret") };
}
const tools = [{ name: "peek", description: "Read a value.", inputSchema: { type: "object" as const, properties: {} } }];

it("shares complete, private catalogues without persisting connection details", async () => {
  const { directory, path } = await fixture();
  await Promise.all([writeCatalogue(directory, path, tools, (text) => text), writeCatalogue(directory, path, tools, (text) => text)]);
  expect(await readCatalogue(path)).toEqual(tools);
  expect(path).not.toContain("connection-with-secret");
  expect(await readFile(path, "utf8")).not.toContain("connection-with-secret");
  expect((await stat(path)).mode & 0o777).toBe(0o600);
  await writeCatalogue(directory, path, [], (text) => text);
  expect(await readCatalogue(path)).toEqual([]);
});

it("does not persist schemas containing credentials, even in nested schema fields", async () => {
  const { directory, path } = await fixture();
  const privateTools = [{ ...tools[0], inputSchema: { type: "object" as const, properties: { token: { default: "a-secret-value" } } } }];
  await writeCatalogue(directory, path, privateTools, (text) => text.replaceAll("a-secret-value", "[credential redacted]"));
  expect(await readCatalogue(path)).toBeUndefined();
});

it("treats missing, damaged, old, oversized and symlinked cache files as misses", async () => {
  const { path, directory } = await fixture();
  expect(await readCatalogue(path)).toBeUndefined();
  for (const contents of ["bad json", "{}", '[{"name":"bad"}]', " ".repeat(4 * 1024 * 1024 + 1)]) {
    await writeFile(path, contents);
    expect(await readCatalogue(path)).toBeUndefined();
  }
  await writeCatalogue(directory, path, tools, (text) => text);
  const old = new Date(Date.now() - 8 * 24 * 60 * 60_000);
  await utimes(path, old, old);
  expect(await readCatalogue(path)).toBeUndefined();
  const link = join(directory, "link.json");
  await symlink(path, link);
  expect(await readCatalogue(link)).toBeUndefined();
});
