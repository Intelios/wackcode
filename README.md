# WackCode

WackCode is a local macOS desktop interface for the Pi coding agent. This proof of concept uses Tauri 2, React, TypeScript, Rust, and `@earendil-works/pi-coding-agent` 0.86.1.

It supports named OpenAI-compatible connections, explicit model limits and reasoning mappings, concurrent task workers, persistent Pi sessions, optional Git worktrees, and review of all staged, unstaged, and untracked changes. Chats open as drafts with a project selector and a Local/Worktree toggle, and can also run without a project in a per-chat scratch folder inside WackCode's application-data directory.

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

The worker integration suite uses a private mock OpenAI-compatible endpoint. It covers concurrent sessions with overlapping model IDs, tool edits, request settings, cancellation, credential isolation, and session restoration. Rust tests cover metadata recovery, Git changes, unusual filenames, binaries, repositories without commits, and worktree isolation.

For a manual UI fixture, run `pnpm mock:provider` and configure `http://127.0.0.1:43127/v1` as a Chat Completions connection.

## Local data and network boundaries

WackCode has no account, backend, analytics, updater, or automatic model discovery. Provider requests contain the conversation and any project context Pi reads or creates through its tools. API keys are stored in a `secrets.json` file restricted to the current user inside WackCode’s macOS application-data directory, and delivered to a task worker only through its private stdin pipe. Settings, task metadata, and Pi sessions live in WackCode’s macOS application-data directory.

Workers disable Pi telemetry, update checks, remote model-catalog refresh, cache warming, extensions, skills, prompt packages, themes, and project `.pi` configuration. Project instruction files such as `AGENTS.md` still load. The default `read`, `bash`, `edit`, and `write` tools execute with the current macOS account’s permissions; a selected folder or worktree is a working directory, not a sandbox.

MCP, extension and skill management, subscription login, attachments, embedded editors and terminals, permission prompts, automatic merging, notarization, updates, and other platforms remain outside this milestone.
