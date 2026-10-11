# Testing

## Suites

| Command | Covers | Where tests live |
|---|---|---|
| `pnpm test:web` | Renderer logic and components (Vitest + jsdom) | Colocated `src/**/*.test.ts(x)`; components use Testing Library role-based queries |
| `pnpm test:worker` | The worker end to end: a real worker process against an inline mock provider | `worker/src/*.test.ts` (`integration.test.ts` is the big one) |
| `pnpm test:rust` | Rust host | Inline `#[cfg(test)]` modules, using `tempfile` for disk |
| `pnpm test` | All three | |
| `pnpm check` | Compile only: frontend, worker, `cargo check` | |

Run `pnpm test:worker` after any protocol change. Some tests are guards rather than behaviour checks; fix the code, not the guard:

- `theme.test.ts`: `:root` defaults match `TOKENS`, and no raw colour appears below `:root`.
- `builtin.test.ts`: the Plan contract's pinned hash. Changing it re-appends the contract to every saved Plan chat.
- `slash.test.ts`: command keys and name resolution shared with Settings.

Pure logic belongs in a colocated `*.test.ts` next to a small module (`chat-utils.ts`, `side-panel.ts`, `scroll-rail.ts`…) rather than inside a component. Rust keeps pure policy separate for the same reason (`computer_use/policy.rs`, `geometry.rs`, `keys.rs`, `outline.rs`).

## The running app

- **Start** with `pnpm dev:background`. It is idempotent, prints `Ready` once the app is up, and logs to `/tmp/wackcode-dev.log`. **Stop** with `pnpm dev:stop`. Don't launch the app any other way. `pnpm dev:reset` stops it and moves its data folder to the Trash.
- **Don't restart between tests.** Frontend edits hot-reload; Rust edits rebuild and relaunch the app automatically; worker edits need `pnpm build:worker` and then a new chat.
- **The dev app is its own app.** `tauri dev` merges `tauri.dev.conf.json`, so it runs as **WackCode Dev**, bundle id `com.wackcode.desktop.dev`, with its own data folder (`~/Library/Application Support/com.wackcode.desktop.dev/` — credentials and the Testing connection must be added there, never copied) and an amber Dock icon. The installed app keeps `com.wackcode.desktop`; the two run side by side and never share state.
- **Bundled build:** only when a test needs the bundled worker and resources, use `pnpm build:desktop:debug` and its `.app`, and never while the dev app is running.

## Driving the dev app

`scripts/wackdev/server.mjs` is a stdio MCP server agents use to operate the dev app — no coordinates, no real mouse, and never the installed app. Register it in the *everyday* WackCode under Settings › MCP servers as `node <repo>/scripts/wackdev/server.mjs` (the same command works in Claude Code). Its tools: `status`, `start`, `stop`, `tree`, `find`, `press`, `focus`, `type`, `key`, `scroll`, `wait_for`, `shot`.

- Call `tree` before acting; refs (`e3-12`) stay valid until the next `tree`/`find`. Acting on a `name` works too — an ambiguous name errors and lists the candidate refs.
- Use `wait_for` instead of sleeping while a run streams or a sheet animates.
- `shot` captures only the dev window, even while covered; `tree` is still the better way to read controls.
- Every call refuses unless a process with bundle id `com.wackcode.desktop.dev` is running from this checkout's `src-tauri/target/` with a "WackCode Dev" window (the rule is in `src-tauri/wackdev/src/driver/target.rs`).
- Accessibility and Screen Recording are charged to the app that launched the server — the everyday WackCode, or the terminal for Claude Code. `status` reports which is missing.
- If a control can't be named precisely, that's an accessibility bug: give it an `aria-label` in the app rather than reaching for coordinates.
- `pnpm build:worker`/`cargo` edits don't touch the driver; `src-tauri/wackdev/` is rebuilt lazily by the server (`cargo build -p wackdev --bin wackdev-helper`).
- Rebuilding the helper while `pnpm dev:background` is up makes `tauri dev` rebuild and relaunch the app (it watches `src-tauri/wackdev` too); occasionally that relaunch comes up without its webview — a plain `pnpm dev:stop` + `pnpm dev:background` fixes it. Never set `AXManualAccessibility` on this app: it removes the windows from `AXWindows` until relaunch.
- The minimal dev bundle is signed and verified after packaging. Its ad-hoc signature changes
  when Rust is rebuilt, so computer-use approvals may need renewing then. For permission and
  relaunch testing, use the bundled debug app: it has its own frontend and starts normally
  through Finder without the dev server.

## Models

- **Real LLMs:** use only the connection named **Testing (AI Agents may use this too)** (under Providers in the composer's model picker), through the app. Never read its key from `secrets.json`. Don't send prompts through any other connection or subscription sign-in: they may cost the owner money. For the dev app, the connection lives in *its* data folder — add it in the dev app's own Settings.
- **Deterministic output:** `pnpm mock:provider` serves a Chat Completions endpoint at `http://127.0.0.1:43127/v1`. A prompt containing "wait until stopped" hangs until cancelled, for testing cancellation.
- "escape fixture" aims the write one folder above the workspace (`../wackcode-escape.txt`). In a Chat mode chat the tool row shows the scratchpad refusal and no file appears.
- With sub-agents enabled, "background fixture" launches `worker` (or the first available role) for 25 seconds, answers immediately, then resumes with its result. "background fixture until stopped" keeps the child running for Stop tests. Give that role the mock model and disable automatic titles during mock testing. Use a disposable workspace; ordinary fixture prompts write `wackcode-live.txt`.

## MCP

- stdio: add `node <repo>/scripts/mock-mcp-server.mjs` as a server.
- HTTP/SSE: run `pnpm mock:mcp`, then add `http://127.0.0.1:43128/mcp` (HTTP) or `http://127.0.0.1:43128/sse` (SSE).
- The dev-app driver: `node <repo>/scripts/wackdev/server.mjs` — see *Driving the dev app* above.
