/**
 * Offline cold-history benchmark: no app data, providers or credentials are read.
 * Creates a disposable 1,000-message / 100-screenshot Pi session, then starts a fresh
 * keyless reader for each run. Supports both the old single JSON response and NDJSON.
 *
 * node scripts/benchmark-history.mjs --runs 5 --keep-fixture
 * node scripts/benchmark-history.mjs --reader /tmp/old/dist/session-reader.js --fixture /tmp/session.jsonl
 */
import { spawn } from "node:child_process";
import { createWriteStream } from "node:fs";
import { mkdtemp, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { once } from "node:events";
import { deflateSync } from "node:zlib";

const args = process.argv.slice(2);
const value = (name, fallback) => {
  const index = args.indexOf(name);
  return index === -1 ? fallback : args[index + 1];
};
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const reader = resolve(value("--reader", join(root, "worker/dist/session-reader.js")));
const runs = Number(value("--runs", "5"));
if (!Number.isInteger(runs) || runs < 1 || runs > 100) throw new Error("--runs must be an integer from 1 to 100.");
const keep = args.includes("--keep-fixture");
const suppliedFixture = value("--fixture");
const folder = suppliedFixture ? undefined : await mkdtemp(join(tmpdir(), "wackcode-history-benchmark-"));
const sessionFile = suppliedFixture ? resolve(suppliedFixture) : join(folder, "session.jsonl");
let activeReader;
for (const signal of ["SIGINT", "SIGTERM"]) process.once(signal, () => {
  activeReader?.kill("SIGKILL");
  process.exit(signal === "SIGINT" ? 130 : 143);
});

// Encode a deterministic, screenshot-sized RGB PNG using only Node's standard library.
function screenshot() {
  const table = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let entry = i;
    for (let bit = 0; bit < 8; bit++) entry = entry & 1 ? 0xedb88320 ^ (entry >>> 1) : entry >>> 1;
    table[i] = entry;
  }
  const chunk = (name, data) => {
    const type = Buffer.from(name);
    const length = Buffer.alloc(4);
    length.writeUInt32BE(data.length);
    let crc = 0xffffffff;
    for (const byte of Buffer.concat([type, data])) crc = table[(crc ^ byte) & 255] ^ (crc >>> 8);
    const checksum = Buffer.alloc(4);
    checksum.writeUInt32BE((crc ^ 0xffffffff) >>> 0);
    return Buffer.concat([length, type, data, checksum]);
  };
  const width = 1920, height = 1080;
  const rows = Buffer.alloc((width * 3 + 1) * height);
  let random = 0x12345678;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      random ^= random << 13; random ^= random >>> 17; random ^= random << 5;
      const at = y * (width * 3 + 1) + x * 3 + 1;
      const noise = x % 15 === 0 ? random & 3 : 0;
      // Repeated UI-like horizontal/vertical bands with small deterministic noise.
      rows[at] = (Math.floor(x / 8) + noise) & 255;
      rows[at + 1] = (Math.floor(y / 8) + noise) & 255;
      rows[at + 2] = (Math.floor((x + y) / 16) + noise) & 255;
    }
  }
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0); header.writeUInt32BE(height, 4);
  header[8] = 8; header[9] = 2;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", header), chunk("IDAT", deflateSync(rows)), chunk("IEND", Buffer.alloc(0))]);
}

