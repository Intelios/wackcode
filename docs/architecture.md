# Architecture

How WackCode's processes fit together, where state lives, and the rules that keep them consistent. For the worker's internals see [worker.md](worker.md); for the renderer see [frontend.md](frontend.md).

## Processes

| Process | Code | Holds credentials? | Network? |
|---|---|---|---|
| Renderer (WKWebView) | `src/` | Never | Never; CSP allows IPC only |
| Rust host | `src-tauri/src/` | Reads `secrets.json` / `auth.json` | Only for user-triggered actions (see [security.md](security.md)) |
| Chat worker, one per chat | `worker/src/index.ts` | Only what arrives over its stdin | Provider, MCP, `web_fetch`; Pi runs with `PI_OFFLINE` |
| Package manager | `worker/src/manager.ts` | Never | npm registry and package sources |
| Skills scan | `worker/src/skills-scan.ts` | Never | Never; reads files only |
| Commands scan | `worker/src/commands-scan.ts` | Never | Never; runs trusted extension code offline |
| MCP probe (Test connection) | `worker/src/mcp-probe.ts` | That one server's values | That one server |
| Model catalogue | `worker/src/catalog.ts` | Never | Never; static catalogue |

Each chat worker runs one Pi session in-process. Sub-agents are in-memory child sessions inside the same worker (`subagent-runner.ts`), not separate processes. Helpers are short-lived and do one job each. The dev app runs them from `worker/dist/`; the bundle runs `resources/worker/dist/` on a pinned Node runtime (`runtime-lock.json`).

Native subsystems live in Rust and never in a worker: the per-chat browser (`browser.rs`, a child `WKWebView`), the terminal (`terminal.rs`), computer use (`computer_use/`), the window backdrop (`glass.rs`), and the menu bar duck (`menu_bar.rs`). A worker asks for browser and computer work over the protocol (`browser_request`, `computer_request`); the host does it and enforces the rules.

## Where state lives

Everything is under the app data directory (`~/Library/Application Support/com.wackcode.desktop/`), except the user's skills in `~/.agents/skills`.

| Path | What |
|---|---|
| `wackcode.json` | Settings, projects, chats. Written only through `MetadataState::mutate` (`storage.rs`). |
| `secrets.json` (0600) | API keys, and MCP header/env values keyed `mcp:<id>`. |
| `subscriptions/<id>/auth.json` (0600) | OAuth credentials, one file per subscription provider. |
| `sessions/<task>/`, `agent/<task>/` | A chat's Pi session files and Pi agent directory. |
| `scratch/<task>/` | Workspace for chats without a project. |
| `checkpoints/<task>/` | The chat's shadow Git repository (see below). |
| `memory/<name>-<key>/` | Project memory notes, one directory per repository shared by its worktrees. |
| `commands/` | The user's slash commands. |
| `pi/` | Installed packages: Pi's `settings.json` plus `npm/` and `git/`. |
| `backgrounds/` | Validated copies of chosen background images; the only asset-protocol scope. |
| `usage/v1/` | The usage ledger ([wackcode-usage-v1.md](wackcode-usage-v1.md)); never deleted with a chat. |

The renderer keeps only UI conveniences in `localStorage` (`wackcode:*` keys such as the last model, the side panel's view and width, collapsed and pinned projects, and Git mode's last repository and diff layout).

## Shared types

- A type that crosses layers changes in `src-tauri/src/models.rs`, `src/types.ts` and, if the worker sees it, `worker/src/protocol.ts` together.
- New optional Rust fields need `#[serde(default)]` so existing `wackcode.json` files load. Runtime-only fields (like `ProjectRecord.branch`) also need `skip_deserializing`.
- Some defaults are mirrored by hand: `DEFAULT_APPEARANCE` in `theme.ts` mirrors `AppearanceConfig::default()`, and `DEFAULT_BACKGROUND` also lives in `glass.rs`.

## New Tauri command

1. Write it in the matching Rust module, returning `Result<T, String>` with a user-facing error sentence.
2. Register it in `generate_handler!` in `lib.rs`.
3. Add one typed wrapper in `src/api.ts`.
4. Add any new plugin permission in `src-tauri/capabilities/default.json`.

