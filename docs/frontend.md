# The renderer

`src/` is a React 19 app in a WKWebView. It never fetches anything and never holds a credential: everything goes through Rust. Visual rules are in [design.md](design.md).

## State and data flow

- **`App.tsx`** owns all cross-cutting state: `data` (the `AppData` from `bootstrap`) and each chat's runtime. Update it immutably through `patchTask`, `patchRuntime` and `setData`.
- **Events:** `App.tsx` is the only place that listens to Tauri events: `worker-event`, `run-event`, `subscription-login-event` and `native-chat-navigation` (the menu bar). A worker protocol change ends in its `worker-event` switch; see [worker.md](worker.md#protocol-changes). `title_changed` carries a `source` — `"opening"` for the first-run stand-in, `"auto"` for the model's title; only the latter bumps `titlePulses`, which drives the header's swipe+glint and the sidebar row's crossfade.
- **`api.ts`** is the only bridge to Rust: one typed `invoke` wrapper per command. Terminal output arrives on a `Channel`, not an event.
- **Components are presentational.** The exceptions call `api` directly because they wrap one self-contained surface: `SettingsPage.tsx` (and `IntegrationsSection.tsx`, `AboutSection.tsx`), `BrowserPanel.tsx` (positions the native web view), `TerminalPanel.tsx` and `RunPanel.tsx` (stream their PTYs) and `markdown-components.tsx` (its link override routes http(s) clicks through `revealPath`; the webview cannot open links itself) and `link-opener.ts` (a web_search result opens in the chat's browser preview through App's opener, otherwise through `revealPath`). Don't add more without the same reason.

## Context menus

`ContextMenuProvider` owns one pointer-anchored renderer menu, using the shared `Menu` primitive. Components supply existing action callbacks through `useContextMenu`; they never call the bridge for menu actions. Text fields take precedence over enclosing targets; selections and HTTP(S) links add their own actions. The app supplies native clipboard callbacks (reads only after Paste text) and the usual external-link opener. Navigation, scroll, resize and loss of window focus dismiss captured actions. The per-chat native browser preview remains a separate webview with its own page interaction.

## Transcript snapshots

Opening a cold chat emits a saved-history snapshot without starting a worker; opening a live chat keeps the worker authoritative. The worker sends full snapshots and deltas ([worker.md](worker.md#snapshots-and-deltas)); `applySnapshotDelta` in `chat-utils.ts` merges them. A new field that changes mid-run needs updates on both sides. Keep the full-snapshot fallback, and keep unchanged messages as the *same objects*: transcript rows are memoized by identity.

A plain send is echoed while it travels: the composer has already cleared, and the real message can sit behind a worker respawn, a workspace checkpoint and MCP servers connecting. `promptTask`/`sendPrompt` set `runtime.pendingMessage` (`pendingEchoMessage`); the transcript appends it through `withPendingEcho` until a user message of the run arrives or the run settles (`run_state` idle/interrupted, or a rejected prompt, clears it). The hero send hides the echo while its frozen-composer handoff still shows the text. Everything else reads the worker's snapshot only — queued sends, commands and resends deliberately have no echo.

The composer never states the active policy: a persistent read-only badge lived next to the Build/Plan toggle and was removed. What remains is the queue notice: the worker's applied `executionPolicy` wins during a run, and a mismatch with saved Settings shows “Applies next turn”. Effective access is shown in Settings (› Tools and › Sub-agents). Cold history carries no policy: a new worker always receives current Settings, never permissions from session history.

Images a tool returned (a picture `read`, a screenshot, an MCP tool's output) show as small previews under the tool's row or chip strip (`ToolImages`), each opening in `ui/ImageLightbox` with the original from `api.toolImage`. Inside a folded Explored group they sit in a captioned strip under the fold and move under their own chips when it opens. Chat mode's chips show the same previews inline.

## Transcript work surfaces

The Code transcript borrows Chat's look for its work but keeps its own components and `.tool-*`/`.explore-*` classes (it never uses `.chat-*` rules, and Chat never uses these).

- **Chips vs rows** (`toolPresentation` in `tool-utils.ts`): edits, writes and shell commands not known to only read are full-width rows; every other call is a pill chip with the same `summarizeTool` words. Plan, sub-agent and skill cards are unchanged. Neighbouring chips within one message flow in one `ToolChips` strip with a single shared sheet; any other slot closes the strip. Open state lives in `ToolExpansion` under the row's key.
- **Explored** (`ExploreGroup.tsx`): the fold rule is untouched (`MIN_GROUP_CALLS`). The head shows only settled items (`settledGroup`): an icon stack (`exploreMarks`) and rolling counts. Calls in flight are live chips under the head (`exploreLiveItems`).
- **The fly-in:** a chip that may be taken into a group, a reasoning row (`FlyingThought`) and the head's marks share a motion `layoutId` (`flyId`), so when the layout moves an item into a head its new mark resumes from the chip's box. Hand-off only happens between nodes whose `layoutDependency` differs (`"chip"`, `"thought"`, `"mark"`); keep them distinct. `.conversation-scroll` is a `motion.div` with `layoutScroll` so follow-scrolling doesn't skew the flight. Motion is off under reduced motion and in jsdom (`useTranscriptMotion`), where the swap is instant.
- **Sheets** (`ToolDetail.tsx`): what a chip or row opens into, shared with Chat's chips. One solid well with a head (a round mark, then the call in full) over a body per kind: a command's output, a read's numbered and highlighted lines, a grep's hits grouped by file, a listing's entries, a question with the answer picked, or the arguments as labelled rows in place of JSON. `tool-output.ts` takes Pi's result text apart (the cut-short note, a command's exit, grep rows) and gives up on any other shape, where the sheet shows the text whole. Line numbers and the `$` prompt are CSS, so a selection copies source text only; copy buttons stay on what the agent authored. A work row is its own sheet's head: `.tool-row` is the card, its bar the top band, and `ToolDetail` gets `head={false}` so it opens straight into the body (a command still leads with itself in full, and the bar's shortened copy is clipped while open). Code fences (`markdown-components.tsx`) wear the same head band and seam.
- **Entering:** a call pops in only the first time this renderer draws it while running (`usePopsIn`), so history and the streamed→saved remount never pop.
- Tests pin accessible names ("Explored 2 files, 1 search", "Thinking… 2s", verbs) and `.agent-working`'s position; keep those when restyling.

## Completed-work folding

`completed-work.ts` projects each user turn into work and outcome slots without changing the
normalized messages or their block indices. `turn` identifies the latest assistant, not run
completion: all user turns at/after `activeRun.startedAt` stay open, including steering turns.
Only settled successful turns with both work and a final answer/plan fold. Missing timings use
“View work”; interrupted, failed, unfinished, tool-only and system-notice turns stay detailed.
The latest successful PlanCard and terminal actions remain outside the fold. Skill-creator
review cards do too: a `skill_creator` preview result renders as the `SkillDraftCard` (the
newest one on the branch actionable; older ones inert), with its publication state hydrated
from the host and its Save publishing through Rust, never the agent. The Appearance
preference defaults on; sub-agent inspection transcripts explicitly opt out.

`WorkTurn` deliberately uses `getSnapshotBeforeUpdate` to capture a reading/focus anchor before
an automatic collapse removes it, including React focus events from work-owned screenshot
portals. Automatic completion always collapses, even during inspection. Manual toggles pause
following and hold the disclosure; automatic folds retain bottom-pinning or hold the surviving
reading row (the disclosure when that row disappears). `useFollowScroll` reconciles anchors on
content resize so motion's height changes between commits do not yank the viewport. User
scrolling releases the anchor. Work/outcome slices must have distinct `data-transcript-anchor`
keys and signature-cache variants. Turn wrappers stay **unpositioned**, so user `offsetTop`
still resolves in scroller coordinates for the rail. Closed bodies unmount after their reduced-
motion-aware exit. Disclosure choices are transient, keyed by chat and actual entry/outcome,
never shared by sibling versions; only the app-wide preference persists.

Compaction boundaries stay inspectable outside completed-work folds. A boundary after the
answer accompanies the outcome; a turn compacted mid-work retains its chronological detail.
Summary disclosure choices are transient and scoped to the chat and entry id. Live compaction
status is separate from the prompt clock; a clock renders only under its user message, never
as an unanchored row at the response tail. While compaction runs the tail shows
`CompactingStage`, a canvas gravity well that escalates through calm/busy/dramatic acts at
20s/60s (with a rare duck cameo) and collapses into the newly-saved boundary row when it
finishes; the status line under it carries the trigger reason and a wall-clock elapsed timer.

## Optional chat tabs

`appearance.chatTabs` defaults off. `App` owns `useChatTabs` and the pure rules in
`chat-tabs.ts`: ordered stable tab ids, selection, taskless draft configuration, panel
state and ten closed records. The strip mounts only once the area holds a second tab —
one open chat keeps the tabs-off layout — while every tab rule still applies
underneath. None of this workspace state is serialized; quitting or a
renderer reload discards it, while hiding the window keeps it. A tab's id
and composer key survive draft-to-chat binding, including binding a closed recovery
record after a send finishes. Saved-chat composer keys also survive eviction from
the recovery shelf, so failed input remains available through the sidebar.
Opening an existing task deduplicates its tab. Closing
never stops a worker, terminal, browser or project command, and tabs do not change reaping.

Only the selected transcript and panel mount. `transcript-view.ts` stores follow-latest,
anchor keys/viewport offsets, a clamped scroll fallback and disclosure sets per task.
`useFollowScroll` waits for cold-history rows before restoring; when an anchor disappears
it uses the nearest surviving saved row. Settings remounts restore the same reading state.
Browser/Terminal/Run/sub-agent selection, panel width and the Changes entry belong to the
tab. The panel keys by tab to retire an outgoing native browser before another mounts;
sub-agent watches still use the single serialized watch queue.

Asynchronous sends and slash preparation capture the originating composer key and tab id. They bind and
report errors there without selecting it, even after closure. Composer updates capture
its stable composer key. Evicting an unsent closed draft retires its key so late attachment
reads cannot resurrect it. Disabling tabs preserves the dormant workspace in memory.
`NavigationScope` dismisses menus, image lightboxes and sent-message editors on navigation. Disclosure sets survive.

The native Tabs menu (`chat_tabs.rs`) owns app-local tab accelerators, including when
a child WKWebView has focus. `api.setChatTabMenu` synchronizes availability, and App is
the only listener for `native-tab-action`. Modal dialogs and Settings disable tab
commands. `⌘⇧T` remains Terminal; `⌘W` closes a tab when enabled and hides the window
in the original layout. The window's close button always hides it.

## Composer drafts

`App` keeps in-memory text, image and file drafts by chat id through `useComposerDrafts`.
The composer stays mounted for the hero→dock animation and reads the selected chat's draft.
⌘N gives the welcome screen a fresh draft key without clearing the previous chat. Async
updates capture their originating key, so a delayed send, file read or queue restore cannot
change another chat's draft. Deleting a chat removes its draft; app exit discards all drafts.
Rewind seeds are consumed once per chat so returning to it cannot overwrite later edits.

Running sends (Enter, ⌥Enter or Queue beside Stop) wait for the active parent run to finish. When `workActivity.parent` is idle and background children remain, the composer shows Send beside Stop: a queued send starts immediately alongside those children. Aggregate status remains running so Git, history and workspace actions stay protected. Background chips follow child lifetime independently of the launch tool; active chips stay outside completed-work folds, and earlier parent answers remain visible beside later continuations. Each
queued row has an explicit Steer action: it interrupts the active work and sends that worker
id next, keeping the rest queued. `queue_state.messages` supplies stable `{ id, text, images? }` rows;
the worker retains their full payloads and images. Restoring returns structured text/image prompts, reconstructs grouped browser chips and retains overflow in the draft with sending blocked until it fits. Queue/run events are authoritative: full
snapshots don't clear the queue, and steering never optimistically removes a row, changes run
status or adds a pending echo. Queue actions are single-flight per draft key, and no sends or
queue actions dispatch while stopping. App captures the originating task id for async errors.

## New-chat project picker

`ProjectBar` (the composer's `header` on the welcome screen) hosts `ProjectPicker`, `BranchPicker` and the Local/Worktree toggle. The picker reads and writes the same `pinnedProjects` set as the sidebar, so a pin made anywhere shows everywhere; it never keeps its own copy. Its display rules (`shortPath`, `monogram`, search) are pure functions in `project-display.ts`. ⌘O stays a global shortcut in `App`; the picker's footer only calls `onAddProject`.

## Attachments

- Images ride `images` through `api.ts` to the worker, and reach only models with Vision on. Browser context chips count toward the same image limits. Their versioned generated text section maps each capture to its image index; `browser-context.ts` owns encoding, parsing and reindexing, including sent-message edits.
- Every attached image opens full size in `ui/ImageLightbox`: the composer shows the picked file it still holds, and the transcript and edit box fetch the original from the chat's session with `api.messageImage` (a snapshot's thumbnail is all they render). In the lightbox, a click zooms toward the clicked point (click again, or −, to zoom out; + zooms to centre) and a zoomed image pans by dragging.
- Attached text files are folded into the message text by `composeFileSection` and read back out for display by `splitFileSection` (`src/attachment-utils.ts`). Change both halves together. A terminal selection is one of these files (`source: "terminal"` only marks its chip with the terminal icon and a line count); it is not its own wire format.
- A plain-text paste of 32 KiB or more (`MIN_PASTE_FILE_BYTES`, UTF-8 bytes) becomes a `Pasted text N.txt` attachment instead of a wall of text in the draft; it meets the same file caps, and a paste that does not fit stays in the draft with a notice explaining why. The notice's **Insert as text** puts it back. Attachments alone can be sent, so a paste-only message works; a queued message made only of files shows their names.
- Attached files never ride a `/`-command message: Pi expands commands and would swallow the section. Sending the same text literally is the deliberate escape hatch.
- **`@` chat references** (`src/chat-refs.ts`) are the same gesture as file mentions, with a different payload: the draft keeps `@"Chat name"` and the message carries only the id in a `<referenced-chats-v1>` tail, so the agent reads that chat on demand (`read_chat`) instead of the prompt carrying it. `rankSuggestions` lists files first, exactly as before, then ≤5 matching chats under a "Chats" heading; `chatRefCandidates` applies the kind rule (Chat mode chats only, for a Chat mode chat) so the picker never offers what the host would refuse. A ref survives only while its token is in the text, at send and in the editor; the transcript highlights it as a chat token and clicks through to that chat (a deleted one stays a plain token).

## Side panel

`SidePanel` shows one `SidePanelView` (`src/side-panel.ts`) at a time: Changes, Browser, Terminal, Run, Games, or one sub-agent's transcript.

- A new view is a union member, a component `SidePanel` renders, and a trigger that opens it (`toggleView`). Give it the shared head: `.panel-header` with a `.panel-mark`, a `.panel-kicker` over an `h3`, and `.panel-header-actions` ([design.md](design.md)).
- In the original layout, Changes, Terminal, Run and Games are durable: which one is showing is remembered across chats and launches (`wackcode:sidePanel`). With chat tabs enabled, every tab keeps its own panel for the session and a new draft starts closed. Run follows the selected checkout and restoring it never launches a command. Browser and sub-agent views belong to one chat; the original layout falls back to the remembered durable view when the chat changes.
- Sub-agent transcripts reach the panel only through `watch_subagent` frames, never through snapshots.
- **Games** (`GamesPanel`, one entry per game in its `GAMES` list): the panel opens on an Arcade home of poster cards; which game was entered lives beside the runs in `games/session.ts`, so a view or tab swap comes back to it paused, while every real close (`exitToArcade`, from the panel's own close paths in `GamesPanel` and `App`'s games toggle) resets to the picker. Each game is a pure simulation under `src/games/` plus a component that owns its canvas and rAF loop. The panel's page remounts on every view or tab switch, so a run in progress lives in `games/session.ts` and comes back paused. Both games autosave through their own versioned `localStorage` envelope, normalised on the way back out (`games/save.ts`, `games/doodle-save.ts`), so a run survives quitting the app; pausing offers **Save & quit**, which hands you back to the chat. Best scores are a `localStorage` convenience (`games/scores.ts`, with a one-time fallback to a game's pre-rename key). The panel takes only the open chat's `status`, so streaming never re-renders it.

## Areas

The app has two peer areas, Code and Chat (`src/areas.ts`). `App` holds which one is on screen in `area`, and `enterArea` is the only way it changes.

- **A chat belongs to one area for life** (`taskArea`, from `TaskRecord.kind`). Every list is built from one area's chats (`areaTasks`, `codeTasks`): the sidebar, Archived, the tab bar. Opening a chat from anywhere else (the tray menu) enters its area first, in `selectTask`.
- **Switching parks, never abandons.** With tabs, both areas' tabs live in one store, each tagged with its `area`; `switchTabArea` parks the open tab and restores the other area's. Without tabs, `parkedViews` holds each area's chat or unsent draft. One store rather than two because a send finishing or a chat binding must find its tab after the user has switched area.
- **A draft's kind is the area on screen when the send starts**, captured before anything is awaited. It is not a `ChatDraft` field: draft edits rebuild that object field by field and would drop it.
- **The Chat area renders nothing from the Code UI.** Instead of `Transcript`, `ChatHeader`, the scroll rail and `Composer`, App renders one `ChatArea` (`src/components/chat/`) with the same state and callbacks. Errors, model problems and notices reach it as `SystemNote` bubbles, not banners. `chatMenu` and `ArchivedList` read `task.kind`.
- **Its side panel is the browser only.** `panelView` is null for anything else there, and the remembered durable view stays Code's: the persistence effect skips Chat.
- **Never call Git for a Chat mode chat** (`refreshChanges` returns early) and never take its snapshot's tool list as the catalogue; see the host's matching rules in [architecture.md](architecture.md).
- **The switch** (`AreaSwitch`) lives in the sidebar's title strip (`.sidebar-titlebar`), beside the traffic lights; the strip's left padding clears them and must follow `trafficLightPosition` in `tauri.conf.json`. A dev build's DEV badge shares the strip, so the segments flex. Don't add a child to `.sidebar`: the Settings rebuild animation addresses its children by position.
- **The duck hops every switch** (`AreaHop`, the last `.app-shell` child): a real change of `area` dims the window for a beat and sends a spotlit duck across in the travel direction — rarely a somersault — then the lights come back up on the arriving area. The overlay is click-through and `aria-hidden`, keyed per switch so rapid toggling restarts it mid-flight instead of stacking. Reduced motion skips it entirely; nothing ever unmounts a finished hop.
- **⌥⌘1 / ⌥⌘2** are matched on `event.code`, because ⌥ changes `event.key`. The native Go menu sends the same commands (`area-code`, `area-chat`) through `native-tab-action`, so they work while a browser page has focus.

## The Chat area (`src/components/chat/`)

A messenger UI for `kind: "chat"` chats, mounted as one `ChatArea` in place of the Code transcript and composer. It never borrows transcript or tool-row components; shared primitives (`DuckMark`, `PonderingDuck`, `Markdown`, `InlineDialog`, `useFollowScroll`, `QueuedRows`, `MentionPicker`, `MentionText`, the attachments hook, `ModelPicker`, `ReasoningToggle`, `ToolDetail`/`ToolImages` for the chip sheet) are the only overlap.

- **`chat-clusters.ts`** turns the flat message list into entries: user bubbles, replies (one per user message — every assistant block until the next user message), time dividers (3-minute gap splits a cluster, 30 minutes draws a rule), compaction notes and system bubbles. Reply keys are `reply-to:${userKey}` and inner piece keys are ordinals, so the partial→saved swap never remounts.
- **Replies** are `AgentReply`: a duck avatar (PonderingDuck while live), the agent's name once per cluster, text bubbles, `ThoughtCaption`s ("Thought it over", tap to unfurl the reasoning), `ActivityChips` and `ScratchpadChips` (paths confined to the scratchpad, revealed via `api.revealPath`). `chat-activity.ts` words the tools in plain past/present tense and folds settled runs of 3+ into "Did N things"; a chip opens a sheet reusing `ToolDetail`.
- **Presence** is `TypingBubble` — three bouncing dots in a bubble with the duck — with moods (typing / pondering while thinking streams / tidying up while compacting). There is no run clock or status timer anywhere in the area.
- **`ChatHero`** is the empty state: a duck afloat on a pond of light, a time-of-day greeting and three of `PROMPT_IDEAS` (picked deterministically from the draft key, `chat-hero.ts`). An idea fills the draft, it doesn't send. On the first send `handoff` freezes the pill's words, the duck dives and the composer glides from hero to dock (`layout` on `.chat-composer`); the pending echo is the first bubble.
- **`ChatComposer`** is the round pill: same `useAttachments`/`QueuedRows`/`useQueueActions` machinery as `Composer`, but no slash menu, workspace file mentions, mode toggle, project bar or stats. Its `@` picker is chats only (`MentionPicker`, shared with `Composer`): Chat mode may reference other Chat mode chats. Enter sends (queues while busy); the foot row keeps `ModelPicker` and `ReasoningToggle`. It measures itself into `--composer-h` on `.chat-area` so the thread pads around it.
- **Errors and questions** are `SystemBubble`s and the shared `InlineDialog`, centred in the thread; an archived chat shows a footer pill with Unarchive instead of the composer.
- **Everything Chat-scoped is a `.chat-*` class** in the `Chat area` section of `styles.css`, plus `[data-area="chat"]` sidebar tweaks; the Code transcript's rules are untouched. Every looping animation has a still end state in the `prefers-reduced-motion` block.

## Changes panel

`ChangesPanel` reads the selected Code chat’s checkout, including its worktree. `App` owns
Git calls and refreshes; the panel, file list, diff and commit dock are presentational.
Chat mode never exposes this view or calls Git.

- Diff comments belong to the chat (`diffComments[taskId]`); Address comments sends them to
  that chat and clears them only after the prompt succeeds.
- Commit handles all changed files or one file, stages them automatically, and guards the
  operation with the displayed changes revision. Discard uses the displayed section revision
  and a danger confirmation, for either a file or one hunk.
- Generated messages use the chat’s own worker and model. Review uses the Reviewer sub-agent.
- Push and PR creation are explicit actions. Local refreshes and branch lists never fetch.

## Terminal

The user's own shell, in `src-tauri/src/terminal.rs`. The agent never sees it.

- One PTY per chat, keyed by task id, spawned lazily in the chat's `workspace_path` with the login-shell environment.
- It is not a worker: it never joins the worker fingerprint and the idle reaper never kills it. It dies only via `kill_for_task` (delete, archive, worktree conversion), when the user ends it, or via `terminate_all` on app exit.
- Output streams over a `Channel<TerminalFrame>` straight into xterm, never through React state. A ~256 KiB scrollback ring buffer replays on reattach; one lock orders replay and live output so nothing is lost or duplicated.
- The agent never sees any of it unless the user says so: a selection rests an **Add to draft** pill at its end row (and a context-menu item), which hands the tidied text to the composer as a `Terminal output N.txt` text-file chip (`terminalOutputFile`/`normalizeTerminalSelection` in `attachment-utils.ts`, `onAttach` follows `BrowserPanel`'s contract). A refusal (the shared file caps) shows as a small notice in the panel.

## Project Run commands

The chat header's split Run/Stop control saves one nullable `ProjectRecord.runCommand` through
`save_project_run_command`. The editor lives beside the button; edits affect only the next run.
`App` owns launch/stop callbacks, the `run-event` listener and checkout lookup. `run-state.ts`
reconciles events and invoke replies by generation/revision, retaining removal watermarks so
late output-state events cannot resurrect a retired session.

`RunPanel` attaches xterm to an existing run with a unique attachment id. Output goes directly
from a Channel to xterm, never through React. Session and attachment ids guard stale frames;
detaching a superseded attachment does not disconnect the current one. Input, resize and
screen clearing work like the manual Terminal. A finished/stopped run keeps its screen until
another launch. Switching between chats in one checkout reuses the same session.

## Browser preview

`browser.rs` owns an ephemeral native WebKit session per chat. A main-thread AppKit container
scales a fixed CSS viewport into `.browser-surface`; changing panel/window size never changes
responsive breakpoints. Presets are mirrored in `browser-context.ts` and Rust's models.
Hidden views remain attached at the selected viewport size for background tool use. Native
window actions use `get_window("main")`: the single-webview wrapper cannot resolve a window
with preview children. Keep observing both the surface and outer drawer, and wait for the
full rectangle to settle before native placement; drawer motion can move it without resizing.

`BrowserPanel` owns one presentation token per effect lifetime (including Strict Mode's
remount). Retirement rejects late placement requests; presenting another owner retires the old
one. Modals and popovers park the native view. Expanded mode spans the main area and keeps the
same composer mounted below it; its height and inline dialogs set `--browser-dock-h`.

Picking intercepts native pointer events, so selecting never activates the page. Capture
reservations serialize with browser tools, leave explicit user control untouched, and end on
completion/cancellation/navigation/closure. DOM inspection runs in WebKit's isolated client
world. The markup editor uses frozen pixels and a bounded geometry index, never the live DOM.
Arrows use their tips, text its anchor, boxes their enclosed elements, and pen strokes their
points; matching prefers specific targets and removes ancestor duplication.

One browser chip bundles image, capture-time URL/viewport and up to eight inert DOM targets.
Its DOM budget is 16 KiB (64 KiB per message), with visible truncation. The image uses the
ordinary image pipeline; metadata lives in a versioned generated prompt tail. Both areas use
`BrowserContextChip`, `MessageBrowserContexts` and the same draft/queue codec. Copies omit
transport text; edits reindex retained images; retries, forks and cold history retain the
stored evidence. An attached chip is inspectable/removable; drawing layers end at attachment.

## Scroll rail

`.conversation-scroll`'s native scrollbar stays hidden; `ScrollRail` replaces it with a turn timeline. Two things are load-bearing: `.conversation-scroll`'s `position: relative`, and the `data-turn` markers on `.msg.user`. Tick offsets come from `offsetTop`, which must resolve in scroll-content coordinates.

## Window

- The main window is created hidden and `transparent` (WKWebView can only stop drawing its background at creation). `glass.rs` paints it opaque in the user's background colour, and see-through only for Liquid Glass while focused. Keep `tauri.conf.json` free of `backgroundColor`.
- Keep `dragDropEnabled: false`, or native handling swallows the composer's attachment drops.
- The title bar is an overlay: keep the `data-tauri-drag-region` strips working when changing headers or the sidebar.
- Closing the window hides it; the app keeps running behind the menu bar duck until ⌘Q.