async function createFixture() {
  const image = screenshot();
  const pixels = image.toString("base64");
  const output = createWriteStream(sessionFile, { flags: "wx" });
  const line = async (entry) => { if (!output.write(`${JSON.stringify(entry)}\n`)) await once(output, "drain"); };
  const epoch = Date.UTC(2026, 9, 1);
  await line({ type: "session", version: 3, id: "history-benchmark", timestamp: new Date(epoch).toISOString(), cwd: "/missing-history-benchmark-workspace" });
  const usage = { input: 1000, output: 100, cacheRead: 0, cacheWrite: 0, totalTokens: 1100, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };
  let parentId = null, position = 0;
  const append = async (message) => {
    const id = `entry-${++position}`;
    await line({ type: "message", id, parentId, timestamp: new Date(epoch + position * 1000).toISOString(), message: { ...message, timestamp: epoch + position * 1000 } });
    parentId = id;
  };
  for (let turn = 0; turn < 250; turn++) {
    const imageTurn = turn % 5 === 0;
    const content = [{ type: "text", text: `Benchmark request ${turn + 1}. Inspect the saved screenshot and continue the implementation.` }];
    if (imageTurn) content.push({ type: "image", mimeType: "image/png", data: pixels });
    await append({ role: "user", content });
    await append({ role: "assistant", api: "openai-completions", provider: "benchmark", model: "offline", stopReason: "toolUse", usage, content: [{ type: "toolCall", id: `call-${turn}`, name: "read", arguments: { path: "fixture.png" } }] });
    const result = [{ type: "text", text: "Read the synthetic fixture.\n" + "Synthetic tool output for the cold history benchmark.\n".repeat(20) }];
    if (imageTurn) result.push({ type: "image", mimeType: "image/png", data: pixels });
    await append({ role: "toolResult", toolName: "read", toolCallId: `call-${turn}`, isError: false, content: result });
    await append({ role: "assistant", api: "openai-completions", provider: "benchmark", model: "offline", stopReason: "stop", usage, content: [{ type: "text", text: `Completed synthetic turn ${turn + 1}.\n\nThe saved transcript remains available offline.` }] });
  }
  output.end(); await once(output, "finish");
  return { messages: position, images: 100, imageBytes: image.length };
}

async function run() {
  const started = performance.now();
  const child = spawn(process.execPath, [reader], { env: {}, stdio: ["pipe", "pipe", "pipe"] });
  activeReader = child;
  const deadline = setTimeout(() => child.kill("SIGKILL"), 120_000);
  let pending = "", firstSnapshotMs, outputBytes = 0, frames = 0, previewCount = 0, error = "";
  const accept = (line) => {
    if (!line.trim()) return;
    const frame = JSON.parse(line);
    frames++;
    if (frame.type === "snapshot" || Array.isArray(frame.messages)) firstSnapshotMs ??= performance.now() - started;
    if (frame.type === "history_previews") previewCount += frame.previews.length;
  };
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    outputBytes += Buffer.byteLength(chunk);
    pending += chunk;
    let newline;
    while ((newline = pending.indexOf("\n")) !== -1) { accept(pending.slice(0, newline)); pending = pending.slice(newline + 1); }
  });
  child.stderr.on("data", (chunk) => { error = (error + chunk).slice(-2000); });
  child.stdin.end(JSON.stringify({ sessionFile, taskId: "history-benchmark", mode: "build", thinkingLevel: "off", contextWindow: 200_000 }));
  let code, signal;
  try {
    [code, signal] = await once(child, "close");
  } finally {
    clearTimeout(deadline);
    activeReader = undefined;
  }
  if (pending.trim()) accept(pending);
  if (code !== 0) throw new Error(`Reader exited ${code ?? signal}: ${error}`);
  if (firstSnapshotMs === undefined) throw new Error("Reader emitted no snapshot.");
  return { firstSnapshotMs: Math.round(firstSnapshotMs), completeMs: Math.round(performance.now() - started), outputBytes, frames, previewCount };
}

try {
  const fixture = folder ? await createFixture() : {};
  const measurements = [];
  process.stderr.write(`Fixture: ${sessionFile} (${(await stat(sessionFile)).size} bytes)\n`);
  for (let i = 0; i < runs; i++) {
    const measurement = await run(); measurements.push(measurement);
    process.stderr.write(`Run ${i + 1}: first snapshot ${measurement.firstSnapshotMs} ms; complete ${measurement.completeMs} ms\n`);
  }
  const median = (key) => [...measurements].sort((a, b) => a[key] - b[key])[Math.floor(measurements.length / 2)][key];
  const report = { reader, sessionFile, sessionBytes: (await stat(sessionFile)).size, ...fixture, runs: measurements, medianFirstSnapshotMs: median("firstSnapshotMs"), medianCompleteMs: median("completeMs") };
  if (value("--report")) await writeFile(resolve(value("--report")), JSON.stringify(report, null, 2) + "\n");
  process.stdout.write(JSON.stringify(report, null, 2) + "\n");
} finally {
  if (folder && !keep) await rm(folder, { recursive: true, force: true });
}
