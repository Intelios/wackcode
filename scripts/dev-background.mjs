#!/usr/bin/env node
// Detached, idempotent launcher for `pnpm dev:desktop`, for agents (and humans)
// that need the live app without babysitting a blocking process.
//
// - A healthy running instance (Vite listening + the debug app binary) is left
//   alone; the script prints "Ready" and exits.
// - A stale half-instance (orphaned Vite on the port, a dead app's tauri CLI)
//   is torn down first.
// - Otherwise it spawns `pnpm dev:desktop` detached with output appended to a
//   log file, waits until Vite is listening on port 1420 and the app process
//   exists, prints "Ready", and exits — so the caller never has to guess when
//   the window is up.

import { spawn, execFile } from "node:child_process";
import { appendFileSync, existsSync, openSync, readFileSync } from "node:fs";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { DEV_APP, DEV_BUNDLE_ID } from "./dev-paths.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PORT = 1420;
const LOG_FILE = "/tmp/wackcode-dev.log";
const POLL_MS = 400;
const SETTLE_MS = 2000;
const HEARTBEAT_MS = 15000;
const DEFAULT_TIMEOUT_SEC = 600;

const timeoutMs =
  (Number(process.env.WACKCODE_DEV_TIMEOUT_SEC) || DEFAULT_TIMEOUT_SEC) * 1000;

function fail(message) {
  console.error(message);
  process.exit(1);
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function run(cmd, args) {
  return new Promise((resolve) => {
    execFile(cmd, args, { encoding: "utf8" }, (error, stdout) => {
      resolve({ ok: !error, stdout: stdout ?? "" });
    });
  });
}

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function alive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code === "EPERM";
  }
}

// The dev app runs from the minimal bundle scripts/dev-app.mjs makes (or, under
// a plain `tauri dev`, as the bare cargo binary). The debug and release bundles'
// binaries live under .../bundle/macos/ and never match.
async function findDevApp() {
  const { ok, stdout } = await run("ps", ["-axo", "pid=,comm="]);
  if (!ok) fail("Could not list processes with `ps`. Is this macOS?");
  const found = [];
  for (const line of stdout.split("\n")) {
    const match = line.trim().match(/^(\d+)\s+(.*)$/);
    if (!match) continue;
    const comm = match[2].trim();
    if (comm.endsWith(DEV_APP) || comm.endsWith("target/debug/wackcode") || comm === "wackcode") {
      found.push({ pid: Number(match[1]), command: comm });
    }
  }
  return found;
}

function tryConnect(host) {
  return new Promise((resolve) => {
    const socket = net.connect({ port: PORT, host });
    socket.once("connect", () => {
      socket.destroy();
      resolve(true);
    });
    socket.once("error", () => resolve(false));
  });
}

async function viteIsUp() {
  return (await tryConnect("127.0.0.1")) || (await tryConnect("::1"));
}

async function portHolders() {
  const { ok, stdout } = await run("lsof", [
    "-nP",
    `-iTCP:${PORT}`,
    "-sTCP:LISTEN",
    "-Fp",
  ]);
  if (!ok) return [];
  const holders = [];
  for (const line of stdout.split("\n")) {
    if (!line.startsWith("p")) continue;
    const pid = Number(line.slice(1));
    if (!Number.isFinite(pid)) continue;
    const { ok: psOk, stdout: command } = await run("ps", [
      "-p",
      String(pid),
      "-o",
      "command=",
    ]);
    holders.push({ pid, command: psOk ? command.trim() : "" });
  }
  return holders;
}

// The tauri CLI tree under this repo's node_modules, running `dev`. Scoped to
// this checkout so another project's `tauri dev` is never touched.
async function findTauriCli() {
  const pattern = escapeRegExp(path.join(REPO, "node_modules")) + ".*tauri";
  const { ok, stdout } = await run("pgrep", ["-f", pattern]);
  if (!ok) return [];
  const found = [];
  for (const line of stdout.split("\n")) {
    const pid = Number(line);
    if (!Number.isFinite(pid) || pid === 0) continue;
    const { ok: psOk, stdout: command } = await run("ps", [
      "-p",
      String(pid),
      "-o",
      "command=",
    ]);
    if (psOk && command.includes("tauri") && /(^|\s)dev(\s|$)/.test(command)) {
      found.push({ pid, command: command.trim() });
    }
  }
  return found;
}

