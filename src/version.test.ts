/// <reference types="node" />
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Guards for the numbers Settings › About shows. The Pi and Node versions are baked into the
 * binary by `build.rs` from `runtime-lock.json`, so the lock must stay in step with the pin the
 * worker actually installs from; and the app version comes from `tauri.conf.json`, so the three
 * places it is written must move together. Fix the files, not the test.
 */

const root = resolve(__dirname, "..");

function readJSON(path: string) {
  return JSON.parse(readFileSync(resolve(root, path), "utf8"));
}

describe("version guards", () => {
  it("runtime-lock.json's Pi pin matches the worker's pinned dependency", () => {
    const lock = readJSON("runtime-lock.json");
    const worker = readJSON("worker/package.json");
    expect(lock.pi.version).toBe(worker.dependencies["@earendil-works/pi-coding-agent"]);
  });

  it("the app version is written once, in agreement, everywhere it lives", () => {
    const pkg = readJSON("package.json");
    const conf = readJSON("src-tauri/tauri.conf.json");
    // The package section's own `version = "…"` line; dependency specs nest theirs inside braces.
    const cargo = readFileSync(resolve(root, "src-tauri/Cargo.toml"), "utf8")
      .match(/^version\s*=\s*"([^"]+)"/m)?.[1];
    expect(cargo).toBeDefined();
    expect(conf.version).toBe(pkg.version);
    expect(cargo).toBe(pkg.version);
  });
});