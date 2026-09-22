# WackCode

WackCode is a local macOS desktop interface for the Pi coding agent, built with Tauri 2, React, TypeScript, Rust, and `@earendil-works/pi-coding-agent` 0.86.1.

It supports named OpenAI-compatible connections and subscription sign-in through Pi for OpenAI Codex (ChatGPT Plus/Pro), GitHub Copilot, Anthropic, xAI, Meta, and Kimi For Coding. API key connections retain explicit model limits, reasoning mappings, and a per-model Vision switch. Settings can suggest those values from Pi's bundled model catalogue when you search by name or ID; you choose a suggestion, review it, and save. It also supports image attachments (paste, drop, or pick PNG, JPEG, GIF, or WebP files for models with Vision on), concurrent task workers, persistent Pi sessions, optional Git worktrees, and review of all staged, unstaged, and untracked changes. Chats open as drafts with a project selector and a Local/Worktree toggle, and can also run without a project in a per-chat scratch folder inside WackCode's application-data directory.

Conversations are Pi session trees, so nothing is lost when you change course: retry the last answer, edit an earlier message in place and send it again, rewind to before any message (its text returns to the composer), flip between the resulting versions with the ‹ n/m › switcher, or fork a chat from any turn into a new one. Before every prompt WackCode takes a checkpoint of the chat's files, so each of these can also put files back — it lists what changed and asks, and every restore can itself be undone.

## Run from source

The build currently targets macOS on Apple Silicon.

```sh
pnpm install
pnpm dev:desktop
```

Build the self-contained application with:

```sh
pnpm build:desktop
```

The result is written to `src-tauri/target/release/bundle/macos/WackCode.app`. The build downloads the official Node archive recorded in `runtime-lock.json`, checks its SHA-256 checksum, and bundles it with the production Pi worker. Running the app does not require a system Node or Pi installation.

## Test

```sh
pnpm test:web
pnpm test:worker
pnpm test:rust
```

The worker integration suite uses a private mock OpenAI-compatible endpoint. It covers concurrent sessions with overlapping model IDs, tool edits, request settings, cancellation, credential isolation, session restoration, and image attachments on vision and text-only models. It also covers retries and edits as session-tree versions, rewinds that survive a worker restart, and forks. Rust tests cover metadata recovery, Git changes, unusual filenames, binaries, repositories without commits, worktree isolation, and checkpoint snapshots and restores (byte-exact round trips, ignored and oversized files left alone, case-only renames, nested repositories).

For a manual UI fixture, run `pnpm mock:provider` and configure `http://127.0.0.1:43127/v1` as a Chat Completions connection.

## Local data and network boundaries

WackCode has no WackCode account, backend, analytics, updater, or automatic model discovery. Browsing or installing a package contacts the public npm registry and the package's own npm or git source, and only when you ask it to. Subscription sign-in contacts the provider’s authorization service after you choose **Sign in**; Pi refreshes an expired token as part of a model request, never on launch or on a timer. Provider requests contain the conversation, any images you attach, and any project context Pi reads or creates through its tools. Manually entered API keys are stored in `secrets.json` and delivered to a task worker only through its private stdin pipe. Subscription credentials (which may include provider-minted keys) live in separate mode-0600 Pi auth files under WackCode’s application-data directory, never in the Pi CLI’s auth file. Settings, task metadata, Pi sessions, and file checkpoints live in WackCode’s macOS application-data directory. Checkpoints are a private Git object store per chat that borrows unchanged files from the project's own repository without writing to it; they are deleted with the chat. Ignored files and files over 25 MB are never captured or restored, checkpoints are off for a home folder, and a restore in a folder other chats also work in changes their files too.

Workers disable Pi telemetry, update checks, remote model-catalog refresh, cache warming, and project `.pi` configuration. Model suggestions read only the catalogue shipped with the app and make no network request. Packages you install through Settings are loaded by explicit path, so a project's own `.pi/extensions` is still never run.

Project instruction files such as `AGENTS.md` still load. The `read`, `bash`, `edit`, `write`, `grep`, `find`, and `ls` tools execute with the current macOS account’s permissions; a selected folder or worktree is a working directory, not a sandbox. Each tool can be switched off in Settings › Tools, and a tool whose helper binary is missing (`grep` needs ripgrep, `find` needs fd) is listed as unavailable rather than offered to the model.

**Installed packages run as ordinary local code with the same permissions.** An extension can read and write any file you can, run any command, make network requests, and read WackCode's API keys and subscription credentials. WackCode names the source and states this before the first install of each package, but that is informed consent, not a sandbox — install only what you trust.

Non-image attachments, embedded editors and terminals, permission prompts, automatic merging, notarization, updates, and other platforms remain outside this milestone. MCP is not a built-in feature but is available by installing a package such as `pi-mcp-adapter`.
