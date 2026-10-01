// Runs after `tauri build`. Tauri re-signs the bundled Node with `codesign --force` and the
// hardened runtime, which drops the entitlements the official binary ships with. Without them V8
// cannot reserve its code range ("Failed to reserve virtual memory for CodeRange") and every
// worker and helper dies on launch. Tauri applies one entitlements file to every executable it
// signs, so this re-signs only the Node runtime, then re-seals the app; the host process keeps
// Tauri's signature, without these entitlements.
//
// - allow-jit: V8 maps its code space with MAP_JIT.
// - disable-library-validation: native addons (Pi's own, and any in packages the user installs)
//   are signed ad hoc or by another team, so library validation would refuse to load them.
//
// Finally it runs the bundled model catalogue offline, so a bundle whose Node or worker resources
// cannot start fails the build instead of the first chat.
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const profile = process.argv.includes("--debug") ? "debug" : "release";
const app = join(root, "src-tauri", "target", profile, "bundle", "macos", "WackCode.app");
const node = join(app, "Contents", "MacOS", "wackcode-node");
const entitlements = join(root, "src-tauri", "node-runtime.entitlements");

if (!existsSync(node)) {
  throw new Error(`There is no bundled Node runtime at ${node}. Run tauri build first.`);
}

await run("codesign", ["--force", "--sign", "-", "--options", "runtime", "--entitlements", entitlements, node]);
// Re-signing nested code changes its hash, so the app's own seal has to be rebuilt around it.
await run("codesign", [
  "--force",
  "--sign",
  "-",
  "--preserve-metadata=identifier,entitlements,requirements,flags,runtime",
  app,
]);
await run("codesign", ["--verify", "--deep", "--strict", app]);

// The same offline, keyless invocation as `list_builtin_models` in worker.rs.
const catalog = await run(node, [join(app, "Contents", "Resources", "resources", "worker", "dist", "catalog.js")], {
  HOME: process.env.HOME,
  PATH: "/usr/bin:/bin",
  PI_TELEMETRY: "0",
  PI_SKIP_VERSION_CHECK: "1",
  PI_OFFLINE: "1",
});
const models = JSON.parse(catalog);
if (!Array.isArray(models) || models.length === 0) {
  throw new Error("The bundled Pi model catalogue came back empty.");
}
process.stdout.write(`Signed the bundled Node runtime; its model catalogue lists ${models.length} models.\n`);

function run(command, args, env) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { env: env ?? process.env, stdio: ["ignore", "pipe", "inherit"] });
    let stdout = "";
    child.stdout.on("data", (chunk) => (stdout += chunk));
    child.on("error", reject);
    child.on("exit", (code, signal) =>
      code === 0 ? resolvePromise(stdout) : reject(new Error(`${command} exited with ${code ?? signal}`)),
    );
  });
}
