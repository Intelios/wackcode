import { rm } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const destination = join(root, "src-tauri", "resources", "worker");

await run("pnpm", ["--dir", join(root, "worker"), "build"]);
await rm(destination, { recursive: true, force: true });
// Hoisted, because Tauri's resource copy skips symlinks and pnpm's default layout links every
// package to its dependencies; bundled, those links vanish and nothing resolves. The only links
// left in a hoisted tree are node_modules/.bin shims, which nothing runs.
await run("pnpm", [
  "--filter",
  "@wackcode/worker",
  "deploy",
  "--prod",
  "--config.node-linker=hoisted",
  destination,
], root);

function run(command, args, cwd = root) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { cwd, stdio: "inherit" });
    child.on("error", reject);
    child.on("exit", (code) => code === 0 ? resolvePromise() : reject(new Error(`${command} exited with ${code}`)));
  });
}
