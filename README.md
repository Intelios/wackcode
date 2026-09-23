# WackCode

WackCode is a local macOS desktop interface for the Pi coding agent, built with Tauri 2, React, TypeScript, Rust, and `@earendil-works/pi-coding-agent` 0.86.1.

It supports named OpenAI-compatible connections and subscription sign-in through Pi for OpenAI Codex (ChatGPT Plus/Pro), GitHub Copilot, Anthropic, xAI, Meta, and Kimi For Coding. API key connections retain explicit model limits, reasoning mappings, and a per-model Vision switch. Settings can suggest those values from Pi's bundled model catalogue when you search by name or ID; you choose a suggestion, review it, and save. It also supports image attachments (paste, drop, or pick PNG, JPEG, GIF, or WebP files for models with Vision on), concurrent task workers, persistent Pi sessions, optional Git worktrees, and review of all staged, unstaged, and untracked changes. Chats open as drafts with a project selector and a Local/Worktree toggle, and can also run without a project in a per-chat scratch folder inside WackCode's application-data directory.

Conversations are Pi session trees, so nothing is lost when you change course: retry the last answer, edit an earlier message in place and send it again, rewind to before any message (its text returns to the composer), flip between the resulting versions with the ‹ n/m › switcher, or fork a chat from any turn into a new one. Before every prompt WackCode takes a checkpoint of the chat's files, so each of these can also put files back — it lists what changed and asks, and every restore can itself be undone.

Plan mode (the Build/Plan toggle in the composer, or ⇧Tab) keeps the agent read-only: it inspects the workspace, asks a few questions, and proposes a plan you can approve, revise, save as `PLAN.md`, or discard. Click Plan again for Ultra Plan, a "grill me" interview: the agent asks as many questions as it needs, one at a time with its recommended answer first, before it writes the plan. Expect more usage; "Write the plan now" on any question ends the interview.

Settings › Prompts shows the built-in prompts — the default system prompt that opens every chat, and the Plan mode and Ultra Plan contracts — and lets you customize them. It's optional: the built-ins are designed to work best with Pi and WackCode, but you can change them to your own liking, and restore the built-in text at any time. Changes apply from the next message in your chats, including chats that are already planning; nothing restarts.

Sub-agents are an optional built-in, off until you switch them on in Settings › Packages. When they're on, the agent can hand a self-contained task to a sub-agent that has its own context window, or run several at once. WackCode ships three roles: Scout (read-only reconnaissance), Reviewer (read-only code review) and Worker (edits files). You can add your own. Each one uses the chat's model unless you give it its own on the Sub-agents settings page. By default the agent only delegates when you ask, because every sub-agent is extra model usage. Read-only sub-agents run in parallel; ones that edit files take turns. In Plan and Ultra Plan, only read-only sub-agents run. Each call appears in the chat as a card showing each sub-agent's progress, answer and token use.

Auto chat titles are another optional built-in, off by default. Open Settings › Auto titles from the Built-in extensions list, choose a connected provider and a small, inexpensive model, then enable it on the same page. On a new chat's first prompt, WackCode sends only that prompt's text to the chosen connection in one extra request. The title arrives in the background while the chat responds. Each chat gets one attempt, even if the request fails or is interrupted; later messages, retries, forks, and chats created before the feature was enabled never trigger it. Until a title succeeds, the usual free title based on the opening prompt remains. A manual rename always wins.

Type `/` in the composer to browse commands from enabled packages, prompt templates, and skills. WackCode also provides `/init`, `/compact [instructions]`, `/new`, `/name <name>`, and `/copy`. `/init` takes no arguments and creates or carefully improves a concise `AGENTS.md` in a Build-mode project chat's current workspace; its change appears in the normal file diff. Selecting a suggestion inserts it so you can add arguments before sending. A new draft initializes its chat when you type `/`, without sending a model prompt.

Type `@` to mention a file or folder in the chat's workspace (in a Git repository, every file that isn't ignored). Pick one from the list and the message keeps its path, such as `@src/App.tsx`, the way Pi's own terminal does; the model reads the file itself when it needs it. Paths with spaces are quoted, and choosing a folder lists what's inside it.

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

WackCode has no WackCode account, backend, analytics, updater, or automatic model discovery. Browsing or installing a package contacts the public npm registry and the package's own npm or git source, and only when you ask it to. Subscription sign-in contacts the provider’s authorization service after you choose **Sign in**; Pi refreshes an expired token as part of a model request, never on launch or on a timer. Provider requests contain the conversation, any images you attach, and any project context Pi reads or creates through its tools. Manually entered API keys are stored in `secrets.json` and delivered to a task worker only through its private stdin pipe. With sub-agents on, that includes the key or sign-in of any connection a sub-agent's own model uses. Subscription credentials (which may include provider-minted keys) live in separate mode-0600 Pi auth files under WackCode’s application-data directory, never in the Pi CLI’s auth file. Settings, task metadata, Pi sessions, and file checkpoints live in WackCode’s macOS application-data directory. Checkpoints are a private Git object store per chat that borrows unchanged files from the project's own repository without writing to it; they are deleted with the chat. Ignored files and files over 25 MB are never captured or restored, checkpoints are off for a home folder, and a restore in a folder other chats also work in changes their files too.

Workers disable Pi telemetry, update checks, remote model-catalog refresh, cache warming, and project `.pi` configuration. Sub-agents run as separate Pi sessions inside the chat's worker. They load no packages and can use only Pi's own tools. Model suggestions read only the catalogue shipped with the app and make no network request. Packages you install through Settings are loaded by explicit path, so a project's own `.pi/extensions` is still never run.

Project instruction files such as `AGENTS.md` still load. The `read`, `bash`, `edit`, `write`, `grep`, `find`, and `ls` tools execute with the current macOS account’s permissions; a selected folder or worktree is a working directory, not a sandbox. Each tool can be switched off in Settings › Tools, and a tool whose helper binary is missing (`grep` needs ripgrep, `find` needs fd) is listed as unavailable rather than offered to the model.

**Installed packages run as ordinary local code with the same permissions.** An extension can read and write any file you can, run any command, make network requests, and read WackCode's API keys and subscription credentials, including those of connections your sub-agents or auto titles use. WackCode names the source and states this before the first install of each package, but that is informed consent, not a sandbox — install only what you trust.

Non-image attachments, embedded editors and terminals, permission prompts, automatic merging, notarization, updates, and other platforms remain outside this milestone. MCP is not a built-in feature but is available by installing a package such as `pi-mcp-adapter`.
