// The dev app's identity, shared by every script that launches, wraps or stops it, so the
// bundle path and ids can only live in one place.

export const DEV_BUNDLE_ID = "com.wackcode.desktop.dev";
export const DEV_APP_NAME = "WackCode Dev";
export const DEV_CONFIG_OVERLAY = "src-tauri/tauri.dev.conf.json";

// Where scripts/dev-app.mjs puts the dev binary. The debug and release bundles live elsewhere
// (target/*/bundle/) and the installed app's `com.wackcode.desktop` never matches.
export const DEV_APP = `target/debug/dev-app/${DEV_APP_NAME}.app/Contents/MacOS/wackcode`;

// The dev app's own data folder; a `pnpm dev:reset` moves it to the Trash.
export const DEV_DATA_DIR = `${process.env.HOME}/Library/Application Support/${DEV_BUNDLE_ID}`;
