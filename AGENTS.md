# AGENTS.md

WackCode is a local macOS desktop app for the [Pi coding agent](https://www.npmjs.com/package/@earendil-works/pi-coding-agent), built with Tauri 2, React 19 + TypeScript, and Rust. Pi is pinned in `worker/package.json`. `README.md` describes user-facing behavior; update it when that behavior changes.

**Keep this file short:** it is sent with every agent request. Explain features in comments next to their code, which is how this codebase already documents its invariants. Add a line here only for a rule that spans files and that an agent would break without being told.

## Ground rules

- macOS on Apple Silicon only. Unix-only APIs (`killpg`, `open(1)`) are deliberate; don't "fix" them for other platforms.
- pnpm only (version pinned in `package.json`). No npm/yarn, no `package-lock.json`.

## Architecture

- **`src/`**: React renderer. `App.tsx` owns all cross-cutting state and is the only place that listens to Tauri events. `api.ts` is the only bridge to Rust (one typed `invoke` wrapper per command). Components are presentational; `SettingsPage.tsx` is the one exception that calls `api` directly.
- **`src-tauri/src/`**: Rust. `commands.rs` holds the Tauri commands (subscription sign-in: `subscriptions.rs`), `worker.rs` runs one Node worker per chat, `storage.rs` writes `wackcode.json`, `secrets.rs` writes API keys.
- **`worker/src/`**: the Node worker wrapping Pi. `protocol.ts` defines its NDJSON stdin/stdout protocol; built-in extensions (Plan mode, todo, `ask_user_question`, sub-agents) live in `builtin/`.
- **One prompt:** `App.tsx` → `api.prompt` → `commands::prompt` → NDJSON on the worker's stdin → Pi → NDJSON events on stdout → `worker.rs::handle_worker_line` (persists, re-emits as the Tauri event `worker-event`) → `App.tsx`.
- **Other processes:** `manager.ts` installs packages and never receives an API key; `subscription-auth.ts` runs OAuth sign-in, only after the user clicks Sign in. Sub-agents are not processes but in-process child sessions inside the chat's worker.

## Cross-file rules

- **Shared types:** a type that crosses layers changes in `models.rs`, `types.ts`, and (if the worker sees it) `protocol.ts` together. New optional Rust fields need `#[serde(default)]` so existing `wackcode.json` files load; runtime-only fields (like `ProjectRecord.branch`) also need `skip_deserializing`.
- **New Tauri command:** function in `commands.rs` (validation helpers are at the bottom), registered in `lib.rs` `generate_handler![…]`, wrapper in `api.ts`. A new plugin permission also goes in `capabilities/default.json`.
- **Worker protocol change:** `protocol.ts` → Rust (`worker.rs` payloads and `handle_worker_line`, call sites in `commands.rs`) → `WorkerEvent` in `types.ts` → the `worker-event` switch in `App.tsx`. Run `pnpm test:worker` afterwards.
- **Command queue:** a running prompt holds the worker's serial command queue until it settles. `abort` and `extension_ui_response` bypass it; anything else that must reach a run in progress has to as well, or the run deadlocks.
- **Incremental snapshots:** a new snapshot field that can change mid-run must also be added to `SnapshotDelta` and merged in `applySnapshotDelta` (`chat-utils.ts`), or the UI only sees it on full snapshots. Keep `delta.ts`'s full-snapshot fallback, and keep unchanged messages as the same objects (the transcript's memoization depends on it).
- **Worker restarts:** a changed fingerprint in `worker.rs` (provider, model, package resources) kills and respawns the worker, even mid-run. Settings applied live (`set_tools`, `set_subagents`, `set_mode`, `set_prompts`) must stay out of it.
- **Per-chat lock:** any command that sends a chat work, moves its conversation, or touches its checkpoints holds that chat's `TaskLocks` entry.
- **Tools:** never pass `tools:` to the chat's `createAgentSession`. In Pi 0.86.1 it is a hard registry filter that erases extension tools, so they can't even appear as switched off in Settings. The user's choice is a denylist applied with `setActiveToolsByName`; built-in tools are exempt from it.
- **Planning modes:** Plan and Ultra Plan both count as planning, so test `mode !== "build"` (`isPlanMode` in the frontend), never `=== "plan"`. The Plan contract text must stay byte-identical: `builtin.test.ts` pins its hash, and any change re-appends the contract to every saved Plan chat, so don't just update the hash. User custom prompts (Settings › Prompts, `prompt-overrides.ts`) layer on top: the marker line stays code-controlled, the shipped defaults are what an absent override leaves, and Settings display copies live in `src/promptDefaults.ts`, pinned to the worker's builders by tests on both sides.
- **Built-ins:** adding or renaming one in `worker/src/builtin/` means updating `BUILTIN_EXTENSIONS` in `PackagesSection.tsx` too.
- **Session tree and checkpoints:** read the invariants at the top of `worker/src/tree.ts` and `src-tauri/src/checkpoints.rs` before changing either. Checkpoints must never write to the project's own Git repository.
- **Drag and drop:** `dragDropEnabled: false` in `tauri.conf.json` is deliberate. The native handler would otherwise swallow the HTML5 drops the composer uses for images.

