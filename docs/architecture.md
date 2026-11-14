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

## Status

Built so far:

- The workspace, the pinned GPUI dependency, a themed window, and the gallery.

Next, in order:

1. The agent core, headless: provider formats, the loop, deferred tool loading, sessions, compaction, the seven core tools, the `wack` CLI.
2. The minimal harness: sidebar with pinned projects, composer, model picker, streaming transcript, provider settings.
3. Panes and chat tabs: the pane tree and solver, Changes, Terminal.
4. Identity: theme colour and accent, image backdrop, motion.

After that foundation, features return in this order: Plan and Ultra Plan with the question dialog, todo list and Ultra reasoning; Memory, Build and Run, auto titles, skills and slash commands, background shell jobs, commit / push / PR, the usage ledger writer; MCP and web tools; sub-agents and task suggestions; the browser pane, visual replies and canvas; the Chat area; computer use, the skill creator, signature motion and Liquid Glass.

Update this section as each piece lands.