async function killProcesses(processes, label) {
  const pids = processes.map((p) => p.pid).filter(alive);
  if (pids.length === 0) return;
  for (const pid of pids) {
    try {
      process.kill(pid, "SIGTERM");
    } catch {}
  }
  const deadline = Date.now() + 5000;
  while (pids.some(alive) && Date.now() < deadline) await sleep(250);
  for (const pid of pids.filter(alive)) {
    try {
      process.kill(pid, "SIGKILL");
    } catch {}
  }
  console.log(`Stopped ${label} (pid ${pids.join(", ")}).`);
}

function logTail(lines = 40) {
  try {
    return readFileSync(LOG_FILE, "utf8").trim().split("\n").slice(-lines).join("\n");
  } catch {
    return "(log is empty or missing)";
  }
}

function printReady(pid) {
  console.log("");
  console.log(`Ready. The WackCode Dev app is running (pid ${pid}).`);
  console.log(
    "It runs from a minimal WackCode Dev.app, so automation can target it by its"
  );
  console.log(`bundle id, ${DEV_BUNDLE_ID}, and its own data folder.`);
  console.log(`Log: ${LOG_FILE}`);
  console.log("Stop with: pnpm dev:stop");
}

const nodeBinary = path.join(
  REPO,
  "src-tauri/binaries/wackcode-node-aarch64-apple-darwin"
);
if (!existsSync(nodeBinary)) {
  fail(
    "The bundled Node runtime is missing. Run `pnpm prepare:runtime` once, then retry."
  );
}

const runningApp = await findDevApp();
const viteUp = await viteIsUp();

if (runningApp.length > 0 && viteUp) {
  console.log(
    `A healthy WackCode dev instance is already running (app pid ${runningApp[0].pid}).`
  );
  printReady(runningApp[0].pid);
  process.exit(0);
}

if (runningApp.length > 0 || viteUp) {
  console.log("Cleaning up a stale dev instance first.");
  if (runningApp.length > 0) {
    await killProcesses(runningApp, "the leftover dev app");
  }
  if (viteUp) {
    const holders = await portHolders();
    const foreign = holders.find((h) => !h.command.includes(REPO + path.sep));
    if (foreign) {
      fail(
        `Port ${PORT} is held by another program (${foreign.command.slice(0, 160)}). ` +
          "Stop it, then run `pnpm dev:background` again."
      );
    }
    if (holders.length > 0) {
      await killProcesses(holders, `the process holding port ${PORT}`);
    }
    const free = Date.now() + 5000;
    while (Date.now() < free && (await viteIsUp())) await sleep(250);
  }
  // The CLI normally exits on its own once the app is gone; sweep it if not.
  const cli = await findTauriCli();
  if (cli.length > 0) await killProcesses(cli, "the leftover tauri CLI");
}

console.log(`Starting \`pnpm dev:desktop\` detached; output is appended to ${LOG_FILE}.`);
const fd = openSync(LOG_FILE, "a");
appendFileSync(fd, `\n===== wackcode dev started ${new Date().toISOString()} =====\n`);
const child = spawn("pnpm", ["dev:desktop"], {
  cwd: REPO,
  detached: true,
  stdio: ["ignore", fd, fd],
});
child.unref();
let childExit = null;
child.on("exit", (code, signal) => {
  childExit = { code, signal };
});

const startedAt = Date.now();
const deadline = startedAt + timeoutMs;

async function waitFor(label, check) {
  let heartbeat = Date.now();
  while (Date.now() < deadline) {
    if (childExit) {
      fail(
        `The dev process exited before ${label} (code ${childExit.code ?? childExit.signal}). Last log lines:\n${logTail()}`
      );
    }
    if (await check()) return;
    await sleep(POLL_MS);
    if (Date.now() - heartbeat >= HEARTBEAT_MS) {
      console.log(
        `Still waiting for ${label} (${Math.round((Date.now() - startedAt) / 1000)}s elapsed).`
      );
      heartbeat = Date.now();
    }
  }
  fail(
    `Timed out waiting for ${label} after ${Math.round(timeoutMs / 1000)}s. Last log lines:\n${logTail()}`
  );
}

console.log("Waiting for Vite on port 1420…");
await waitFor("Vite to listen on port 1420", viteIsUp);
console.log("Waiting for the app process (a cold Rust build can take a few minutes)…");
await waitFor("the app process to start", async () => (await findDevApp()).length > 0);
await sleep(SETTLE_MS);
const app = await findDevApp();
if (app.length === 0 || !alive(app[0].pid)) {
  fail(`The app process did not stay up. Last log lines:\n${logTail()}`);
}
printReady(app[0].pid);