## Worker lifecycle (`worker.rs`)

- **Spawn:** any command that talks to a chat's worker goes through `ensure_worker`, which starts one if needed. Never assume a worker is running.
- **Fingerprint:** provider, model and the trusted package resources resolved at spawn. When it changes, the worker is killed and respawned, even mid-run. Settings that apply live (tools, prompts, sub-agents, MCP, skills, commands, memory, computer use, a provider's on/off switch) travel in `init` and in `set_*` commands and must stay out of the fingerprint.
- **Idle reaper:** stops a worker 15 minutes after its last output, never the open chat (`SelectedTask`) and never the newest four. It takes the chat's task lock and re-checks status, so it can't race a run.
- **Stopping vs crashing:** intentional stops remove the worker from the registry *before* signalling, so any exit while still registered is reported as a crash. `killpg` takes down the worker's whole process group, including stdio MCP servers.

## Locks

- **`TaskLocks`** (`commands.rs`): any command that sends a chat work, moves its conversation, or touches its checkpoints holds that chat's lock.
- **`GitLocks`**: one per checkout root. Git writes from the Changes panel and Git mode (commit, discard, push, fast-forward, undo, revert), branch switches and prompt dispatch (which snapshots a checkpoint) take it, so chats sharing a folder never interleave those operations.
- **`GitNetworkLocks`**: one per checkout root, held only while a fetch talks to the remote. Keep network I/O off `GitLocks`: prompt dispatch waits on that lock, and a fetch can take 90 seconds. A pull therefore fetches first, then takes `GitLocks` for the local fast-forward.

## Git targets

Every Git command takes `task_id` or `project_id` and resolves it with `git_target` (`commands.rs`): a chat's workspace (the Changes panel) or a project's folder (Git mode, and the draft's branch picker). `TaskLocks` is held only for a chat target.

- **Writes** go through `locked_checkout`: `idle_checkout` (no chat may be running in that checkout), then `GitLocks`, then `idle_checkout` again, because a chat can start while the command waits for the lock.
- **Reads** (`git_changes`, `git_sync_status`, `git_log`, `git_commit_files`, `git_commit_diff`) and `git_fetch` take neither: a fetch moves no branch, index or file.
- **Network** (`git_fetch`, `git_pull`'s fetch, `git_push`, every `gh` call) runs through `git_cli`, which applies the login-shell environment so the user's credential helper and ssh-agent work. `git.rs` stays pure and synchronous.
- **Pull never merges.** `git::fast_forward` reports a diverged branch and leaves HEAD alone; a conflicting `git::revert_commit` aborts itself. Both hand the rest to the agent.
- `git_generate_message` keeps `task_id`: the chat's own worker and model write the message.

## Session tree and checkpoints

Read the invariants at the top of `worker/src/tree.ts` and `src-tauri/src/checkpoints.rs` before changing either. In short:

- A conversation is a Pi session tree. Retry, edit, rewind and fork create or move between branches; nothing is deleted. Versions of a message are grouped by their *logical parent* (the nearest user, assistant or tool-result ancestor), because Pi inserts other entries between turns.
- WackCode writes its own custom entries: `wackcode-checkpoint` above every user message, `wackcode-leave` / `wackcode-nav` around every navigation (so a rewind survives a restart), and `wackcode-command-presentation` above command-generated prompts.
- Checkpoints are a private shadow Git repository per chat under app data. It borrows the project's objects via `alternates`, stores snapshots as bare tree objects (no commits, hooks or signing), and ignores the system and global Git config (no hooks, filters or fsmonitor). A restore only touches paths that differ, never files the snapshot doesn't hold (ignored, over 25 MB, or untracked). It must never write objects or refs into the project's own repository.

## Usage ledger

`worker/src/usage.ts` instruments every model runtime and emits `usage_record`; `src-tauri/src/usage.rs` validates and appends it to `usage/v1/`. The Rust `UsageRecord` uses `deny_unknown_fields`, so a new field lands in both at once. The format is a published contract that TokenTrail reads ([wackcode-usage-v1.md](wackcode-usage-v1.md)): changes are additive, and records never carry prompts, responses, titles, URLs or credentials.
