# AGENTS.md

WackCode is a local macOS desktop app for the [Pi coding agent](https://www.npmjs.com/package/@earendil-works/pi-coding-agent), built with Tauri 2, React 19 + TypeScript, and Rust. Pi is pinned in `worker/package.json`. `README.md` describes user-facing behavior; update it when that behavior changes.

**Keep this file short:** it is sent with every agent request. Explain features in comments next to their code, which is how this codebase already documents its invariants. Add a line here only for a rule that spans files and that an agent would break without being told.

## Ground rules

- macOS on Apple Silicon only. Unix-only APIs (`killpg`, `open(1)`) are deliberate; don't "fix" them for other platforms.
- pnpm only (version pinned in `package.json`). No npm/yarn, no `package-lock.json`.

## Architecture

- **`src/`**: React renderer. `App.tsx` owns all cross-cutting state and is the only place that listens to Tauri events. `api.ts` is the only bridge to Rust (one typed `invoke` wrapper per command). Components are presentational; `SettingsPage.tsx` is the one exception that calls `api` directly.
- **`src-tauri/src/`**: Rust commands, worker lifecycle, persistence, and credentials.
- **`worker/src/`**: Node wrapper around Pi; `protocol.ts` defines the NDJSON protocol and `builtin/` holds built-in extensions.

## Cross-file rules

- **Shared types:** a type that crosses layers changes in `models.rs`, `types.ts`, and (if the worker sees it) `protocol.ts` together. New optional Rust fields need `#[serde(default)]` so existing `wackcode.json` files load; runtime-only fields (like `ProjectRecord.branch`) also need `skip_deserializing`.
- **New Tauri command:** register in `lib.rs`, add a typed wrapper in `api.ts`, and add any new plugin permission in `capabilities/default.json`.
- **Worker protocol change:** update `protocol.ts`, Rust payloads and handlers, `WorkerEvent` in `types.ts`, and the event switch in `App.tsx` together. Run `pnpm test:worker` afterwards.
- **Command queue:** commands that must reach an active run must bypass the serial prompt queue or they deadlock; see the dispatch comments in `worker/src/index.ts`.
- **Incremental snapshots:** new fields that change mid-run need `SnapshotDelta` and `applySnapshotDelta` (`chat-utils.ts`) updates. Preserve the full-snapshot fallback and unchanged message object identities for transcript memoization.
- **Worker restarts:** keep settings applied live out of the `worker.rs` fingerprint; changing it kills and respawns the worker even mid-run. Idle workers are also stopped on purpose (a reaper in `worker.rs`, 15 minutes after their last output; the open chat, tracked by `SelectedTask`, is exempt), so any command that talks to a worker must reach it through `ensure_worker`, which respawns it.
- **Per-chat lock:** any command that sends a chat work, moves its conversation, or touches its checkpoints holds that chat's `TaskLocks` entry.
- **Tools:** never pass `tools:` to the chat's `createAgentSession`: Pi treats it as a registry filter and erases extension tools. Apply the user's denylist with `setActiveToolsByName`.
- **Planning modes:** Plan and Ultra Plan both count as planning: use `mode !== "build"` (`isPlanMode` in the frontend). Keep the Plan contract byte-identical: edits re-append it to saved chats, so do not just update the test's pinned hash. Custom prompts layer on top; keep Settings defaults in `src/promptDefaults.ts` aligned with the worker builders.
- **Built-ins:** adding or renaming one in `worker/src/builtin/` also requires updating `BUILTIN_EXTENSIONS` in `PackagesSection.tsx`. Preserve the separate built-in and MCP tool-switch rules in `worker/src/index.ts`.
- **Session tree and checkpoints:** read the invariants at the top of `worker/src/tree.ts` and `src-tauri/src/checkpoints.rs` before changing either. Checkpoints must never write to the project's own Git repository.
- **Window:** keep `tauri.conf.json` free of `backgroundColor`; `glass.rs` controls the native background. Keep `dragDropEnabled: false` so native handling does not swallow composer image drops.
- **Agent persona:** the app's own copy never calls the agent "Pi"; the name comes from `appearance.agentName`, resolved in `src/agentName.ts` (default "WackCode"). Genuine Pi product references (catalogue, sign-in, the worker process) keep the real name.

## Security (don't weaken)

- **Network:** no telemetry, account, backend, updater, automatic model discovery, or traffic on launch or on a timer. The app contacts only user-configured provider endpoints, Pi's subscription endpoints (after an explicit sign-in; token refresh during prompts), the npm registry plus a package's own source while the user browses or installs one, pi.dev's catalogue page while the user browses Settings › Skills (redirects within pi.dev only), public pages the agent reads with the built-in `web_fetch` during a run (public IPs only, user-switchable), and the HTTP/SSE MCP servers the user adds (their URL's origin only). Never add a network origin silently; call it out explicitly.
- **CSP** stays `default-src 'self'` with IPC-only `connect-src`. The renderer never fetches; remote calls go through Rust or the package-manager process.
- **Credentials:** API keys and MCP header/env values (`mcp:<id>`) persist only in `secrets.json` (0600); OAuth credentials only in per-provider 0600 `auth.json` files. A worker receives them only over its stdin (`init`, `set_subagents`, `set_mcp`, first-prompt `autoTitle`) and only the auth paths it needs. Credentials never go in argv, env vars, `wackcode.json`, Tauri events, or the renderer; the one exception is an MCP env value, which becomes its own stdio server's environment. Workers get the login-shell environment (`shell_env.rs`), with provider keys stripped afterwards. Keep the provider env-key stripping in `worker.rs` and both redaction layers (`safeError` in the worker, `redact_and_limit` in Rust). The macOS Keychain backend was removed on purpose; don't bring it back.
- **Pi lockdown:** the worker keeps Pi's telemetry, version check, network model refresh, and all auto-discovery disabled (every `no*` flag stays `true`). Packages load only from explicit paths Rust passes for trusted packages, so a project's own `.pi/` never runs. User skill folders reach a worker only as the absolute roots `skills::payload` names (`~/.agents/skills`, then folders the user switched on); a project's own skill folders never load, and Settings writes only inside `~/.agents/skills`. Project context files such as `AGENTS.md` load on purpose.
- **Asset protocol** is enabled only for `$APPDATA/backgrounds/*`. Only `choose_background_image` writes there (native picker opened in Rust, magic-byte check); the renderer never passes it a path. Don't widen the scope.
- **Package trust:** an installed extension is unsandboxed code inside the worker that holds the API key. `trusted_at` is set only by a user-confirmed `install_package` or `trust_package`. Never default it, and keep `sync_packages` from trusting packages that arrive by any other route.

