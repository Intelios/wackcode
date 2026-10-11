// `wackdev`: the dev-app driver, as a stdio MCP server. Register it once in WackCode under
// Settings › MCP servers as `node <repo>/scripts/wackdev/server.mjs`; Claude Code can run the
// same command. Every tool is aimed only at the dev app (`com.wackcode.desktop.dev` running
// from this checkout's src-tauri/target) — the targeting rule lives in Rust
// (wackdev/src/driver/target.rs), not here, so this file can never widen it.
//
// Thin: it keeps one `wackdev-helper` process alive (refs are element indexes; only a live
// process can hold them), builds it on first use, and runs `dev:background`/`dev:stop`
// itself. Plain JSON-RPC over stdio, no dependencies — the protocol surface the app's MCP
// client needs is initialize, tools/list and tools/call.

import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const SRC_TAURI = path.join(REPO, "src-tauri");
const HELPER = path.join(SRC_TAURI, "target/debug/wackdev-helper");

const TOOLS = [
  {
    name: "status",
    description:
      "Whether the WackCode dev app is running (its pid, window and executable), and whether this " +
      "process has the Accessibility and Screen Recording permissions the other tools need. " +
      "Start here; it never fails.",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "start",
    description:
      "Start the dev app (`pnpm dev:background`) and return once it is ready. The dev app is a " +
      "separate identity (bundle id com.wackcode.desktop.dev, its own data folder) — this never " +
      "touches the installed WackCode.",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "stop",
    description: "Stop the dev app (`pnpm dev:stop`). Only the dev instance is stopped.",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "tree",
    description:
      "The dev window's controls as an indented outline, e.g. `[e3-12] button \"Send\" (disabled)`. " +
      "Call this before acting: refs stay valid until the next `tree` or `find`. `within` scopes " +
      "to a ref or control name (e.g. a sheet); `filter` keeps matching lines plus their ancestors.",
    inputSchema: {
      type: "object",
      properties: {
        within: { type: "string", description: "A ref (e3-12) or control name to scope the tree to." },
        filter: { type: "string", description: "Keep only lines containing this text (with ancestors)." },
      },
    },
  },
  {
    name: "find",
    description: "Controls whose name matches `name` (optionally a `role`), with fresh refs. Use it to check before acting on a name.",
    inputSchema: {
      type: "object",
      properties: {
        name: { type: "string", description: "Control name; exact first, then case-insensitive contains." },
        role: { type: "string", description: "AX role, e.g. button, textField, checkBox." },
      },
      required: ["name"],
    },
  },
  {
    name: "press",
    description:
      "Press a control (the Accessibility press action — no mouse, no coordinates, the dev app " +
      "stays where it is). Give a `ref` from `tree`/`find`, or a `name` (+ optional `role`); " +
      "an ambiguous name errors and lists the refs to choose from.",
    inputSchema: {
      type: "object",
      properties: {
        ref: { type: "string", description: "An element ref like e3-12." },
        name: { type: "string", description: "Control name, if no ref." },
        role: { type: "string", description: "AX role to disambiguate a name." },
      },
    },
  },
  {
    name: "focus",
    description:
      "Focus a control. Keyboard focus reveals the same controls hover does, so this reaches " +
      "hover-only buttons without a pointer.",
    inputSchema: {
      type: "object",
      properties: {
        ref: { type: "string" },
        name: { type: "string" },
        role: { type: "string" },
      },
    },
  },
  {
    name: "type",
    description:
      "Type text into a field: focuses it, inserts the text (or sends real key events to the dev " +
      "app's process only — React's onChange fires either way). `submit` presses Return after. " +
      "Never types into a secure (password) field.",
    inputSchema: {
      type: "object",
      properties: {
        ref: { type: "string" },
        name: { type: "string", description: "Field name, if no ref (placeholder text counts)." },
        role: { type: "string" },
        text: { type: "string", description: "The text to type." },
        submit: { type: "boolean", description: "Press Return after typing." },
      },
      required: ["text"],
    },
  },
  {
    name: "key",
    description:
      "Send a key or chord to the dev app's process only (`cmd+k`, `cmd+shift+p`, `Return`, " +
      "`Escape`). Goes nowhere else, and never to a secure field.",
    inputSchema: {
      type: "object",
      properties: {
        chord: { type: "string", description: "e.g. \"Return\", \"Escape\", \"cmd+k\", \"cmd+shift+p\"." },
      },
      required: ["chord"],
    },
  },
  {
    name: "scroll",
    description: "Scroll the area containing an element (Accessibility scroll bars; no pointer).",
    inputSchema: {
      type: "object",
      properties: {
        ref: { type: "string" },
        name: { type: "string" },
        role: { type: "string" },
        direction: { type: "string", enum: ["up", "down", "left", "right"], description: "Default down." },
      },
    },
  },
  {
    name: "wait_for",
    description:
      "Poll until a control appears (or, with `gone`, disappears) — this replaces sleeping while " +
      "a run streams or a sheet animates. Times out with an error after `timeout_ms` " +
      "(default 10s, max 120s).",
    inputSchema: {
      type: "object",
      properties: {
        name: { type: "string", description: "Control name (title, description or placeholder)." },
        role: { type: "string" },
        gone: { type: "boolean", description: "Wait until the control is gone instead of present." },
        timeout_ms: { type: "number", description: "Default 10000, max 120000." },
      },
    },
  },
  {
    name: "shot",
    description:
      "A JPEG of the dev window only — ScreenCaptureKit, so it works while other windows cover " +
      "it. `ref` crops to that element; `save_to` also writes the file. Use it for visual " +
      "checks; for reading controls prefer `tree`.",
    inputSchema: {
      type: "object",
      properties: {
        ref: { type: "string", description: "Crop to this element's frame." },
        save_to: { type: "string", description: "Absolute path to also save the JPEG to." },
      },
    },
  },
];

