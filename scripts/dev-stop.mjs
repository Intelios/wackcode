#!/usr/bin/env node
// Tears down whatever a `pnpm dev:background` (or a stray foreground
// `pnpm dev:desktop`) left behind: the dev app binary, this repo's
// tauri CLI, and Vite listening on port 1420. Safe to run when nothing is up,
// and it never touches another project's processes or a foreign program that
// happens to hold the port.

import { execFile } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { DEV_APP } from "./dev-paths.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PORT = 1420;

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

async function findDevApp() {
  const { ok, stdout } = await run("ps", ["-axo", "pid=,comm="]);
  if (!ok) return [];
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

let stopped = 0;

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
  stopped += pids.length;
  console.log(`Stopped ${label} (pid ${pids.join(", ")}).`);
}

const app = await findDevApp();
if (app.length > 0) await killProcesses(app, "the WackCode Dev app");

const cli = await findTauriCli();
if (cli.length > 0) await killProcesses(cli, "the tauri CLI");

const holders = await portHolders();
if (holders.length > 0) {
  const foreign = holders.find((h) => !h.command.includes(REPO + path.sep));
  if (foreign) {
    console.log(
      `Port ${PORT} is still held by another program (${foreign.command.slice(0, 160)}); leaving it alone.`
    );
  } else {
    await killProcesses(holders, `Vite on port ${PORT}`);
  }
}

if (stopped === 0) {
  console.log("No WackCode Dev instance is running.");
} else {
  console.log("WackCode Dev instance stopped.");
}
