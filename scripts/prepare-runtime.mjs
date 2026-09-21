import { createHash } from "node:crypto";
import { chmod, copyFile, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const lock = JSON.parse(await readFile(join(root, "runtime-lock.json"), "utf8"));
const runtime = lock.node;

if (process.platform !== runtime.platform || process.arch !== runtime.architecture) {
  throw new Error(`This proof of concept packages ${runtime.platform}/${runtime.architecture}; current host is ${process.platform}/${process.arch}`);
}

const destination = join(root, "src-tauri", "binaries", "wackcode-node-aarch64-apple-darwin");
const temporary = await mkdtemp(join(tmpdir(), "wackcode-node-"));
const archivePath = join(temporary, runtime.archive);

try {
  const response = await fetch(runtime.source);
  if (!response.ok) throw new Error(`Could not download Node ${runtime.version}: HTTP ${response.status}`);
  const archive = Buffer.from(await response.arrayBuffer());
  const checksum = createHash("sha256").update(archive).digest("hex");
  if (checksum !== runtime.sha256) {
    throw new Error(`Node archive checksum mismatch: expected ${runtime.sha256}, received ${checksum}`);
  }
  await writeFile(archivePath, archive, { mode: 0o600 });
  await run("tar", ["-xJf", archivePath, "-C", temporary]);
  const source = join(temporary, `node-v${runtime.version}-darwin-arm64`, "bin", "node");
  await copyFile(source, destination);
  await chmod(destination, 0o755);
  process.stdout.write(`Prepared Node ${runtime.version} (${runtime.sha256.slice(0, 12)}…)\n`);
} finally {
  await rm(temporary, { recursive: true, force: true });
}

function run(command, args) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { stdio: "inherit" });
    child.on("error", reject);
    child.on("exit", (code) => code === 0 ? resolvePromise() : reject(new Error(`${command} exited with ${code}`)));
  });
}
