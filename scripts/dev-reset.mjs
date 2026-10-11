#!/usr/bin/env node
// Stops the dev app and moves its data folder to the Trash for a clean slate. Only ever the
// dev folder (`com.wackcode.desktop.dev`) — the installed app's data is never touched, and the
// path is checked literally rather than derived.

import { execFile } from "node:child_process";
import { existsSync, renameSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { DEV_DATA_DIR } from "./dev-paths.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function run(cmd, args) {
  return new Promise((resolve) => {
    execFile(cmd, args, { encoding: "utf8" }, (error, stdout, stderr) => {
      resolve({ ok: !error, stdout: stdout ?? "", stderr: stderr ?? "" });
    });
  });
}

// Guard belt and braces: the constant is the only folder this script will ever move, and it
// must end in the dev bundle id.
if (!DEV_DATA_DIR.endsWith("/com.wackcode.desktop.dev")) {
  console.error(`Refusing to move ${DEV_DATA_DIR}: it isn't the dev app's data folder.`);
  process.exit(1);
}

const stop = await run("node", [path.join(REPO, "scripts/dev-stop.mjs")]);
process.stdout.write(stop.stdout);
if (!stop.ok) process.exit(1);

if (!existsSync(DEV_DATA_DIR)) {
  console.log(`No dev data folder at ${DEV_DATA_DIR}; nothing to reset.`);
  process.exit(0);
}

const trash = `${process.env.HOME}/.Trash/com.wackcode.desktop.dev ${new Date().toISOString().replace(/[:.]/g, "-")}`;
renameSync(DEV_DATA_DIR, trash);
console.log(`Moved the dev data folder to the Trash (${trash}).`);
console.log("The next `pnpm dev:background` starts with a clean slate.");
