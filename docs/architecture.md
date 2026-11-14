# Architecture

WackCode 3 is one Rust process. There is no Node worker, no IPC protocol and no web view.

## Crates

```
apps/wackcode      the app binary: opens the window, hands it to wack-ui
apps/gallery       renders UI scenes to PNG files
crates/wack-ui     views, pane tree, layout solver, theme        (GPUI)
crates/wack-core   engine: projects, chats, settings, secrets, Git, PTYs, events
crates/wack-agent  turn loop, tool registry, prompt assembly, sessions, compaction
crates/wack-tools  read, write, edit, bash, grep, find, ls
crates/wack-llm    provider wire formats, streaming events, usage, retries
crates/wack-cli    `wack`: the agent without the app
```

Dependencies point downwards only: `wack-ui` → `wack-core` → `wack-agent` → `wack-llm`, with `wack-tools` beside the agent.

## The one boundary that matters

**Nothing below `wack-ui` may depend on GPUI.** The UI is a viewport onto the engine: it calls typed async commands and subscribes to typed event streams. That keeps the choice of UI framework confined to one crate, and it is why the agent can run and be tested with no window at all (`wack-cli`).

The engine runs on Tokio. `gpui_tokio` bridges the two, so a view can await engine work.

State lives in models, not views. Closing a pane or switching a tab must never lose a terminal's scrollback or a diff's selection.

## Where data lives

```
~/Library/Application Support/com.wackcode.desktop/     (.dev for the dev app)
  usage/v1/...              the TokenTrail ledger: a published contract, see wackcode-usage-v1.md
  v3/state.json             projects, chats, settings
  v3/secrets.json           API keys, mode 0600
  v3/sessions/<chat>.jsonl  one append-only file per chat
  v3/backgrounds/           backdrop images
```

Version 3 keeps everything of its own under `v3/` and never reads or writes version 2's files, which share the same folder.

## The agent core

Each file's header comment states its invariants; this is the map.

- **`wack-llm`** turns one internal `Request` into three wire formats (`anthropic.rs`, `responses.rs`, `chat.rs`) and their event streams back into one `Response`. Blocks a provider needs back unchanged (thinking signatures, encrypted reasoning, its own tool-search blocks) are kept as `Opaque` and replayed only to the format that made them. `retry.rs` retries rate limits, overload and network failures, never once any text has reached the user. `testing.rs` has the scripted provider every test above this crate uses.
- **`wack-agent/agent.rs`** is the turn loop. Every tool call gets exactly one result in the next message, whatever happens (refused, failed, stopped). Calls that only read run side by side; anything else runs alone, in the model's order. A message sent mid-run rides along with the next tool results.
- **Deferred tools** (`registry.rs`): a tool is `Always` or `Deferred`. Anthropic's own endpoint gets deferred tools marked for its tool search. Everywhere else the model gets our `tool_search`, and what it finds is added to the *end* of the tool list so the cached start of the request never changes. The session records what was found, so a resumed chat offers the same tools. The seven core tools are always loaded; MCP, browser, canvas, computer use and web tools will be deferred.
- **Sessions** (`session.rs`): one append-only JSON-lines file per chat, linear. Opening one drops a cut-off last line and answers any tool call a crash left unanswered.
- **Compaction** (`compaction.rs`): at 80% of the model's context window (when the connection states one), or when the provider refuses a request as too long. The model writes the summary's body; the user's own messages and the lists of files read and changed are carried in code.
- **The prompt** (`prompt.rs`): the profile's core text, a guide to this chat (date, folder, tool list), then `AGENTS.md` / `CLAUDE.md` from the working directory and every folder above it. Built once per run.
- **Features** hook in through `agent::Feature` (`before_request`, `on_tool_call`, `after_turn`): compiled in, no extension runtime.
- **`wack-core/store.rs`** holds connections in `state.json` and their keys in `secrets.json`; `Store::provider` is the only place a key leaves the secret store.

## The UI layer

- **`app.rs`** (`AppState`) owns the engine's `Store`, what is selected, and a `ChatSession` for every chat opened since launch. Every change goes through its methods.
- **`chat.rs`** (`ChatSession`) is one chat's live state: its conversation, the transcript items built from it, its composer, and the run on Tokio. Runs are numbered so a finished run's task can never touch the next one.
- **`actions.rs`** lists what the user can do as GPUI actions. Views dispatch them; `shell.rs` handles them by calling `AppState`, so views hold no handle on the app and gallery scenes stay inert.
- **`conversation.rs`** folds saved history and live events into turns; **`work.rs`** turns those into the rows the transcript draws; both are pure and tested.

## Status

Built so far:

- The workspace, the pinned GPUI dependency, a themed window, and the gallery.
- The theme derived from appearance, theme colour and accent (`theme.rs`, `colour.rs`); the type scale (`typography.rs`); icons.
- The agent core, headless: the three provider formats, the loop, deferred tool loading, sessions, compaction, the seven core tools, connections, and the `wack` CLI.
- The minimal harness, wired to the engine: projects (add, pin, remove) and their chats in the sidebar; the new-chat screen; a streaming transcript with work logs, diffs and highlighted code; the composer with queue (↩ while working), steering (⌘↩), stop (Esc), the context meter and Compact; the model and effort menus; Settings › Providers. Verified in the dev app against the Testing connection.

Known gaps in the agent core, to close as the features that need them arrive:

- Only the Chat Completions format has run against a live endpoint (the Testing connection: multi-tool tasks, resume across models, stop mid-command, a message sent mid-run). The Anthropic and OpenAI Responses formats are tested against recorded streams only.
- Tool results are text only: `read` refuses images.
- `bash` inherits the environment of the process that started it. Resolving the user's login-shell environment (for an app launched from the Dock) comes with the app.
- Nothing writes the usage ledger yet.

Known gaps in the harness:

- Renaming a chat and searching chats are not built.
- A stopped turn reads "Stopped." only until the app restarts: the session file does not record it.

Next, in order:

1. Panes and chat tabs: the pane tree and solver, Changes, Terminal.
2. Identity: theme colour and accent, image backdrop, motion.

After that foundation, features return in this order: Plan and Ultra Plan with the question dialog, todo list and Ultra reasoning; Memory, Build and Run, auto titles, skills and slash commands, background shell jobs, commit / push / PR, the usage ledger writer; MCP and web tools; sub-agents and task suggestions; the browser pane, visual replies and canvas; the Chat area; computer use, the skill creator, signature motion and Liquid Glass.

Update this section as each piece lands.
