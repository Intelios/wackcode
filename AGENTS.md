# AGENTS.md

WackCode is a native macOS coding agent: a Rust app drawn with GPUI, with its own agent core. This is **version 3**, a ground-up remake on the `3.0-Remake` branch. Version 2 (Tauri, React and the Pi coding agent) lives on `2.5-Legacy` and is reference material only. `README.md` still describes version 2 and is rewritten before release; until then, leave it alone.

**Keep this file short:** it is sent with every agent request. It holds only rules that any change could break, plus the design direction. Area detail goes in `docs/`, and a feature's invariants go in comments next to its code, so read a file's header comment before changing it.

## Roles
- The Agent working in this repo is allowed to commit freely, if you finish your work and it indeed works correctly then you may commit without asking the user for permission. A title and description of your changes inside the commit is encouraged.

## Ground rules

- macOS 26+ on Apple Silicon only. Unix-only APIs are deliberate; don't "fix" them for other platforms.
- Cargo only. No Node, no pnpm, no web UI.
- No data compatibility with version 2: never read or write its files. The one shared contract is the usage ledger ([docs/wackcode-usage-v1.md](docs/wackcode-usage-v1.md)).
- Out of scope; don't propose these as fixes: embedded editors, permission prompts (tool-call approval gates), automatic merging, notarization, auto-updates. Computer use's per-app access card is not one: it grants an app once per chat, never per action (the owner's call, 2026-11-24).
- Cut from version 3 on purpose; don't bring these back: arcade games, checkpoints / rewind / fork, per-chat worktrees, the menu bar duck, the goal loop, Pi packages, subscription sign-ins, composer styles, dynamic backdrops, renaming the agent.
- To port something from version 2, read it from the legacy checkout (`../wackcode-legacy`, or `git show 2.5-Legacy:<path>`). Port the idea and the tested logic; don't port Tauri, Pi or web-view workarounds.
- `todo.md` (gitignored) is the owner's idea list: ask before starting anything from it.

## Map

- **`apps/wackcode/`**: the app binary. Opens the window and hands it to `wack-ui`.
- **`apps/gallery/`**: renders UI scenes to PNG files (`cargo snap`).
- **`apps/xtask/`**, **`apps/wackcode-helper/`**: the `.app` bundle the browser engine (CEF) needs, and its helper processes.
- **`crates/wack-ui/`**: every view, the pane tree and layout solver, the theme. The only crate that may use GPUI.
- **`crates/wack-core/`**: the engine: projects, chats, settings, secrets, Git, PTYs, events.
- **`crates/wack-agent/`**, **`wack-llm/`**, **`wack-tools/`**: the agent loop, the provider wire formats, and the core tools.
- **`crates/wack-computer/`**: computer use: the agent in the user's other Mac apps, and its access rules.
- **`crates/wack-cli/`**: `wack`, the agent without the app.

## Read before changing

| If you touch… | Read |
|---|---|
| Crate boundaries, the engine/UI split, where data lives, what is built so far and what comes next | [docs/architecture.md](docs/architecture.md) |
| The GPUI version, its patches, bumping it | [docs/gpui.md](docs/gpui.md) |
| The CEF (Chromium) version, the bundle, bumping it | [docs/cef.md](docs/cef.md) |
| Network, credentials, the automation socket | [docs/security.md](docs/security.md) |
| Checking your work: tests, the gallery, the dev app, real LLMs | [docs/testing.md](docs/testing.md) |
| The usage ledger TokenTrail reads (a published contract) | [docs/wackcode-usage-v1.md](docs/wackcode-usage-v1.md) |

## Rules that break things silently

- **Security is not a trade-off.** Never add a network origin, let a credential travel anywhere but `secrets.json` and the request that needs it, or let the automation socket into a release build. If a change seems to need one, stop and say so. Full list: [docs/security.md](docs/security.md).
- **GPUI stays in `wack-ui` and `apps/`.** Nothing in `wack-core` or below may depend on it: the UI is a viewport onto the engine.
- **One GPUI revision.** Every GPUI crate shares the rev pinned in the root `Cargo.toml`. Bumping it is its own task, never part of feature work ([docs/gpui.md](docs/gpui.md)).
- **Theme tokens only.** Views read colours from `Theme` (`crates/wack-ui/src/theme.rs`) and never write a raw colour; a test there guards it. Text styles likewise come from `Type` (`typography.rs`), never a raw size.

## Design

WackCode should feel like a distinctive macOS app with its own character, never a generic web app or generic AI-generated design. Make deliberate choices in layout, typography, detail and interaction that reinforce that character.

- **Focused, not configurable.** One layout and one look, chosen on purpose. The user personalises by choosing, never by making: a theme from a fixed set (WackCode 2's ten; never hand-picked colours), light or dark, an image backdrop. Never structure.
- **A frame and a stage.** The window is a frame around glass panes (the sidebar, and the stage holding the screen); over an image backdrop the picture shows in the frame and through the glass.
- **Typographic, with round controls.** Strong type hierarchy; controls are pills and circles. The serif is for the rare place it lifts the page (a screen's one display line), never every label. The chat column and the new-chat and welcome screens are centred, on the owner's call.
- **Motion is identity.** Expressive, considered transitions make the app feel alive. Every animation respects the system's reduced-motion setting, with a still end state for anything that loops.
- **Design for every theme.** Each view must look right in light and dark, with every theme, and over an image backdrop. The accent recolours interaction, never meaning: danger, warning, diff and success stay fixed.
- **Native, not web.** Own title strip that drags the window, macOS key glyphs (⌘⇧⌥⌃), and settings named as `Settings › Section`.
- **Panes are planned, not bolted on.** Each chat owns a pane tree beside its transcript; the transcript never drops below its minimum width. Anything new that needs screen space is a pane.
- **The duck stays.**

## Conventions

- **Rust:** fallible functions return `Result<T, String>` whose error is a user-facing sentence; no `anyhow`/`thiserror` in our crates (convert GPUI's own results at the boundary). Git goes through the system `git` CLI (no `git2`).
- **State:** lives in models, not views, so closing or switching a view never loses it.
- **Tests:** pure logic in inline `#[cfg(test)]` modules, with `tempfile` for anything on disk. UI is checked with gallery scenes: add a scene for each component state worth looking at.

## Commands

| Command | When |
|---|---|
| `cargo check --workspace` | Fast compile check. |
| `cargo test --workspace` | All tests. |
| `cargo clippy --workspace` | Lints. |
| `cargo snap --list`, `cargo snap <scene> --theme dark` | Render a UI scene to a PNG and look at it. |
| `cargo dev` | Run the dev app (from `target/debug/WackCode Dev.app`), with the automation socket compiled in. |
| `cargo wack run "…"` | Run the agent without the app ([docs/testing.md](docs/testing.md)). |
| `cargo deny check licenses` | After adding or bumping a dependency. |

**Real LLMs:** use only the connection named **Testing (AI Agents may use this too)**, through the app or `cargo wack`, and never read its key from `secrets.json`. Any other connection may cost the owner money. The dev app has its own data folder: the Testing connection must be added *inside the dev app*, never copied from another folder.
