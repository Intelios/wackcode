#!/usr/bin/env node
// Builds the dev app for scripts/dev-runner.sh, wraps the binary in a minimal WackCode.app and
// prints the wrapped executable's path on stdout. Everything else goes to stderr, which
// `tauri dev` shows as usual.
//
// macOS gives a process an app identity (a bundle id) only when its executable lives inside an
// .app bundle: an Info.plist embedded in a bare binary is ignored when it checks in as an app.
// So the bare target/debug/wackcode that `cargo run` would start is invisible to automation.
// Running the same binary from a bundle carrying tauri.conf.json's identifier fixes that and
// changes nothing else: debug builds find the worker, Node and the frontend (Vite's devUrl) from
// the source tree, never relative to the executable.
// The linker's signature covers a bare executable, not this bundle. Sign the completed app so
// its Info.plist, resources and executable have a valid, matching identity for macOS permissions.
// The ad-hoc requirement still changes on a rebuild; restarting an unchanged build does not.
//
// Usage: node scripts/dev-app.mjs <cargo args> [-- <app args>]

import { spawnSync } from "node:child_process";
import { constants, copyFileSync, existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function fail(message) {
  console.error(message);
  process.exit(1);
}

const args = process.argv.slice(2);
const separator = args.indexOf("--");
const cargoArgs = separator === -1 ? args : args.slice(0, separator);

// What `cargo run` would build. The JSON messages name the binary, wherever the profile or
// target put it; diagnostics are still rendered to stderr.
const build = spawnSync("cargo", ["build", ...cargoArgs, "--message-format=json-render-diagnostics"], {
  stdio: ["ignore", "pipe", "inherit"],
  encoding: "utf8",
  maxBuffer: 256 * 1024 * 1024,
});
if (build.error) fail(`Could not run cargo: ${build.error.message}`);
if (build.status !== 0) process.exit(build.status ?? 1);

let executable;
for (const line of build.stdout.split("\n")) {
  if (!line.startsWith("{")) continue;
  try {
    const message = JSON.parse(line);
    if (message.reason === "compiler-artifact" && message.executable && message.target?.kind?.includes("bin")) {
      executable = message.executable;
    }
  } catch {}
}
if (!executable) fail("cargo build finished without producing the app binary.");

const config = JSON.parse(readFileSync(path.join(REPO, "src-tauri/tauri.conf.json"), "utf8"));
const bundle = path.join(path.dirname(executable), "dev-app", `${config.productName}.app`);
const contents = path.join(bundle, "Contents");
const binaryName = path.basename(executable);
mkdirSync(path.join(contents, "MacOS"), { recursive: true });
mkdirSync(path.join(contents, "Resources"), { recursive: true });

const escape = (value) => String(value).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const icon = path.join(REPO, "src-tauri/icons/icon.icns");
const keys = {
  CFBundleIdentifier: config.identifier,
  CFBundleName: config.productName,
  CFBundleExecutable: binaryName,
  CFBundlePackageType: "APPL",
  CFBundleShortVersionString: config.version,
  CFBundleVersion: config.version,
  ...(existsSync(icon) ? { CFBundleIconFile: "icon.icns" } : {}),
};
writeFileSync(
  path.join(contents, "Info.plist"),
  `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
${Object.entries(keys).map(([key, value]) => `  <key>${key}</key>\n  <string>${escape(value)}</string>`).join("\n")}
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
`
);
if (existsSync(icon)) copyFileSync(icon, path.join(contents, "Resources/icon.icns"));

// An APFS clone: instant, and no second copy on disk. Staged and renamed into place, so a copy
// that is somehow still running is never overwritten underneath itself.
const target = path.join(contents, "MacOS", binaryName);
const staged = `${target}.tmp`;
copyFileSync(executable, staged, constants.COPYFILE_FICLONE);
renameSync(staged, target);

for (const args of [
  ["--force", "--sign", "-", "--identifier", config.identifier, bundle],
  ["--verify", "--strict", bundle],
]) {
  const signed = spawnSync("/usr/bin/codesign", args, { stdio: ["ignore", "ignore", "inherit"] });
  if (signed.error) fail(`Could not sign the dev app: ${signed.error.message}`);
  if (signed.status !== 0) fail("The dev app's code signature could not be verified.");
}
process.stdout.write(target);