## Design

- **Animation:** expressive motion is core to the app's identity. Favour lively, considered transitions and feedback that make interactions feel responsive and full of personality. Respect reduced-motion preferences.
- **Customisation:** make the app feel personal and adaptable to the user. Design features to work across the user's themes, backgrounds, and appearance settings rather than assuming the defaults.
- **Personality:** WackCode should feel like a distinctive macOS app, never a generic web app or generic AI-generated design. Make deliberate choices in layout, typography, details, and interaction that reinforce its own character.
- **Theme:** themeable colours are tokens computed in `src/theme.ts` (`TOKENS`) with matching `:root` defaults in `styles.css`; never write a raw colour below `:root` (`theme.test.ts` guards it). Over an image or Liquid Glass only the shell (window, sidebar, header, side panels) turns see-through; content surfaces stay solid.

## Conventions

- **Rust:** fallible functions return `Result<T, String>` whose error is a user-facing sentence; no `anyhow`/`thiserror`. Every `wackcode.json` write goes through `MetadataState::mutate`. Git goes through the system `git` CLI (no `git2`).
- **React:** update state immutably via `patchTask` / `patchRuntime` / `setData`. Reuse the primitives in `src/components/ui/`. Destructive actions go through `ConfirmDialog` with `danger`.
- **CSS:** one stylesheet, `src/styles.css`: add rules to the matching section, use `--wc-*` variables and the existing button classes, no Tailwind or other CSS system. Keep the `data-tauri-drag-region` strips working (overlay title bar), `aria-label` on icon-only buttons, and the `prefers-reduced-motion` block.
- **Tests:** pure logic in colocated `*.test.ts`; components with Testing Library role-based queries; worker behavior in `worker/src/*.test.ts` (a real worker against an inline mock provider); Rust in inline `#[cfg(test)]` modules with `tempfile`.

## Commands

| Command | When |
|---|---|
| `pnpm check` | Fast compile check: frontend, worker, `cargo check`. |
| `pnpm test` | All suites; or `test:web`, `test:worker`, `test:rust` individually. |
| `pnpm build:worker` | After editing `worker/src/` (the dev app runs `worker/dist/`). |
| `pnpm dev:background` / `pnpm dev:stop` | Start / stop the live dev app (below). |
| `pnpm mock:provider` | Deterministic mock provider at `http://127.0.0.1:43127/v1` (Chat Completions). |

## Testing the running app

- **Start** with `pnpm dev:background`: idempotent, prints `Ready` once the app is up, logs to `/tmp/wackcode-dev.log`. **Stop** with `pnpm dev:stop`. Don't launch the app any other way, and don't restart it between tests: frontend edits hot-reload, Rust edits rebuild and relaunch the app automatically, and worker edits need `pnpm build:worker` plus a new chat.
- **Drive** the window by its bundle id, `com.wackcode.desktop`. Only when a test needs the bundled worker and resources, use `pnpm build:desktop:debug` and its `.app`, never while the dev app is running.
- **Real LLMs:** use the connection named **Testing (AI Agents may use this too)** (under Providers in the composer's model picker). Use it through the app; never read its key from `secrets.json`. Don't send prompts through any other connection or subscription sign-in, as those may cost the owner money. For deterministic output, such as testing cancellation, use `pnpm mock:provider`: it hangs on "wait until stopped".

## Out of scope

Don't propose these as fixes: non-image attachments, embedded editors or terminals, permission prompts, automatic merging, notarization, auto-updates, other platforms. `todo.md` (gitignored) is the owner's idea list: ask before starting anything from it.