## Security (don't weaken)

- **Network:** no telemetry, account, backend, updater, automatic model discovery, or traffic on launch or on a timer. The app contacts only user-configured provider endpoints, Pi's subscription endpoints (after an explicit sign-in; token refresh during prompts), and the npm registry plus a package's own source while the user browses or installs one. Never add a network origin silently; call it out explicitly.
- **CSP** stays `default-src 'self'` with IPC-only `connect-src`. The renderer never fetches; remote calls go through Rust or the package-manager process.
- **Credentials:** API keys persist only in `secrets.json` (0600); OAuth credentials only in per-provider 0600 `auth.json` files. A worker receives keys only over its stdin (`init`, `set_subagents`, first-prompt `autoTitle`) and only the auth paths it needs. Credentials never go in argv, env vars, `wackcode.json`, Tauri events, or the renderer. Keep the provider env-key stripping in `worker.rs` and both redaction layers (`safeError` in the worker, `redact_and_limit` in Rust). The macOS Keychain backend was removed on purpose; don't bring it back.
- **Pi lockdown:** the worker keeps Pi's telemetry, version check, network model refresh, and all auto-discovery disabled (every `no*` flag stays `true`). Packages load only from explicit paths Rust passes for trusted packages, so a project's own `.pi/` never runs. Project context files such as `AGENTS.md` load on purpose.
- **Package trust:** an installed extension is unsandboxed code inside the worker that holds the API key. `trusted_at` is set only by a user-confirmed `install_package` or `trust_package`. Never default it, and keep `sync_packages` from trusting packages that arrive by any other route.

## Conventions

- **Rust:** fallible functions return `Result<T, String>` whose error is a user-facing sentence; no `anyhow`/`thiserror`. Every `wackcode.json` write goes through `MetadataState::mutate`. Git goes through the system `git` CLI (no `git2`).
- **React:** update state immutably via `patchTask` / `patchRuntime` / `setData`. Reuse the primitives in `src/components/ui/` (Popover, Menu, MenuButton, Select, Tooltip, ConfirmDialog). Destructive actions go through `ConfirmDialog` with `danger`.
- **CSS:** one stylesheet, `src/styles.css`: add rules to the matching section, use `--wc-*` variables and the existing button classes, no Tailwind or other CSS system. Keep the `data-tauri-drag-region` strips working (overlay title bar), `aria-label` on icon-only buttons, and the `prefers-reduced-motion` block.
- **Tests:** pure logic in colocated `*.test.ts`; components with Testing Library role-based queries; worker behavior in `worker/src/*.test.ts` (a real worker against an inline mock provider); Rust in inline `#[cfg(test)]` modules with `tempfile`.

## Commands

| Command | When |
|---|---|
| `pnpm check` | Fast compile check: frontend, worker, `cargo check`. |
| `pnpm test` | All suites; or `test:web`, `test:worker`, `test:rust` individually. |
| `pnpm build:worker` | After editing `worker/src/` (the dev app runs `worker/dist/`). |
| `pnpm prepare:runtime` | Once per machine: fetches the pinned Node that `tauri dev`/`build` need. |
| `pnpm dev:background` / `pnpm dev:stop` | Start / stop the live dev app (below). |
| `pnpm mock:provider` | Deterministic mock provider at `http://127.0.0.1:43127/v1` (Chat Completions). |

## Testing the running app

- **Start** with `pnpm dev:background`: idempotent, prints `Ready` once the app is up, logs to `/tmp/wackcode-dev.log`. **Stop** with `pnpm dev:stop`. Don't launch the app any other way, and don't restart it between tests: frontend edits hot-reload, Rust edits rebuild and relaunch the app automatically, and worker edits need `pnpm build:worker` plus a new chat.
- **Drive** the window by its bundle id, `com.wackcode.desktop`. Only when a test needs the bundled worker and resources, use `pnpm build:desktop:debug` and its `.app`, never while the dev app is running.
- **Real LLMs:** use the connection named **Testing (AI Agents may use this too)** (under Providers in the composer's model picker). It costs the repo owner nothing and exists for agents' testing. Use it through the app; never read its key from `secrets.json`. Don't send prompts through any other connection or subscription sign-in, as those may cost the owner money. For deterministic output, such as testing cancellation, use `pnpm mock:provider`: it hangs on "wait until stopped".

## Out of scope

Don't propose these as fixes: non-image attachments, embedded editors or terminals, permission prompts, automatic merging, notarization, auto-updates, other platforms. MCP isn't built in; a package such as `pi-mcp-adapter` provides it. `todo.md` (gitignored) is the owner's idea list: ask before starting anything from it.