// ---------------------------------------------------------------------------------------------
// wackdev-helper: built lazily, kept alive so element refs stay valid between calls.

let helper = null;
let nextId = 1;
const pending = new Map();
let building = null;

/** Builds the helper (no-op when nothing changed); falls back to the binary already on disk. */
function ensureBuilt() {
  building ??= new Promise((resolve) => {
    const build = spawnSync("cargo", ["build", "-p", "wackdev", "--bin", "wackdev-helper"], {
      cwd: SRC_TAURI,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    });
    if (build.status !== 0) {
      if (existsSync(HELPER)) {
        process.stderr.write(`wackdev: helper build failed; using the existing binary.\n${build.stderr?.slice(-2000) ?? ""}\n`);
        resolve(true);
      } else {
        process.stderr.write(`wackdev: helper build failed:\n${build.stderr?.slice(-4000) ?? build.error?.message ?? ""}\n`);
        resolve(false);
      }
      return;
    }
    resolve(true);
  });
  return building;
}

function spawnHelper() {
  const child = spawn(HELPER, [], { stdio: ["pipe", "pipe", "inherit"] });
  let buffer = "";
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    buffer += chunk;
    let newline;
    while ((newline = buffer.indexOf("\n")) !== -1) {
      const line = buffer.slice(0, newline).trim();
      buffer = buffer.slice(newline + 1);
      if (!line) continue;
      let reply;
      try {
        reply = JSON.parse(line);
      } catch {
        continue;
      }
      const waiter = pending.get(reply.id);
      if (waiter) {
        pending.delete(reply.id);
        clearTimeout(waiter.timer);
        waiter.resolve(reply);
      }
    }
  });
  child.on("exit", () => {
    const failure = { ok: false, error: "wackdev-helper exited; refs are lost. Try again (it restarts on the next call)." };
    for (const [id, waiter] of pending) {
      clearTimeout(waiter.timer);
      waiter.resolve({ id, ...failure });
    }
    pending.clear();
    helper = null;
  });
  return child;
}

/** One helper command, against a fresh helper if needed. */
async function call(cmd, args = {}, timeoutMs = 90_000) {
  if (!(await ensureBuilt())) {
    return { ok: false, error: "wackdev-helper isn't built. Run `cargo build -p wackdev --bin wackdev-helper` in src-tauri (needs the Rust toolchain)." };
  }
  helper ??= spawnHelper();
  const id = nextId++;
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      if (pending.delete(id)) resolve({ id, ok: false, error: `wackdev-helper didn't answer within ${Math.round(timeoutMs / 1000)}s.` });
    }, timeoutMs);
    pending.set(id, { resolve, timer });
    helper.stdin.write(`${JSON.stringify({ id, cmd, ...args })}\n`);
  });
}

/** `pnpm dev:background`/`dev:stop` via the same Node that's running this server. */
function script(name, timeoutMs) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [path.join(REPO, "scripts", name)], { cwd: REPO });
    let output = "";
    child.stdout.on("data", (chunk) => (output += chunk));
    child.stderr.on("data", (chunk) => (output += chunk));
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      resolve({ ok: false, error: `${name} didn't finish within ${Math.round(timeoutMs / 1000)}s.\n${output.slice(-2000)}` });
    }, timeoutMs);
    child.on("exit", (code) => {
      clearTimeout(timer);
      resolve(code === 0 ? { ok: true, output } : { ok: false, error: `${name} exited with code ${code}.\n${output.slice(-2000)}` });
    });
  });
}

// ---------------------------------------------------------------------------------------------
// Tool calls → helper commands → MCP content.

const text = (value) => ({ type: "text", text: typeof value === "string" ? value : JSON.stringify(value, null, 2) });
const ok = (value) => ({ content: [text(value)] });
const fail = (message) => ({ content: [text(message)], isError: true });

