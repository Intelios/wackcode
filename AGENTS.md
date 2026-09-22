# AGENTS.md — WackCode contributor guide for coding agents

WackCode is a local macOS desktop interface for the [Pi coding agent](https://www.npmjs.com/package/@earendil-works/pi-coding-agent), built on Tauri 2, React 19, TypeScript, and Rust. This file tells you how the repo fits together and which conventions you must follow when changing it.

Read `README.md` first for the user-facing feature set and security posture; this file is the engineering-side contract.

## Hard facts

- **Platform:** macOS on Apple Silicon only. `scripts/prepare-runtime.mjs` hard-fails elsewhere. Unix-only APIs (`nix::killpg`, `open(1)`) are used deliberately — do not file or "fix" these as cross-platform bugs.
- **Package manager:** pnpm 11.5.0 (pinned in `package.json` → `packageManager`). Do not use npm/yarn; do not add a root `package-lock.json`.
- **No telemetry, no account, no backend, no updater, no automatic model discovery.** Anything that "phones home" from the app itself is out of scope. Outbound traffic is limited to three user-initiated destinations: user-configured OpenAI-compatible provider endpoints; the public npm registry (`registry.npmjs.org`), only while the user browses or installs a package; and the package's own npm or git source during that install. Nothing contacts any of them on launch or on a timer.
- **No AGENTS.md loading in the bundled worker** — Pi's project instruction files (like this one) still load in the *user's* projects, but this repo's guide is for you, the agent working on WackCode itself.

## Architecture

Three layers, one repo, exactly one bridge between each pair:

```
React frontend (src/)                Tauri v2 bridge                Rust backend (src-tauri/)
  api.ts  ──invoke(snake_case)──►   commands.rs (32 commands)  ──►  storage.rs / secrets.rs / git.rs
  App.tsx ◄──listen("worker-event")──  worker.rs (per-task Node child process)
                                                   │ stdin/stdout NDJSON
                                                   ▼
                                        worker/src/index.ts  (Pi SDK wrapper)
                                                   │ HTTPS
                                                   ▼
                                  user-configured OpenAI-compatible endpoint
```

A third, short-lived process handles packages. It is spawned per operation by `worker::run_manager`, runs `worker/src/manager.ts` on the same bundled Node, and wraps Pi's `DefaultPackageManager` against a shared store at `<app-data>/pi/`. It **never receives an API key**, the provider env keys are stripped before spawn, and unlike a task worker it does not set `PI_OFFLINE` — installing is the one thing it exists to do. Its protocol lines are prefixed with `\x1e` because Pi spawns npm with inherited stdio and npm's own output lands on the same stream.

Data flow for one prompt: `App.tsx` → `api.prompt` → `commands::prompt` (validates, ensures worker, generates `runId`) → `worker.rs` writes an NDJSON `prompt` line to the worker's stdin → `worker/src/index.ts` drives the Pi session → Pi streams back → worker emits `partial` / `activity` / `snapshot` / `run_state` NDJSON lines on stdout → `worker.rs::handle_worker_line` parses, persists key fields to `wackcode.json`, and re-emits every line as the single Tauri event `"worker-event"` → `App.tsx` dispatches on `payload.type` into `patchRuntime` / `patchTask`.

## Commands

| Command | What it does | When to use it |
|---|---|---|
| `pnpm install` | Install workspace deps (root app + `worker/`). | After clone, or when `pnpm-lock.yaml` changed. |
| `pnpm prepare:runtime` | Download Node 24.18.0 (darwin-arm64) per `runtime-lock.json`, verify SHA-256, place the binary at `src-tauri/binaries/wackcode-node-aarch64-apple-darwin` and stage npm from the same tarball into `src-tauri/resources/npm/`. | Once per machine, and after `runtime-lock.json` changes. Required for `tauri dev` and `tauri build`, and for installing packages. |
| `pnpm dev:desktop` | `tauri dev`: builds the worker, starts Vite on port 1420, launches the app. Dev worker = `worker/dist/index.js`, dev Node = `$WACKCODE_NODE_PATH` or `node` on PATH. | Day-to-day development. |
| `pnpm build:desktop` | `prepare:runtime` then `tauri build` → `src-tauri/target/release/bundle/macos/WackCode.app`. | Producing the self-contained app. Requires the pinned runtime downloaded above. |
| `pnpm build:web` | `tsc -b && vite build` → type-check the frontend and emit `dist/`. | Catching TS errors without launching the app; also what `beforeBuildCommand` runs. |
| `pnpm build:worker` | `tsc` the worker → `worker/dist/`. | After editing `worker/src/` (also run by `dev:desktop` and the worker tests). |
| `pnpm prepare:worker` | Stage a self-contained worker package into `src-tauri/resources/worker/` via `pnpm deploy`. | Only as part of release bundling (`beforeBuildCommand` already does this). |
| `pnpm test` | `test:web` → `test:worker` → `test:rust` in sequence. | Before any PR. |
| `pnpm test:web` | `vitest run` on `src/**/*.test.{ts,tsx}` (jsdom). | Frontend unit/component changes. |
| `pnpm test:worker` | Builds then runs `worker/src/*.test.ts` against an inline mock OpenAI server, plus `manager.test.ts` against a local fixture package (no network). | Worker, manager, or protocol changes. |
| `pnpm test:rust` | `cargo test --manifest-path src-tauri/Cargo.toml`. | Rust changes. |
| `pnpm check` | `build:web` + `build:worker` + `cargo check`. | Fast "does everything still compile" pass. |
| `pnpm mock:provider` | Standalone mock endpoint on `http://127.0.0.1:43127/v1` (streams a fixed write-tool round trip; hangs on "wait until stopped" for cancellation testing). | Manual UI testing without a real provider. Configure as a Chat Completions connection. |

## Repository layout

```
src/            React frontend (Vite entry: ../index.html → src/main.tsx)
  App.tsx       Root orchestrator. Owns ALL state (~760 lines). The only file that listens to "worker-event".
  api.ts        The only bridge to Rust: a flat `api` object of typed one-line `invoke` wrappers.
  types.ts      Shared contract with Rust/worker. Must mirror models.rs and worker/src/protocol.ts.
  model-utils.ts, chat-utils.ts, tool-utils.ts, attachment-utils.ts   Pure helpers; put testable logic here, not in components.
  hooks/        useSmoothText, useFollowScroll (Transcript-only, no tests).
  components/   Presentational components. Only SettingsPage.tsx is allowed to import api directly.
  components/ui/  Reusable primitives: Popover, Menu, MenuButton, Select, Tooltip, ConfirmDialog.
  styles.css    Single global stylesheet, CSS variables on :root, section banner comments. No Tailwind.
  test/setup.ts Vitest setup (jest-dom).
worker/         Node child process wrapping @earendil-works/pi-coding-agent 0.86.1 (pinned).
  src/index.ts    Main loop: init → session create/restore → prompt stream → abort/shutdown.
  src/protocol.ts Command/event types. Single source of truth for the task worker's stdin/stdout protocol.
  src/builtin/    Built-in extensions (inline factories, always on): ask_user_question, plan-mode, todo.
  src/manager.ts  Short-lived package-manager process wrapping Pi's DefaultPackageManager.
  src/manager-protocol.ts  Its protocol. Frames are \x1e-prefixed because npm shares stdout.
  src/framing.ts  JsonLineDecoder. Splits on LF only; U+2028/U+2029 inside strings are safe. Joins a line's
                  chunks once, so multi-megabyte prompt lines (images) stay linear.
  dist/           tsc output. Build artifact, gitignored. Never edit by hand, never commit.
src-tauri/
  src/main.rs     3-line entry calling wackcode_lib::run().
  src/lib.rs      Tauri builder, plugin init, .manage(WorkerState), generate_handler![...32 commands],
                  terminate_all on RunEvent::Exit.
  src/commands.rs All 32 #[tauri::command] fns + validation helpers + tests. Error strings are user-facing.
  src/models.rs   Serde records that must stay in sync with src/types.ts.
  src/worker.rs   Spawns per-task Node workers, NDJSON bridge, crash detection, stderr redaction.
  src/git.rs      Shells out to system git (no git2). Worktrees, porcelain parsing, diff previews.
  src/storage.rs  MetadataState: atomic JSON writes of wackcode.json, interrupted-task recovery.
  src/secrets.rs  SecretStore: secrets.json with 0600 perms. The ONLY place API keys are persisted.
  tauri.conf.json identifier com.wackcode.desktop, CSP default-src 'self', externalBin wackcode-node,
                  bundle resources/worker, macOS ad-hoc signing.
  capabilities/default.json   Tauri permissions (core:default, dialog:allow-open, clipboard write-text).
  binaries/       Bundled Node binary lands here (gitignored except .gitkeep).
  resources/worker/  Staged by pnpm deploy (gitignored).
scripts/        prepare-runtime.mjs, prepare-worker.mjs, mock-provider.mjs. Plain Node ESM, no deps.
runtime-lock.json  Pins Node 24.18.0 + records the Pi version (informational; the enforced pin is worker/package.json).
```

## Conventions that bite

### Type contract across three layers
Adding or changing a field on a task/provider/project means **four** files, in sync:
1. `src-tauri/src/models.rs` — the serde struct (`#[serde(rename_all = "camelCase")]`, `#[serde(default)]` on any new optional field so old `wackcode.json` files keep loading, `skip_deserializing` for runtime-only fields like `ProjectRecord.branch`).
2. `src/types.ts` — the TS mirror.
3. `src-tauri/src/commands.rs` — any command that constructs/returns it.
4. `src/App.tsx` — state handling.

`TaskStatus` is the one exception: it serializes `snake_case` (`idle`, `running`, `stopping`, `interrupted`, `error`).

### Adding a new Tauri command (exact order)
1. `models.rs`: declare request/response structs if needed.
2. `commands.rs`: write `#[tauri::command] async fn snake_case_name(state: State<'_, MetadataState>, ...) -> Result<T, String>`. Validation goes in the helper cluster at the bottom of the file (`required`, `validate_base_url`, `validate_models`, `validate_thinking`…), not inline. Error strings are complete sentences shown to users.
3. `lib.rs`: add to `generate_handler![...]`.
4. `api.ts`: add one line — scalar args for simple inputs, a single `{ input: {...} }` object for multi-field inputs (matches existing style exactly).
5. `App.tsx` (or the one component that needs it): call `api.*`, then update state immutably via `patchTask` / `patchRuntime` / `setData`. Never mutate `data` in place. Handle errors per the tiers below.

### The 32 commands (full surface, mirrored in `api.ts`)

| Command | File:line | Summary |
|---|---|---|
| `bootstrap` | commands.rs:20 | Full `AppData` + `app_data_path`; refreshes `has_api_key` and live branch names. |
| `save_provider` | commands.rs:35 | Create/update a connection; restarts workers using it. |
| `delete_provider` | commands.rs:89 | Refuses while tasks reference it. |
| `discover_models` | commands.rs:114 | `GET {baseUrl}/models` (bearer, 20 s), sorted IDs. |
| `set_tool_config` | commands.rs:146 | Persist the tool denylist; pushes `set_tools` to every running worker without restarting them. |
| `add_project` | commands.rs:144 | Canonicalize folder; idempotent; records git root/HEAD. |
| `create_task` | commands.rs:170 | New chat; optional worktree else per-chat scratch folder. |
| `configure_task` | commands.rs:244 | Change provider/model/thinking; refuses while running; restarts worker. |
| `open_task` | commands.rs:273 | Ensure worker, then request `snapshot`. |
| `prompt` | commands.rs:281 | Configure, ensure worker, send `prompt` with fresh `runId`. |
| `stop_task` | commands.rs:302 | Send `abort`. |
| `archive_task` / `unarchive_task` | commands.rs:307 / 319 | Terminate worker + flag; clear flag. |
| `rename_task` | commands.rs:329 | Validated non-empty. |
| `delete_task` | commands.rs:340 | Terminate worker, drop record, remove agent/session/scratch dirs + worktree. |
| `convert_task_to_worktree` | commands.rs:355 | Only before the first message / before `session_file` is set. |
| `remove_project` | commands.rs:393 | Refuses while non-archived chats exist; deletes its tasks. |
| `git_changes` | commands.rs:437 | Staged + unstaged + untracked diffs for the task workspace. |
| `reveal_task` / `reveal_path` | commands.rs:445 / 454 | macOS `open` on the workspace / an arbitrary path. |
| `list_packages` / `refresh_packages` | commands.rs | Cached package list / re-read the shared store via the manager process. |
| `install_package` / `trust_package` | commands.rs | Install (refuses without `trusted`) / grant trust to a package already on disk. |
| `remove_package` / `update_packages` | commands.rs | Remove, update. Both restart every worker. |
| `set_package_resources` | commands.rs | Per-resource toggles, persisted as Pi's `PackageSource` object form. |
| `search_packages` / `package_details` | commands.rs | npm registry search for `keywords:pi-package` / one package's manifest. |
| `respond_extension_ui` | commands.rs | Answer a dialog an extension raised (plain value or structured `answers`). |
| `set_task_mode` | commands.rs:644 | Persist Build/Plan on the task record and push `set_mode` to a live worker. |
| `export_plan` | commands.rs:665 | Write the ready plan to `PLAN.md` in the workspace; refuses to overwrite. |

New plugin commands also need an entry in `src-tauri/capabilities/default.json`.

### Worker protocol changes (three files deep, always together)
1. `worker/src/protocol.ts` — extend `WorkerCommand` / `WorkerOutput` unions.
2. Rust side — `worker.rs` for payload construction (`init` shape at the top of the worker-facing code) and `commands.rs` for the call sites that send the command; update `handle_worker_line` for new event types.
3. Frontend — `src/types.ts` `WorkerEvent` union + the single `listen("worker-event")` switch in `App.tsx`.

Rules that must not regress:
- **Abort bypasses the queue.** A prompt holds the worker's serial command queue until the agent settles; `abort` is handled immediately (`worker/src/index.ts`). Never route abort through the same queue as prompt.
- **Throttling:** snapshots are coalesced with a 32 ms timer; partials are throttled to ~16 ms. Full snapshots at `message_end` are authoritative.
- **Crash handling:** any worker exit while still registered is unexpected — Rust marks the task `Interrupted` and emits a redacted, 1 000-char-capped stderr tail as `worker_error`.

### Secrets discipline (security-critical)
- API keys live **only** in `secrets.json` (mode 0600) via `secrets.rs`, and are delivered to the worker **only** through the stdin `init` command. Never argv, never env vars.
- Rust strips all 15 known provider API-key env vars before spawning the worker (`worker.rs`).
- Redaction happens twice: `safeError` in the worker, then `redact_and_limit` in Rust. Preserve both layers.
- A dedicated test in `storage.rs` asserts `wackcode.json` contains no secrets. The earlier macOS-Keychain backend was deliberately removed (commit `ea25c08`) — do not reintroduce keyring unless that decision is explicitly revisited.

### Pi feature surface the worker hard-disables
`worker/src/index.ts` sets `PI_TELEMETRY=0`, `PI_SKIP_VERSION_CHECK=1`, `PI_OFFLINE=1`, `allowModelNetwork: false`, `refreshOnCreate: false`, `cacheWarming: "off"`, `noExtensions`, `noSkills`, `noPromptTemplates`, `noThemes`, `defaultProjectTrust: "never"`, and Rust strips provider env keys. **Kept on purpose:** project instruction/context files (`noContextFiles: false` — so an `AGENTS.md` in the user's project still loads), compaction, and auto-retry.

The `no*` flags stay `true` even now that packages load. They switch off *discovery*, not loading: `DefaultResourceLoader` still honours `additionalExtensionPaths` / `additionalSkillPaths` / `additionalPromptTemplatePaths` / `additionalThemePaths` when they are set (`resource-loader.js`, the `noExtensions ? cliEnabledExtensions : merge(...)` branches). Rust fills those from `init.resources` with the enabled paths of trusted packages only. Nothing else can execute — a project's own `.pi/extensions` is unreachable regardless of trust settings.

### Images
Images are Pi's own `ImageContent` (`{ type: "image", data: <base64>, mimeType }`) at **every** layer: `src/types.ts`, `models.rs` (`ImageContent`, `PromptInput.images`), and `worker/src/protocol.ts` (`prompt.images`, the same field Pi's RPC `prompt` takes). Nothing converts them on the way, and Pi stores them in its session JSONL in that shape, so sessions stay readable by Pi itself.
- **Vision is Pi's `input` flag.** `ModelRecord.vision` (default `false`, confirmed by the user in Settings like limits) becomes `input: ["text", "image"]` in the worker's `models.json`; off means `["text"]`, and Pi then swaps any image for its "image omitted" placeholder and stops `read` returning image content. Toggling it changes the provider record, so the worker fingerprint respawns affected workers. Rust also refuses a prompt with images for a non-vision model (`require_vision`), because Pi's silent downgrade would hide the problem.
- **Limits live in two places and must match:** `src/attachment-utils.ts` and `validate_images` in `commands.rs` (8 images, 20 MB each, PNG/JPEG/GIF/WebP). A message is still required alongside images: Pi always sends a text part.
- **Resizing happens in the worker**, not the UI: `prepareImages` repeats Pi's CLI `processImage` with the exported `resizeImage` and Pi's defaults (2000 px, 4.5 MB), because `session.prompt` forwards images untouched.
- **Snapshots never carry originals.** User-message image blocks become `{ type: "image", mimeType, imageId, thumbnail? }`; the worker makes a ≤512 px `data:` preview one at a time, caches it in a `WeakMap` keyed by Pi's (stable) block object, and re-snapshots when it lands. `Transcript`'s memo signature uses `imageId`, never the preview data. Tool-result images (e.g. `read` on a PNG) still reach the model but are not rendered; they are dropped from normalization so they can't overwrite the call's text result.
- Drag and drop uses HTML5 events, which is why `tauri.conf.json` sets `dragDropEnabled: false` (the native handler would otherwise swallow file drops). The CSP already allows `data:` images; do not add `blob:` or remote origins for this.

### Extension dialogs
Extensions may call `ctx.ui.select/confirm/input/editor`. The worker implements a headless `ExtensionUIContext` modelled on Pi's RPC mode and bridges those four to React modals; ambient TUI affordances (`setStatus`, `setWidget`, `setFooter`, themes, custom components) are accepted and discarded. A fifth `method: "questions"` (structured questionnaires, `answers` on the response) exists for the built-in `ask_user_question` tool and is rendered by `QuestionDialog.tsx`, not `ExtensionDialog.tsx`.

**`extension_ui_response` must bypass the worker's command queue, exactly like `abort`.** An extension awaiting a dialog is usually doing so inside a `tool_call` handler, which runs inside `session.prompt()`, which owns the serial queue — queueing the answer behind it deadlocks the run outright. There is a regression test for this (`integration.test.ts`, "answers an extension dialog raised mid-prompt"); removing the bypass makes it time out.

### Built-in extensions
`worker/src/builtin/` ships extensions compiled into the worker itself. They are passed to `DefaultResourceLoader` as `extensionFactories`, so they load even with `noExtensions: true`, appear in the tool catalogue as `source: "wackcode"`, are hidden from the Settings toggles, and are **immune to the tool denylist** — a disabled `plan_mode_complete` would silently break Plan mode. They talk to React through a small `BuiltinHost` bridge (`askQuestions`, `publishPlanState`, `publishTodoState`) over the normal `extension_ui_*` channel; they never touch `ctx.ui` dialogs directly. Settings → Packages lists them under "Built-In" for visibility (`BUILTIN_EXTENSIONS` in `PackagesSection.tsx` — keep it in sync when a built-in is added or renamed).

Three are registered in `builtin/index.ts`:
- **`ask-user-question`** — `ask_user_question` tool (1–3 questions, 2–4 options each, optional `multiSelect`, free-form "Other"). Available in every mode.
- **`plan-mode`** — ported from `@narumitw/pi-plan-mode` 0.58.3 (MIT). Enforces a read-only policy while Plan mode is active (mutating tools and package tools blocked; bash limited to a fail-closed allowlist; `gh`/`git` validated per segment — upstream's whole-command prefix shortcut was removed because it let `--web` and command substitutions bypass validation). `plan_mode_complete` publishes `plan_state` and persists it via `pi.appendEntry`, so it survives worker restarts; a `<proposed_plan>` block is the fallback when the model skips the tool. Mode switches are model-visible via a hidden `custom` contract message, not a system-prompt rewrite. `TaskRecord.mode` is the durable hint; on init mismatch the record wins.
- **`todo`** — ported from `@juicesharp/rpiv-todo` 2.11.0 (MIT). A `todo` tool (create/update/list/get/delete/clear; `pending → in_progress → completed` plus a `deleted` tombstone; `blockedBy` dependencies with additive merge and cycle rejection) rendered as a live panel above the composer (`TodoPanel.tsx`). No disk writes, like upstream: every tool result embeds the full list in `details` (versioned `TodoDetails`) and `replayFromBranch` rebuilds state on `session_start`/`session_compact`/`session_tree`, so the list survives worker restarts and compaction; `todo_state` events plus `getSnapshot().todoState` feed the UI. Allowed in Plan mode — it mutates only its own list, never the workspace. Adaptations from upstream: the TUI overlay and `/todos` command are gone (the panel is the view), i18n is dropped, `owner`/`metadata` are trimmed from the schema, and the sid-keyed multi-session store collapses to one closure (one session per worker). The panel fades completed tasks behind "+N done" at each run boundary.

### Tool selection (read this before touching tools)
**Never pass `tools:` to `createAgentSession`.** In Pi 0.86.1 it sets `allowedToolNames`, a hard *registry* filter (`agent-session.js` `isAllowedTool`) — not just an initial active set. Anything not listed is erased from the registry, so extension-contributed tools disappear from `getAllTools()` entirely and can never be shown as an off toggle. `excludeTools:` erases in the same way; it is reserved for tools that must not exist at all, currently just `powershell` (registered on every platform, meaningless on macOS-only WackCode).

The user's selection is a **denylist**, so a tool a newly installed package adds is on by default. It arrives as `init.disabledTools`, is applied with `session.setActiveToolsByName(...)` right after session creation, and is changed live by the `set_tools` command — `setActiveToolsByName` takes effect on the next agent turn, so tool changes **never restart a worker** (unlike provider/model, which are in the `worker.rs` fingerprint).

`grep` and `find` shell out to `rg` and `fd`. Pi normally downloads those on demand, but `PI_OFFLINE=1` blocks that, so the worker probes for them (Pi's bin dir, then `PATH`) and reports missing ones as `available: false` with a reason rather than offering the model a tool that errors mid-task. A packaged `.app` launched from Finder gets a minimal `PATH`, so this is the common case, not the edge case — bundling those binaries is tracked with the npm bundling work.

### Git integration
- Uses the **system `git` CLI** via `std::process::Command` — there is intentionally no `git2` crate.
- Worktrees: branch `wackcode/<slug>-<taskId[..8]>`, destination `<app-data>/worktrees/<taskId>`. Require at least one commit (`git_has_head`) and can only be chosen before the first message (Rust refuses once `session_file` is set).
- `git_changes` parses `status --porcelain=v1 -z` (rename records are two NUL-separated entries) and concatenates staged + unstaged diffs with `# Staged changes` / `# Working tree changes` headers; previews are capped at 240 KB with a `truncated` flag; binaries are detected by NUL bytes in the first 8 KB and rendered as `Binary file · N bytes`.

### Frontend rules
- Only `App.tsx` holds cross-cutting state and only `App.tsx` subscribes to Tauri events. Components are presentational: named `export function`, in-file `interface Props`, `onX` callback props, `import type` for types.
- `SettingsPage.tsx` is the sole component that imports `api` directly — treat this as a pragmatic exception, not a new pattern.
- Error surfaces, pick by scope: `globalError` toast (app-wide), per-task `runtime.error`/`task.lastError` banner, or local form `error`/`notice` state. All use `String(reason)`; optimistic updates are rolled back in `catch`.
- Destructive actions go through `ConfirmDialog` with a `danger` variant, driven from a single `ConfirmState` in `App.tsx`.
- Styling: add rules to the matching banner section of `src/styles.css`; reuse `--wc-*` variables and existing button classes (`primary-button`, `secondary-button`, `danger-button`, `icon-button`, `ghost-button`) instead of inventing new ones. Respect keyboard shortcuts already on ⌘N / ⌘O / ⌘, / ⇧⌘C and the `data-tauri-drag-region` strips for the overlay titlebar.
- Accessibility: role/name-based queries in tests, `aria-label` on icon-only buttons, `:focus-visible` uses `color-mix`, and a `prefers-reduced-motion` block exists — don't break it with new animations.

### Rust rules
- No `anyhow`/`thiserror`; every fallible function returns `Result<T, String>` with a **user-facing sentence** as the error. Mutex poison maps to `"<Thing> lock was poisoned"`.
- All metadata writes go through `MetadataState::mutate(...)` → atomic `.tmp` + `rename`. Never hand-roll a partial write.
- IDs are `Uuid::new_v4()` strings; timestamps `chrono::Utc::now().to_rfc3339()`.
- Managed state via `.manage(...)`; access with `State<'_, MetadataState>` / `State<'_, WorkerState>`. `WorkerState` is a `std::sync::Mutex<HashMap<task_id, WorkerProcess>>`; `WorkerProcess.stdin` is a `tokio::sync::Mutex` because it's held across `.await`. One worker per task; changing provider or model kills and respawns it (fingerprint match in `worker.rs`).

### Build artifacts — never commit
`worker/dist/`, `src-tauri/resources/worker/`, `src-tauri/binaries/*` (except `.gitkeep`), `dist/`, `*.tsbuildinfo`, `src-tauri/target/`. `todo.md` is also gitignored on purpose.

## Testing guidance

- **What to test where:** pure logic → colocated `*.test.ts` next to `model-utils.ts`/`chat-utils.ts`/`tool-utils.ts`; UI → Testing Library component tests (`ModelPicker.test.tsx`, `ProjectBar.test.tsx`, `ui/Select.test.tsx` show the style: `afterEach(cleanup)`, `screen.getByRole`, `fireEvent`, small controlled `Harness` wrappers); worker protocol → `worker/src/*.test.ts` (real spawned worker + inline mock provider on an ephemeral port); Rust → inline `#[cfg(test)] mod tests` using `tempfile`.
- **What's currently untested (don't mimic a pattern that isn't there):** `App.tsx`, `Transcript.tsx`, both hooks, and `api.ts` have no tests. `Composer.tsx` is covered for attachments only. Add coverage deliberately if you touch them, but don't treat existing silence as license to ship regressions.
- The worker integration suite exercises properties the UI cannot: concurrent sessions with overlapping model IDs, per-credential isolation, thinking-level wire mapping, cancellation mid-stream, and session restoration without replay. Run it after any protocol change.
- For a manual end-to-end check: `pnpm mock:provider`, then add `http://127.0.0.1:43127/v1` as a Chat Completions connection in the app.

## Out of scope for this milestone (do not propose as "fixes")

Subscription login, non-image attachments, embedded editors and terminals, permission prompts, automatic merging, notarization, auto-updates, and any non-macOS platform. MCP is not a WackCode feature, but is reachable by installing a package such as `npm:pi-mcp-adapter`.

**In scope but not built:** per-project package overrides (Pi's project scope lives in the user's own `.pi/`, which is untrusted here by design), skill authoring, and theme selection. `todo.md` (gitignored) tracks longer-term Pi-surface ideas; coordinate before picking those up.

## Security posture to preserve

App-initiated network calls are limited to user-configured provider endpoints (`GET {baseUrl}/models` and chat completions) and, only on an explicit user action, the npm registry plus the chosen package's npm or git source. CSP is `default-src 'self'` with only `ipc:` connect-src and **must stay that way**: the renderer never fetches, so every remote call goes through Rust `reqwest` or the package-manager process. A new network origin still requires an explicit note in the PR. The worker environment is sanitized, credentials travel only over a private stdin pipe, and local state lives under the app's macOS application-data directory.

**Installed packages are the largest trust boundary in the app.** An extension is ordinary local code running inside the task worker — the process holding the decrypted provider API key. The trust dialog is informed consent, not containment: it names the source and states the API-key exposure, and `trusted_at` gates whether a package's paths ever reach a worker (`worker::resource_paths`). Do not weaken that gate. **`trusted_at` is only ever set by an explicit `install_package` or `trust_package` the user confirmed** — `sync_packages` takes a `newly_trusted` argument precisely so that a package appearing in the shared store by any other route (a restored `wackcode.json`, or an already-installed extension editing `settings.json`) stays inert and is shown in Settings as "Not enabled" until reviewed. Never default it to the current time. Project-local `.pi/` remains unloadable by construction, not by a flag: the worker keeps every `no*` discovery option on and loads only the absolute paths Rust hands it.
