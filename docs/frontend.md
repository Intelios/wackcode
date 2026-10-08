# The renderer

`src/` is a React 19 app in a WKWebView. It never fetches anything and never holds a credential: everything goes through Rust. Visual rules are in [design.md](design.md).

## State and data flow

- **`App.tsx`** owns all cross-cutting state: `data` (the `AppData` from `bootstrap`) and each chat's runtime. Update it immutably through `patchTask`, `patchRuntime` and `setData`.
- **Events:** `App.tsx` is the only place that listens to Tauri events: `worker-event`, `run-event`, `subscription-login-event` and `native-chat-navigation` (the menu bar). A worker protocol change ends in its `worker-event` switch; see [worker.md](worker.md#protocol-changes). `title_changed` carries a `source` — `"opening"` for the first-run stand-in, `"auto"` for the model's title; only the latter bumps `titlePulses`, which drives the header's swipe+glint and the sidebar row's crossfade.
- **`api.ts`** is the only bridge to Rust: one typed `invoke` wrapper per command. Terminal output arrives on a `Channel`, not an event.
- **Components are presentational.** The exceptions call `api` directly because they wrap one self-contained surface: `SettingsPage.tsx` (and `IntegrationsSection.tsx`, `AboutSection.tsx`), `BrowserPanel.tsx` (positions the native web view), `TerminalPanel.tsx` and `RunPanel.tsx` (stream their PTYs) and `markdown-components.tsx` (its link override routes http(s) clicks through `revealPath`; the webview cannot open links itself). Don't add more without the same reason.

## Context menus

`ContextMenuProvider` owns one pointer-anchored renderer menu, using the shared `Menu` primitive. Components supply existing action callbacks through `useContextMenu`; they never call the bridge for menu actions. Text fields take precedence over enclosing targets; selections and HTTP(S) links add their own actions. The app supplies native clipboard callbacks (reads only after Paste text) and the usual external-link opener. Navigation, scroll, resize and loss of window focus dismiss captured actions. The per-chat native browser preview remains a separate webview with its own page interaction.

## Transcript snapshots

Opening a cold chat emits a saved-history snapshot without starting a worker; opening a live chat keeps the worker authoritative. The worker sends full snapshots and deltas ([worker.md](worker.md#snapshots-and-deltas)); `applySnapshotDelta` in `chat-utils.ts` merges them. A new field that changes mid-run needs updates on both sides. Keep the full-snapshot fallback, and keep unchanged messages as the *same objects*: transcript rows are memoized by identity.

A plain send is echoed while it travels: the composer has already cleared, and the real message can sit behind a worker respawn, a workspace checkpoint and MCP servers connecting. `promptTask`/`sendPrompt` set `runtime.pendingMessage` (`pendingEchoMessage`); the transcript appends it through `withPendingEcho` until a user message of the run arrives or the run settles (`run_state` idle/interrupted, or a rejected prompt, clears it). The hero send hides the echo while its frozen-composer handoff still shows the text. Everything else reads the worker's snapshot only — queued sends, commands and resends deliberately have no echo.

The composer never states the active policy: a persistent read-only badge lived next to the Build/Plan toggle and was removed. What remains is the queue notice: the worker's applied `executionPolicy` wins during a run, and a mismatch with saved Settings shows “Applies next turn”. Effective access is shown in Settings (› Tools and › Sub-agents). Cold history carries no policy: a new worker always receives current Settings, never permissions from session history.

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
state and ten closed records. None of this workspace state is serialized; quitting or a
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
`NavigationScope` dismisses menus, image lightboxes and sent-message editors on navigation,
including when Git mode keeps the transcript mounted underneath. Disclosure sets survive.

The native Tabs menu (`chat_tabs.rs`) owns app-local tab accelerators, including when
a child WKWebView has focus. `api.setChatTabMenu` synchronizes availability, and App is
the only listener for `native-tab-action`. Modal dialogs and Settings/Git disable tab
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
id next, keeping the rest queued. `queue_state.messages` supplies stable `{ id, text }` rows;
the worker retains their full payloads and images. Queue/run events are authoritative: full
snapshots don't clear the queue, and steering never optimistically removes a row, changes run
status or adds a pending echo. Queue actions are single-flight per draft key, and no sends or
queue actions dispatch while stopping. App captures the originating task id for async errors.

## New-chat project picker

`ProjectBar` (the composer's `header` on the welcome screen) hosts `ProjectPicker`, `BranchPicker` and the Local/Worktree toggle. The picker reads and writes the same `pinnedProjects` set as the sidebar and Git mode's `RepoSwitcher`, so a pin made anywhere shows everywhere; it never keeps its own copy. Its display rules (`shortPath`, `monogram`, search) are pure functions in `project-display.ts`. ⌘O stays a global shortcut in `App`; the picker's footer only calls `onAddProject`.

## Attachments

- Images ride `images` through `api.ts` to the worker, and reach only models with Vision on.
- Every attached image opens full size in `ui/ImageLightbox`: the composer shows the picked file it still holds, and the transcript and edit box fetch the original from the chat's session with `api.messageImage` (a snapshot's thumbnail is all they render). In the lightbox, a click zooms toward the clicked point (click again, or −, to zoom out; + zooms to centre) and a zoomed image pans by dragging.
- Attached text files are folded into the message text by `composeFileSection` and read back out for display by `splitFileSection` (`src/attachment-utils.ts`). Change both halves together.
- Attached files never ride a `/`-command message: Pi expands commands and would swallow the section. Sending the same text literally is the deliberate escape hatch.

## Side panel

`SidePanel` shows one `SidePanelView` (`src/side-panel.ts`) at a time: Changes, Browser, Terminal, Run, Games, or one sub-agent's transcript.

- A new view is a union member, a component `SidePanel` renders, and a trigger that opens it (`toggleView`).
- In the original layout, Changes, Terminal, Run and Games are durable: which one is showing is remembered across chats and launches (`wackcode:sidePanel`). With chat tabs enabled, every tab keeps its own panel for the session and a new draft starts closed. Run follows the selected checkout and restoring it never launches a command. Browser and sub-agent views belong to one chat; the original layout falls back to the remembered durable view when the chat changes.
- Sub-agent transcripts reach the panel only through `watch_subagent` frames, never through snapshots.
- **Games** (`GamesPanel`, one entry per game in its `GAMES` list): each game is a pure simulation under `src/games/` plus a component that owns its canvas and rAF loop. The panel's page remounts on every view or tab switch, so a run in progress lives in `games/session.ts` and comes back paused. Quack Survivors also autosaves through `games/save.ts` (a versioned `localStorage` envelope, normalised on the way back out), so a run survives quitting the app; pausing offers **Save & quit**, which hands you back to the chat. Best scores are a `localStorage` convenience (`games/scores.ts`, with a one-time fallback to a game's pre-rename key). The panel takes only the open chat's `status`, so streaming never re-renders it.

## Areas

The app has two peer areas, Code and Chat (`src/areas.ts`). `App` holds which one is on screen in `area`, and `enterArea` is the only way it changes.

- **A chat belongs to one area for life** (`taskArea`, from `TaskRecord.kind`). Every list is built from one area's chats (`areaTasks`, `codeTasks`): the sidebar, Archived, Git mode's linked chats, the tab bar. Opening a chat from anywhere else (the tray menu) enters its area first, in `selectTask`.
- **Switching parks, never abandons.** With tabs, both areas' tabs live in one store, each tagged with its `area`; `switchTabArea` parks the open tab and restores the other area's. Without tabs, `parkedViews` holds each area's chat or unsent draft. One store rather than two because a send finishing or a chat binding must find its tab after the user has switched area.
- **A draft's kind is the area on screen when the send starts**, captured before anything is awaited. It is not a `ChatDraft` field: draft edits rebuild that object field by field and would drop it.
- **The Chat area leaves things out by not passing them:** the composer gets no `header`, `onModeChange` or `onRequestMentions`, and only `appCommandsFor("chat")`. `ChatHeader`, `chatMenu` and `ArchivedList` read `task.kind`.
- **Its side panel is the browser only.** `panelView` is null for anything else there, and the remembered durable view stays Code's: the persistence effect skips Chat.
- **Never call Git for a Chat mode chat** (`refreshChanges` returns early) and never take its snapshot's tool list as the catalogue; see the host's matching rules in [architecture.md](architecture.md).
- **The switch** (`AreaSwitch`) lives inside `.sidebar-top`. Don't add a child to `.sidebar`: the Settings rebuild animation addresses its children by position.
- **⌥⌘1 / ⌥⌘2** are matched on `event.code`, because ⌥ changes `event.key`. The native Go menu sends the same commands (`area-code`, `area-chat`) through `native-tab-action`, so they work while a browser page has focus.

## Git mode

A full-window Git client over the chat view, for one project's own folder (worktree chats keep their changes in the Changes panel). `useGitMode` (`src/hooks/`) holds its state per project and is called by `App`, so it may call `api`; `src/git-mode.ts` holds the pure rules (which project it opens on, the linked chat, the sync button's next action, the remote's web page, the checkbox arithmetic). The `Git*` components and `RepoSwitcher` are presentational.

- **Entering** (the sidebar's Git tile, ⌘⇧G, the Changes panel's button) swaps the sidebar to its `git` page and mounts `.git-view` over the workspace. **Leaving** is the toolbar ✕, ⌘⇧G, ⌘N, ⌘⇧C / ⌘⇧T, or any chat navigation (`dismissGitMode` in `menu-navigation.ts`). Settings opens over it and returns to it.
- **Nothing underneath unmounts.** The chat view stays mounted with `inert` (scroll position, no transcript remount), and so does the composer, whose drafts live in `App`: `.composer-layer` takes `inert` and `data-git`, and CSS ducks it away.
- **The side panel** closes through `view={null}`, keeping the remembered view. A native browser page would still float over Git mode while the drawer animates shut, because the exiting element keeps its old props; `NativeOverlaysHidden` (a context in `BrowserPanel.tsx`) is what hides it in time. Use it for any future full-window view.
- **Network is user-driven:** one background fetch when Git mode opens or switches project, and whatever the user clicks. Refreshes on window focus, on `tool_execution_end` and at the end of a run read local state only (debounced and single-flight), and the chat's own Changes refresh is skipped while Git mode is open.
- **The linked chat** runs the AI actions (comments, Review, Generate, Ask): the chat the user came from if it works in the project's folder, else the project's most recent, else one created on first use. `promptTask` sends to it without selecting it, and Git mode stays open. Comments are stored under the linked chat's id, so its Changes panel shows the same ones.
- **Git writes wait for running chats** (`idle_checkout` in Rust). The UI says why through `GitActivity` and each disabled control's tooltip.
- **"Open in editor" and "Open in GitHub"** open things outside the app. `list_editors` finds the installed editors by bundle name in `/Applications` and `~/Applications`; the pick lives in `localStorage` (`GIT_EDITOR_KEY`), and `open_in_editor` runs `open -a` on the project folder. The clean state's web card takes `repoUrl` — `repoWebUrl` (`git-mode.ts`) over the fetch remote's URL, loaded once per visit in `enter` — and opens it through `revealPath`, like a PR link.
- **Performance:** `App` re-renders on every streaming delta. The Git components are `memo`'d and take callbacks from the ref-backed `gitHandlers` object, so only changed data re-renders them.

## Terminal

The user's own shell, in `src-tauri/src/terminal.rs`. The agent never sees it.

- One PTY per chat, keyed by task id, spawned lazily in the chat's `workspace_path` with the login-shell environment.
- It is not a worker: it never joins the worker fingerprint and the idle reaper never kills it. It dies only via `kill_for_task` (delete, archive, worktree conversion), when the user ends it, or via `terminate_all` on app exit.
- Output streams over a `Channel<TerminalFrame>` straight into xterm, never through React state. A ~256 KiB scrollback ring buffer replays on reattach; one lock orders replay and live output so nothing is lost or duplicated.

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

A native child `WKWebView` per chat (`browser.rs`), placed over `.browser-surface` by `BrowserPanel` through `browserPresent`. Hidden views stay attached, so a background chat's agent can keep using its page. Pages get an ephemeral data store and no Tauri capabilities.

Native window actions use `get_window("main")`: after adding a preview, Tauri's
single-webview `get_webview_window` wrapper no longer resolves the main window.

The native page stays parked while the surface's full rectangle settles. Observe both the
surface and the outer drawer: opening animates the drawer's width while its content keeps its
final size, so a surface-only resize observer misses the position change.

## Scroll rail

`.conversation-scroll`'s native scrollbar stays hidden; `ScrollRail` replaces it with a turn timeline. Two things are load-bearing: `.conversation-scroll`'s `position: relative`, and the `data-turn` markers on `.msg.user`. Tick offsets come from `offsetTop`, which must resolve in scroll-content coordinates.

## Window

- The main window is created hidden and `transparent` (WKWebView can only stop drawing its background at creation). `glass.rs` paints it opaque in the user's background colour, and see-through only for Liquid Glass while focused. Keep `tauri.conf.json` free of `backgroundColor`.
- Keep `dragDropEnabled: false`, or native handling swallows the composer's attachment drops.
- The title bar is an overlay: keep the `data-tauri-drag-region` strips working when changing headers or the sidebar.
- Closing the window hides it; the app keeps running behind the menu bar duck until ⌘Q.