async function toolCall(name, args) {
  switch (name) {
    case "status": {
      const reply = await call("status");
      if (!reply.ok) return fail(reply.error);
      const status = reply.result;
      const lines = [status.running ? `The dev app is running (pid ${status.pid}).` : `The dev app is not running. ${status.reason}`];
      if (status.executable) lines.push(`Executable: ${status.executable}`);
      if (status.window) lines.push(`Window: "${status.window.title}" (#${status.window.number})`);
      lines.push(`Accessibility permission: ${status.permissions.accessibility ? "yes" : "NO — grant it to the app that launched this server (System Settings › Privacy & Security › Accessibility)."}`);
      lines.push(`Screen Recording permission: ${status.permissions.screenRecording ? "yes" : "NO — needed for shot."}`);
      return ok(lines.join("\n"));
    }
    case "start": {
      const result = await script("dev-background.mjs", 660_000);
      return result.ok ? ok(result.output.trim()) : fail(result.error);
    }
    case "stop": {
      const result = await script("dev-stop.mjs", 30_000);
      return result.ok ? ok(result.output.trim() || "Stopped.") : fail(result.error);
    }
    case "tree": {
      const reply = await call("tree", { within: args.within, filter: args.filter });
      if (!reply.ok) return fail(reply.error);
      const { outline, elements, truncated } = reply.result;
      return ok(`${outline}${truncated ? "(truncated — use `within` or `filter` to narrow)\n" : ""}${elements} controls; refs are e-refs from this tree.`);
    }
    case "find": {
      const reply = await call("find", { name: args.name, role: args.role });
      if (!reply.ok) return fail(reply.error);
      const matches = reply.result.matches;
      if (matches.length === 0) return ok(`Nothing named "${args.name}" is in the dev window.`);
      return ok(matches.map((match) => `${match.ref} ${match.role}${match.enabled === false ? " (disabled)" : ""}`).join("\n"));
    }
    case "press": {
      const reply = await call("press", args);
      return reply.ok ? ok(`Pressed ${reply.result.pressed}.`) : fail(reply.error);
    }
    case "focus": {
      const reply = await call("focus", args);
      return reply.ok ? ok(`Focused ${reply.result.focused}.`) : fail(reply.error);
    }
    case "type": {
      const reply = await call("type", args, 60_000);
      return reply.ok ? ok(`Typed ${reply.result.typed} characters (${reply.result.via === "ax" ? "inserted" : "as key events"}).`) : fail(reply.error);
    }
    case "key": {
      const reply = await call("key", args);
      return reply.ok ? ok(`Pressed ${args.chord}.`) : fail(reply.error);
    }
    case "scroll": {
      const reply = await call("scroll", args);
      return reply.ok ? ok("Scrolled.") : fail(reply.error);
    }
    case "wait_for": {
      const reply = await call("wait_for", args, (args.timeout_ms ?? 10_000) + 20_000);
      if (!reply.ok) return fail(reply.error);
      return ok(args.gone ? `"${args.name}" is gone.` : `"${args.name}" is present.`);
    }
    case "shot": {
      const reply = await call("shot", args);
      if (!reply.ok) return fail(reply.error);
      const { jpeg, width, height, savedTo } = reply.result;
      return {
        content: [
          { type: "image", data: jpeg, mimeType: "image/jpeg" },
          text(`Dev window, ${width}×${height}px${savedTo ? ` — saved to ${savedTo}` : ""}.`),
        ],
      };
    }
    default:
      return fail(`Unknown tool: ${name}`);
  }
}

// ---------------------------------------------------------------------------------------------
// MCP: initialize, tools/list, tools/call — nothing more is needed (see mock-mcp-server.mjs).

function handle(message) {
  const { id, method, params } = message;
  if (id === undefined || id === null) return Promise.resolve(null);
  const result = (value) => Promise.resolve({ jsonrpc: "2.0", id, result: value });
  switch (method) {
    case "initialize":
      return result({
        protocolVersion: params?.protocolVersion ?? "2025-06-18",
        capabilities: { tools: { listChanged: false } },
        serverInfo: { name: "wackdev", version: "1.0.0" },
      });
    case "ping":
      return result({});
    case "tools/list":
      return result({ tools: TOOLS });
    case "tools/call":
      return toolCall(params?.name, params?.arguments ?? {}).then(result);
    default:
      return Promise.resolve({ jsonrpc: "2.0", id, error: { code: -32601, message: `Method not found: ${method}` } });
  }
}

// Calls in flight must still answer before stdin EOF takes the process down (a client can
// close its pipe after its last request).
let stdinEnded = false;
let inflight = 0;
function maybeExit() {
  if (stdinEnded && inflight === 0) {
    if (helper) helper.stdin.end();
    process.exit(0);
  }
}

let buffer = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buffer += chunk;
  let newline;
  while ((newline = buffer.indexOf("\n")) !== -1) {
    const line = buffer.slice(0, newline).trim();
    buffer = buffer.slice(newline + 1);
    if (!line) continue;
    inflight++;
    void handle(JSON.parse(line))
      .then((reply) => {
        if (reply) process.stdout.write(`${JSON.stringify(reply)}\n`);
      })
      .finally(() => {
        inflight--;
        maybeExit();
      });
  }
});
process.stdin.on("end", () => {
  stdinEnded = true;
  maybeExit();
});
process.stderr.write("wackdev MCP server ready on stdio\n");
