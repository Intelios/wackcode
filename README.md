# WackCode

WackCode is a local macOS desktop interface for the Pi coding agent, built with Tauri 2, React, TypeScript, Rust, and `@earendil-works/pi-coding-agent` 0.86.1.

It supports named OpenAI-compatible connections, explicit model limits, reasoning mappings, and a per-model Vision switch. Settings can suggest those values from Pi's bundled model catalogue when you search by name or ID; you choose a suggestion, review it, and save. It also supports image attachments (paste, drop, or pick PNG, JPEG, GIF, or WebP files for models with Vision on), concurrent task workers, persistent Pi sessions, optional Git worktrees, and review of all staged, unstaged, and untracked changes. Chats open as drafts with a project selector and a Local/Worktree toggle, and can also run without a project in a per-chat scratch folder inside WackCode's application-data directory.

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

The worker integration suite uses a private mock OpenAI-compatible endpoint. It covers concurrent sessions with overlapping model IDs, tool edits, request settings, cancellation, credential isolation, session restoration, and image attachments on vision and text-only models. Rust tests cover metadata recovery, Git changes, unusual filenames, binaries, repositories without commits, and worktree isolation.

For a manual UI fixture, run `pnpm mock:provider` and configure `http://127.0.0.1:43127/v1` as a Chat Completions connection.

## Local data and network boundaries

WackCode has no account, backend, analytics, updater, or automatic model discovery. Browsing or installing a package contacts the public npm registry and the package's own npm or git source, and only when you ask it to. Provider requests contain the conversation, any images you attach, and any project context Pi reads or creates through its tools. API keys are stored in a `secrets.json` file restricted to the current user inside WackCode’s macOS application-data directory, and delivered to a task worker only through its private stdin pipe. Settings, task metadata, and Pi sessions live in WackCode’s macOS application-data directory.

Workers disable Pi telemetry, update checks, remote model-catalog refresh, cache warming, and project `.pi` configuration. Model suggestions read only the catalogue shipped with the app and make no network request. Packages you install through Settings are loaded by explicit path, so a project's own `.pi/extensions` is still never run.

Project instruction files such as `AGENTS.md` still load. The `read`, `bash`, `edit`, `write`, `grep`, `find`, and `ls` tools execute with the current macOS account’s permissions; a selected folder or worktree is a working directory, not a sandbox. Each tool can be switched off in Settings › Tools, and a tool whose helper binary is missing (`grep` needs ripgrep, `find` needs fd) is listed as unavailable rather than offered to the model.

**Installed packages run as ordinary local code with the same permissions.** An extension can read and write any file you can, run any command, make network requests, and read the API keys WackCode has stored. WackCode names the source and states this before the first install of each package, but that is informed consent, not a sandbox — install only what you trust.

Subscription login, non-image attachments, embedded editors and terminals, permission prompts, automatic merging, notarization, updates, and other platforms remain outside this milestone. MCP is not a built-in feature but is available by installing a package such as `pi-mcp-adapter`.
