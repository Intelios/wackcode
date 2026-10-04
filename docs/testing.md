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

- **Start** with `pnpm dev:background`. It is idempotent, prints `Ready` once the app is up, and logs to `/tmp/wackcode-dev.log`. **Stop** with `pnpm dev:stop`. Don't launch the app any other way.
- **Don't restart between tests.** Frontend edits hot-reload; Rust edits rebuild and relaunch the app automatically; worker edits need `pnpm build:worker` and then a new chat.
- **Drive** the window by its bundle id, `com.wackcode.desktop`.
- **Bundled build:** only when a test needs the bundled worker and resources, use `pnpm build:desktop:debug` and its `.app`, and never while the dev app is running.
- The minimal dev bundle is signed and verified after packaging. Its ad-hoc signature changes
  when Rust is rebuilt, so computer-use approvals may need renewing then. For permission and
  relaunch testing, use the bundled debug app: it has its own frontend and starts normally
  through Finder without the dev server.

## Models

- **Real LLMs:** use only the connection named **Testing (AI Agents may use this too)** (under Providers in the composer's model picker), through the app. Never read its key from `secrets.json`. Don't send prompts through any other connection or subscription sign-in: they may cost the owner money.
- **Deterministic output:** `pnpm mock:provider` serves a Chat Completions endpoint at `http://127.0.0.1:43127/v1`. A prompt containing "wait until stopped" hangs until cancelled, for testing cancellation.

## MCP

- stdio: add `node <repo>/scripts/mock-mcp-server.mjs` as a server.
- HTTP/SSE: run `pnpm mock:mcp`, then add `http://127.0.0.1:43128/mcp` (HTTP) or `http://127.0.0.1:43128/sse` (SSE).
