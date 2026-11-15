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
- **`panes/`** is the pane system. `layout.rs` is a chat's two docks (right of the transcript, and along the bottom under both) as a tree of weighted splits and tab stacks; `solve.rs` turns the window size and that tree into rectangles for the sidebar, the transcript and every stack. Both are pure and property-tested: everything tiles exactly, the transcript never drops below 420 × 240 at any window the app allows (700 × 480 and up), and the sidebar steps aside before a dock is clamped. `dock.rs` draws the result; `changes.rs` and `terminal.rs` are the two panes. Each chat owns its panes (`ChatPanes` in `ChatSession`), so they outlast switching chats, and its layout is saved to `state.json` shortly after it changes.
- **Terminals** run in the engine (`wack-core/terminal.rs`, on `alacritty_terminal`): a login shell in the project folder, its screen handed to the UI as a plain `Snapshot`. The UI paints the grid itself, with the ANSI colours as theme tokens.
- **Appearance** is kept by the engine (`wack-core/appearance.rs`: light or dark, theme colour, accent, backdrop) and turned into the theme by `theme::apply`. A chosen picture is copied into `v3/backgrounds/` as WackCode's own JPEG, with a small blurred copy beside it, so the original can move. **`backdrop.rs`** draws it the way WackCode 2 and Zeron do: softened and dimmed behind the whole window on every screen, with the sidebar and title strip see-through over it; and clear across the top of the new-chat and welcome screens, optionally through a texture (Dither, ASCII, Halftone, Scanlines, after Zeron), fading out downwards and feathered away around the centred block by a GPU mask. Textures are made once per picture off the UI thread. The transcript fades out at its edges with the fork's edge fade (`edge_fade.rs`), so nothing paints a band across the picture. Text stays readable over the picture as in WackCode 2: each picture is measured on import for how dark and light its softened copy gets, and the theme moves the quieter steps of ink towards the loud end by one shared amount (keeping their order) until each clears a floor over the worst part, dimming the picture beyond the user's choice only when that is not enough (`theme.rs`, `Theme::veil`).
- **Motion** is in one place, `motion.rs`: arrivals on a critically damped spring (screens, menus, a settings section), dock panes sliding in from their edge, and the working duck's stepped paddle. Everything follows macOS's Reduce Motion, read at launch and whenever the window comes forward.

## Status

Built so far:

- The workspace, the pinned GPUI dependency, a themed window, and the gallery.
- The theme derived from appearance, theme colour and accent (`theme.rs`, `colour.rs`); the type scale (`typography.rs`); icons.
- The agent core, headless: the three provider formats, the loop, deferred tool loading, sessions, compaction, the seven core tools, connections, and the `wack` CLI.
- The minimal harness, wired to the engine: projects (add, pin, remove) and their chats in the sidebar; the new-chat screen; a streaming transcript with work logs, diffs and highlighted code; the composer with queue (↩ while working), steering (⌘↩), stop (Esc), the context meter and Compact; the model and effort menus; Settings › Providers. Verified in the dev app against the Testing connection.
- Identity: Settings as a full page with its sections down the left; Settings › Appearance with light or dark, theme colour and accent (paint chips and a hex field, applied as you choose), and an image backdrop with a texture, a dim and a focus point; the chat column and the new-chat and welcome screens centred; text kept readable over the picture; the motion kit. Verified in the dev app in light and dark, with and without a backdrop, and signed off by the owner on 2026-11-15.
- Panes and chat tabs: per-chat docks that split, stack, tab and resize (drag a tab onto a stack's middle or edge, drag the lines, double-click a line to reset it); the Changes pane (files changed since the last commit and their diffs, refreshed when shown, after each tool and each run); the Terminal pane; chat tabs across the title strip, saved with the chat in front and whether the sidebar is hidden. Keys: ⌘⇧G Changes, ⌃` terminal, ⌃⇧` new terminal, ⌃⌘S sidebar, ⌘W closes the focused pane or the tab, ⌘⇧[ ⌘⇧] or ⌃⇥ between tabs. Verified in the dev app from 700 to 2,500 points wide.

Known gaps in the agent core, to close as the features that need them arrive:

- Only the Chat Completions format has run against a live endpoint (the Testing connection: multi-tool tasks, resume across models, stop mid-command, a message sent mid-run). The Anthropic and OpenAI Responses formats are tested against recorded streams only.
- Tool results are text only: `read` refuses images.
- `bash` inherits the environment of the process that started it. Resolving the user's login-shell environment (for an app launched from the Dock) comes with the app.
- Nothing writes the usage ledger yet.

Known gaps in the harness:

- Renaming a chat and searching chats are not built.
- A stopped turn reads "Stopped." only until the app restarts: the session file does not record it.
- Panes are only on chat screens: the new-chat screen has none until the first message is sent.
- A terminal saved in a layout comes back as a fresh shell after a relaunch; its scrollback is not kept.
- The terminal has no IME composition, no mouse reporting to programs, and no links yet. Scrolling with the wheel is built but has not been driven by the automation socket, which cannot scroll.

Known gaps in identity:

- The hero appears with a fade but leaves at once when a chat starts; Zeron dissolves it.
- The window's own controls follow macOS's light or dark, not the one chosen in Settings.

The foundation (phases 0 to 4) is complete: the owner signed off its look on 2026-11-15. Features now return in this order, each with its own plan: Plan and Ultra Plan with the question dialog, todo list and Ultra reasoning; Memory, Build and Run, auto titles, skills and slash commands, background shell jobs, commit / push / PR, the usage ledger writer; MCP and web tools; sub-agents and task suggestions; the browser pane, visual replies and canvas; the Chat area; computer use, the skill creator, signature motion and Liquid Glass.

Update this section as each piece lands.
