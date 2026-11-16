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
  v3/tasks/<chat>/<id>.log  output of the commands the agent ran in the background (removed with the chat)
  v3/backgrounds/           backdrop images
  v3/memory/<label>-<key>/  one project's memory notes (Markdown with frontmatter), origin.json, .trash/
  v3/commands/<name>.md     the user's own slash commands (Settings › Commands)
~/.agents/skills/           the user's skills, shared with other tools; the only skill folder WackCode writes to
```

Version 3 keeps everything of its own under `v3/` and never reads or writes version 2's files, which share the same folder. Skills also load (read-only) from other tools' folders the user switches on (`~/.claude/skills`, `~/.codex/skills`, `~/.config/opencode/skills`) and folders they add; a project's own skill folders are never read.

## The agent core

Each file's header comment states its invariants; this is the map.

- **`wack-llm`** turns one internal `Request` into three wire formats (`anthropic.rs`, `responses.rs`, `chat.rs`) and their event streams back into one `Response`. Blocks a provider needs back unchanged (thinking signatures, encrypted reasoning, its own tool-search blocks) are kept as `Opaque` and replayed only to the format that made them. `retry.rs` retries rate limits, overload and network failures, never once any text has reached the user. `testing.rs` has the scripted provider every test above this crate uses.
- **`wack-agent/agent.rs`** is the turn loop. Every tool call gets exactly one result in the next message, whatever happens (refused, failed, stopped). Calls that only read run side by side; anything else runs alone, in the model's order. A message sent mid-run rides along with the next tool results.
- **Deferred tools** (`registry.rs`): a tool is `Always` or `Deferred`. Anthropic's own endpoint gets deferred tools marked for its tool search. Everywhere else the model gets our `tool_search`, and what it finds is added to the *end* of the tool list so the cached start of the request never changes. The session records what was found, so a resumed chat offers the same tools. The ten core tools (the seven in `wack-tools` and the three for background commands) are always loaded; MCP, browser, canvas, computer use and web tools will be deferred.
- **Sessions** (`session.rs`): one append-only JSON-lines file per chat, linear. Opening one drops a cut-off last line and answers any tool call a crash left unanswered.
- **Compaction** (`compaction.rs`): at 80% of the model's context window (when the connection states one), or when the provider refuses a request as too long. The model writes the summary's body; the user's own messages and the lists of files read and changed are carried in code.
- **The prompt** (`prompt.rs`): the profile's core text, a guide to this chat (date, folder, tool list), then `AGENTS.md` / `CLAUDE.md` from the working directory and every folder above it. Built once per run.
- **Features** hook in through `agent::Feature` (`on_run_start`, `notes`, `before_request`, `on_tool_call`, `after_turn`, `on_run_end`, `state_to_save`, `summary_sections`): compiled in, no extension runtime. A feature keeps its state in the session (`Entry::State`, latest per key, kept through compaction) and tells the model things through app notes (user-role messages the transcript hides and compaction never quotes as the user's words). A tool can end the run (`ToolSpec::ends_run`) or wait on the user (`ToolContext::user`, answered through `Control::answer`). Separately, the loop reports every model invocation to a `usage::Recorder` (`wack-agent/usage.rs`): one report per invocation, with its purpose, duration, outcome and counts; the engine's ledger writer implements it, and title requests report through it too.
- **Shell tasks** (`tasks/`): every `bash` command is a task of the chat's `Tasks`, which the chat owns (not the run), so a server outlives the turn. A foreground command is a task the call waits on; when the model asks (`run_in_background`), its timeout passes (120 s by default: it moves, it is not killed) or the user presses ⌘B, it carries on in the background and its result tells the model, once, everything it needs. There is no status tool: a task's end (and each batch of a `watch`'s lines) reaches the model as a note before its next request, or, when the reply was the last, keeps the run going (`Feature::pending`). In an idle chat the app wakes it (`Agent::wake`, `Job::Wake`) unless a plan or a queued message waits, the user stopped the last run, or it has woken three times since they last wrote. `stop_task` only stops; `wait_for_tasks` is the one bounded wait (270 s, once per reply). Stopping a run kills only its foreground command. The session's `tasks` state lets the next run say which tasks a quit killed.
- **Plan and Ultra Plan, questions, todos** (`plan/`, `ask.rs`, `todo/`, assembled by `kit.rs`), ported from version 2 with its prompts: the mode contract is a note, re-sent only when the mode in force differs from the last one the model saw; while planning, a tool runs only if it changes nothing (`parallel_safe`) or, for `bash`, passes the shell policy (`plan/policy.rs`, fail-closed), unless Settings › Tools lifts that. `plan_mode_complete` ends the run with the plan saved as state `plan`; a new user message consumes it. The tool list and system prompt never change between modes, so the prompt cache holds. **Ultra** reasoning is `wack_core::Reasoning::Ultra`: the provider gets its maximum effort and the system prompt gains `prompt::ULTRA_PROMPT`; it never reaches `wack-llm`.
- **Memory** (`memory/`, the folder chosen by `wack-core/memory.rs`), ported from version 2 with its tool texts: `memory_save`, `memory_recall` and `memory_forget` work on one project's note files, keyed on the repository's common root so worktrees share them. The kit reads the index once per run and adds it to the system prompt after the project instructions and before Ultra; bodies never enter the prompt. Memory switched off (Settings › Memory, globally or per project) means no folder for the run: no tools and no index. Plan mode lets the memory tools through. Version 2's notes are not imported.
- **Skills and slash commands** (`skills/`, `commands/`; folders and switches in `wack-core/skills.rs`), ported from version 2's rules: skills are found in the roots the engine names, first name wins, a switched-off skill frees its name. The kit lists every skill the model may choose (name, description, file) in the system prompt, read once per run; the model loads one with `read`, so there is no skill tool. A leading `/name` is expanded in the engine (a skill's `<skill>` block, a template's `$1`/`$ARGUMENTS`, `/init`'s prompt) before the run, the same for the app and `wack run`, and the session keeps what the user typed beside the expanded message (`Entry::Message::command`) so the transcript shows `/name · args`. `/new`, `/name`, `/copy` and `/compact` act on the app and never reach the model.
- **`wack-core/store.rs`** holds connections in `state.json` and their keys in `secrets.json`; `Store::provider` is the only place a key leaves the secret store. A connection switched off stays stored with its key but is skipped by `Store::resolve`, the model menu and `wack run`.

## The UI layer

- **`app.rs`** (`AppState`) owns the engine's `Store`, what is selected, and a `ChatSession` for every chat opened since launch. Every change goes through its methods.
- **`chat.rs`** (`ChatSession`) is one chat's live state: its conversation, the transcript items built from it, its composer, and the run on Tokio. Runs are numbered so a finished run's task can never touch the next one.
- **`actions.rs`** lists what the user can do as GPUI actions. Views dispatch them; `shell.rs` handles them by calling `AppState`, so views hold no handle on the app and gallery scenes stay inert.
- **`conversation.rs`** folds saved history and live events into turns; **`work.rs`** turns those into the rows the transcript draws; both are pure and tested.
- **`panes/`** is the pane system. `layout.rs` is a chat's two docks (right of the transcript, and along the bottom under both) as a tree of weighted splits and tab stacks; `solve.rs` turns the window size and that tree into rectangles for the sidebar, the transcript and every stack. Both are pure and property-tested: everything tiles exactly, the transcript never drops below 420 × 240 at any window the app allows (700 × 480 and up), and the sidebar steps aside before a dock is clamped. `dock.rs` draws the result; `changes.rs` and `terminal.rs` are the two panes. Each chat owns its panes (`ChatPanes` in `ChatSession`), so they outlast switching chats, and its layout is saved to `state.json` shortly after it changes.
- **Terminals** run in the engine (`wack-core/terminal.rs`, on `alacritty_terminal`): a login shell in the project folder, its screen handed to the UI as a plain `Snapshot`. The UI paints the grid itself, with the ANSI colours as theme tokens.
- **The window** (`shell.rs`) is a frame around two panes, after Zeron: the sidebar, and the *stage* holding the screen (a chat's transcript and docks, a new chat, Settings), with the chat tabs on pills across the title strip between them. Every edge keeps an 8-point gutter; the pane solver works in the frame's inner space (`shell::solver_space`, `shell::to_solver`), so its guarantees hold and the window's minimum is the solver's plus the gutters.
- **Appearance** is kept by the engine (`wack-core/appearance.rs`: light or dark, one of WackCode 2's ten themes by name, and a library of backdrop pictures, at most one shown, each with its blur, dim, texture, focus and the theme it brings back) and turned into the theme by `theme::apply`. A chosen picture is copied into `v3/backgrounds/` as WackCode's own JPEG, with a small blurred copy beside it, so the original can move. **`backdrop.rs`** draws it behind the whole window, blurred and dimmed exactly as the user chose (a heavy blur starts from the blurred copy so the GPU does less), and the sidebar and stage are frosted glass over it (`glass.rs`: GPUI's backdrop blur with the window's colour over it). On the new-chat and welcome screens the whole stage shows it clear and undimmed instead, optionally through a texture (Dither, ASCII, Halftone, Scanlines, after Zeron), with its smaller words on frosted pills and the display line ringed by a faint halo, nothing laid over the picture; when a chat opens it fades away to the glass, and back on New chat (`motion::Ease`). Textures are made once per picture off the UI thread. The transcript fades out at its edges with the fork's edge fade (`edge_fade.rs`), so nothing paints a band across the glass. Text stays readable over the picture as in WackCode 2: each picture is measured on import for how dark and light its softened copy gets, and the theme moves the quieter steps of ink towards the loud end by one shared amount (keeping their order) until each clears a floor on the glass over the worst part, thickening the glass past its thinnest only when that is not enough (`theme.rs`, `Theme::glass`). The user's dim is never overridden.
- **Planning in the window**: the composer's Build | Plan toggle (⇧Tab cycles Build, Plan, Ultra Plan) sets `Chat::mode`, the truth between runs; `conversation.rs` folds the session's `plan` and `todo` state, so the mode in force, the plan awaiting review and the todo list read the same after a relaunch. Over the composer a row of status pills reports what a run is doing and for how long, the todo list and the waiting messages; the list (`todo_panel.rs`) and the messages open above their pills, in a column capped so some transcript always shows. The question card (`question.rs`) comes first: while it is open the pills only report. The plan card (`plan_card.rs`) is the transcript's hero item; only the plan still awaiting review has actions. Ultra's look (`ultra.rs`) is a fixed prism in the theme, never the accent.
- **Motion** is in one place, `motion.rs`: arrivals on a critically damped spring (screens, menus, a settings section), dock panes sliding in from their edge, and the working duck's stepped paddle. Everything follows macOS's Reduce Motion, read at launch and whenever the window comes forward.

## Status

Built so far:

- The workspace, the pinned GPUI dependency, a themed window, and the gallery.
- The theme derived from appearance and one of ten fixed themes, four with an accent sweep (`theme.rs`, `colour.rs`); the type scale (`typography.rs`); icons.
- The agent core, headless: the three provider formats, the loop, deferred tool loading, sessions, compaction, the core tools, connections, and the `wack` CLI.
- The minimal harness, wired to the engine: projects (add, pin, remove) and their chats in the sidebar; the new-chat screen; a streaming transcript with work logs, diffs and highlighted code; the composer with queue (↩ while working), steering (⌘↩), stop (Esc), the context meter and Compact; the model and effort menus; Settings › Providers, with an overview, a connection editor (its address, key and model rows) and a save bar. Verified in the dev app against the Testing connection, and the connection editor with a throwaway connection.
- Identity: Settings as a full page with its sections down the left; Settings › Appearance with light or dark, a theme from a fixed set (phase 6 of the redesign replaced the earlier paint chips and hex fields), and an image backdrop with a texture, a dim and a focus point; the chat column and the new-chat and welcome screens centred; text kept readable over the picture; the motion kit. Verified in the dev app in light and dark, with and without a backdrop, and signed off by the owner on 2026-11-15.
- Panes and chat tabs: per-chat docks that split, stack, tab and resize (drag a tab onto a stack's middle or edge, drag the lines, double-click a line to reset it); the Changes pane (files changed since the last commit and their diffs, refreshed when shown, after each tool and each run); the Terminal pane; chat tabs across the title strip, with whether the sidebar is hidden saved (the app opens on a new chat, not the last one, as version 2 did). Keys: ⌘⇧G Changes, ⌃` terminal, ⌃⇧` new terminal, ⌃⌘S sidebar, ⌘W closes the focused pane or the tab, ⌘⇧[ ⌘⇧] or ⌃⇥ between tabs. Verified in the dev app from 700 to 2,500 points wide.

- Skills and slash commands: the composer's `/` menu (opens on a `/word`, ↑↓ ⇥ ↩ ⎋, picking moves the command to the front), the built-ins, skills and user commands, the transcript's `/name · args` bubble with the prompt sent behind it, "Used skill" rows, Settings › Skills (folders, per-skill switches, an editor, import from a folder, `.md` or `.zip`, Copy to Your skills) and Settings › Commands (switches, an editor with a live preview), and `wack skills` / `wack commands`. Verified in the dev app against the Testing connection (a user command run end to end, an unknown command, `/name`) and over a backdrop.

- Background shell tasks, rebuilt rather than ported from version 2's `bash_job`: one registry per chat that outlives runs, `bash`'s `run_in_background` and timeout-to-background, ⌘B, `stop_task`, `wait_for_tasks`, `watch`, news delivered as notes and waking an idle chat (capped), transcript rows with each command's description and the news on its own, the tasks pill and its list (output, Stop), the Tasks pane, Plan mode's gates, and `wack run` staying up while commands run. Verified headless against the Testing connection (a server started, used and stopped; a slow build moved to the background and reported). Not yet: quiet and stall check-ins for a command that hangs, and a dot on an idle chat's tab while its commands run.
- The usage ledger, ported from version 2 with its tested writer logic (`wack-core/usage.rs`): one record per model invocation — chats, compactions, automatic titles and `wack run` — appended to `usage/v1/YYYY-MM/<writer-uuid>.jsonl` beside `v3`, byte-identical in shape to version 2's records (the additive `provider_name` kept, `mode` waiting for the Chat area). The writer queues (cap 1024, drop-oldest), flushes as records arrive, retries every 5 seconds, repairs only its own unfinished tail, and keeps files 0600 in 0700 folders; months are UTC. Recording is on by default, and Settings › Integrations turns it off and shows the ledger's health (state, last record, a waiting queue, losses, failures, the folder). The dev app's ledger stays in the dev app's folder. Verified in the dev app against the Testing connection and headless with the fixture, which the writer reproduces byte-for-byte.

- Plan and Ultra Plan, the question card, the todo list and Ultra reasoning, ported from version 2 with its prompts and tested logic: the mode toggle and ⇧Tab, the interview with Write the plan now, the plan card (Approve & implement, Save PLAN.md, Copy, Discard), the todo list folding finished tasks, agent-initiated Plan, Settings › Tools' unrestricted planning, and `wack run --mode` / `--effort ultra`. Verified in the dev app against the Testing connection (Ultra Plan interview to plan to implementation, relaunch mid-plan, Discard, a Build run keeping a todo list) and headless.

Known gaps in the agent core, to close as the features that need them arrive:

- Only the Chat Completions format has run against a live endpoint (the Testing connection: multi-tool tasks, resume across models, stop mid-command, a message sent mid-run). The Anthropic and OpenAI Responses formats are tested against recorded streams only.
- Tool results are text only: `read` refuses images.
- `bash` inherits the environment of the process that started it. Resolving the user's login-shell environment (for an app launched from the Dock) comes with the app.
- Settings › Prompts (editing the built-in prompts, as version 2 allowed) is deferred; the prompt builders are written so overrides can slot in.
- Planning's sub-agent gate returns with that feature.

Known gaps in the harness:

- Searching chats is not built.
- A stopped turn reads "Stopped." only until the app restarts: the session file does not record it.
- Panes are only on chat screens: the new-chat screen has none until the first message is sent.
- A terminal saved in a layout comes back as a fresh shell after a relaunch; its scrollback is not kept.
- The terminal has no IME composition, no mouse reporting to programs, and no links yet. Scrolling with the wheel is built but has not been driven by the automation socket, which cannot scroll.

Known gaps in identity:

- The UI redesign (docs/redesign.md) is under way: the frame, stage, glass, sidebar, transcript, composer and Settings are built; WackCode 2's themes are next.

- The window's own controls follow macOS's light or dark, not the one chosen in Settings.

The foundation (phases 0 to 4) is complete: the owner signed off its look on 2026-11-15. Features now return in this order, each with its own plan: Plan and Ultra Plan with the question dialog, todo list and Ultra reasoning (done); Memory (done); Build and Run (done); auto titles and renaming a chat (done); skills and slash commands (done); background shell jobs (done); the usage ledger writer (done); git; MCP and web tools; sub-agents and task suggestions; the browser pane, visual replies and canvas; the Chat area; computer use, the skill creator, signature motion and Liquid Glass.

Update this section as each piece lands.
