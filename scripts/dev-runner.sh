#!/bin/sh
# The `tauri dev` runner (see "dev:desktop" in package.json). Tauri calls it in place of cargo,
# as `<runner> run <cargo args> -- <app args>` from src-tauri/.
#
# It builds what `cargo run` would, then starts the binary from inside a minimal WackCode.app
# (scripts/dev-app.mjs), so the dev app has the bundle identity macOS automation needs.
#
# A shell script rather than Node: to restart the app after a Rust edit, Tauri kills the
# runner's process, so the app has to *be* that process (exec), or it would outlive the kill.
set -e

if [ "$1" != "run" ]; then
  exec cargo "$@"
fi
shift

executable=$(node "$(dirname "$0")/dev-app.mjs" "$@")

while [ $# -gt 0 ] && [ "$1" != "--" ]; do shift; done
if [ $# -gt 0 ]; then shift; fi
exec "$executable" "$@"
