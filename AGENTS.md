# AGENTS.md

WackCode is a local macOS desktop app for the [Pi coding agent](https://www.npmjs.com/package/@earendil-works/pi-coding-agent), built with Tauri 2, React 19 + TypeScript, and Rust. Pi is pinned in `worker/package.json`. `README.md` describes user-facing behavior; update it when that behavior changes.

**Keep this file short:** it is sent with every agent request. It holds only rules that any change could break, plus the design direction. Area detail goes in `docs/`, and a feature's invariants go in comments next to its code, so read a file's header comment before changing it.

## Ground rules

- macOS on Apple Silicon only. Unix-only APIs (`killpg`, `open(1)`) are deliberate; don't "fix" them for other platforms.
- pnpm only (version pinned in `package.json`). No npm/yarn, no `package-lock.json`.
- Out of scope; don't propose these as fixes: embedded editors, permission prompts (tool-call approval gates; Computer use's per-app access card is the one deliberate exception), automatic merging, notarization, auto-updates, other platforms.
- `todo.md` (gitignored) is the owner's idea list: ask before starting anything from it.

## Map

- **`src/`**: React renderer. `App.tsx` owns cross-cutting state and every Tauri event listener; `api.ts` is the only bridge to Rust.
- **`src-tauri/src/`**: Rust host: commands, worker lifecycle, persistence, credentials, and the native subsystems (browser, terminal, computer use, glass).
- **`worker/src/`**: one Node process per chat wrapping Pi. `protocol.ts` is its NDJSON contract with Rust; `builtin/` holds WackCode's own extensions.

## Read before changing

| If you touch… | Read |
|---|---|
| Processes, worker lifecycle, locks, persistence, types crossing layers, session tree, checkpoints | [docs/architecture.md](docs/architecture.md) |
| Worker protocol, command queue, tools, built-ins, planning modes, memory, slash commands | [docs/worker.md](docs/worker.md) |
| Renderer state, snapshots, side panel, terminal, scroll rail, attachments, window | [docs/frontend.md](docs/frontend.md) |
| Network, credentials, packages, skill/command loading, asset protocol, computer use | [docs/security.md](docs/security.md) |
| Styles, theme, motion, components, icons, copy | [docs/design.md](docs/design.md) |
| Tests, the dev app, mock servers, real LLMs | [docs/testing.md](docs/testing.md) |
| The usage ledger TokenTrail reads (a published contract) | [docs/wackcode-usage-v1.md](docs/wackcode-usage-v1.md) |

## Rules that break things silently

- **Security is not a trade-off.** Never add a network origin, let a credential travel anywhere but `secrets.json`/`auth.json` and a worker's stdin, flip one of Pi's `no*` lockdown flags, widen the asset protocol, or trust a package without the user's confirmation. If a change seems to need one, stop and say so. Full list: [docs/security.md](docs/security.md).
- **Shared types** change in `models.rs`, `types.ts` and (if the worker sees them) `protocol.ts` together. New optional Rust fields need `#[serde(default)]` so existing `wackcode.json` files load. Every `wackcode.json` write goes through `MetadataState::mutate`.
- **Worker respawns:** anything in the `worker.rs` fingerprint respawns the worker, even mid-run, so live settings stay out of it. Idle workers are reaped, so reach one only through `ensure_worker`.
- **Worker queue:** commands that must reach an active run bypass the serial prompt queue or they deadlock (`worker/src/index.ts`).
- **Tools:** never pass `tools:` to the chat's `createAgentSession`; Pi treats it as a registry filter and erases extension tools. Apply the denylist with `setActiveToolsByName`.
- **Plan contract** stays byte-identical: saved chats match it by exact text. Don't just update the test's pinned hash.
- **Checkpoints** never write to the project's own Git repository.

## Design

WackCode should feel like a distinctive macOS app with its own character, never a generic web app or generic AI-generated design. Make deliberate choices in layout, typography, detail and interaction that reinforce that character. The toolkit is in [docs/design.md](docs/design.md).

- **Motion is identity.** Expressive, playful, considered transitions and feedback make interactions feel responsive and full of personality: the composer's comet, the hero→dock glide, the Ultra Plan flame, direction-aware panel slides. Every animation respects reduced motion: `useReducedMotion` in motion/react, and a still end state under `prefers-reduced-motion` for looping CSS animations (a global rule shortens the rest).
- **Personal and adaptable.** Users pick the accent, background, image backdrop, Liquid Glass and the agent's name. Design every feature to look right across all of them rather than for the default lime-on-near-black.
- **Theme tokens only.** Themeable colours are tokens computed in `src/theme.ts` (`TOKENS`) with matching `:root` defaults in `styles.css`; never write a raw colour below `:root` (`theme.test.ts` guards it). The accent recolours interaction, never meaning: danger, warning, diff, success and Ultra Plan's warm palette stay fixed.
- **Shell vs content.** Over an image or Liquid Glass only the shell (window, sidebar, header, side panels) turns see-through; content surfaces (bubbles, composer, cards) stay solid so text stays readable.
- **Native, not web.** Overlay title bar with working `data-tauri-drag-region` strips, macOS key glyphs (⌘⇧⌥⌃), and settings named as `Settings › Section`.
- **Agent persona.** The app's own copy never calls the agent "Pi": the name comes from `appearance.agentName`, resolved in `src/agentName.ts` (default "WackCode"). Genuine Pi product references (catalogue, sign-in, the worker process) keep the real name.
- **Reuse before inventing.** Use the primitives in `src/components/ui/`, the icons in `Icons.tsx`, and the button classes in `styles.css`. Destructive actions go through `ConfirmDialog` with `danger`. Icon-only buttons get an `aria-label`, and anything revealed on hover is also revealed on keyboard focus.

## Conventions

- **Rust:** fallible functions return `Result<T, String>` whose error is a user-facing sentence; no `anyhow`/`thiserror`. Git goes through the system `git` CLI (no `git2`).
- **React:** update state immutably via `patchTask` / `patchRuntime` / `setData`. Components are presentational; the few that call `api` directly are listed in [docs/frontend.md](docs/frontend.md).
- **CSS:** one stylesheet, `src/styles.css`. Add rules to the matching section and use `--wc-*` variables. No Tailwind or other CSS system.
- **Tests:** pure logic in colocated `*.test.ts`, components with Testing Library role-based queries, worker behavior in `worker/src/*.test.ts`, Rust in inline `#[cfg(test)]` modules with `tempfile`.

## Commands

| Command | When |
|---|---|
| `pnpm check` | Fast compile check: frontend, worker, `cargo check`. |
| `pnpm test` | All suites; or `test:web`, `test:worker`, `test:rust` individually. |
| `pnpm build:worker` | After editing `worker/src/` (the dev app runs `worker/dist/`). |
| `pnpm dev:background` / `pnpm dev:stop` | Start / stop the live dev app. The only way to launch it; see [docs/testing.md](docs/testing.md). |
| `pnpm mock:provider` | Deterministic mock provider at `http://127.0.0.1:43127/v1` (Chat Completions). |

**Real LLMs:** use only the connection named **Testing (AI Agents may use this too)**, through the app, and never read its key from `secrets.json`. Any other connection or subscription sign-in may cost the owner money.
