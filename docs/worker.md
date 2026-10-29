# The chat worker

`worker/src/index.ts` is a Node process wrapping one Pi session per chat. The host talks to it in NDJSON over stdin/stdout (`protocol.ts`, `framing.ts`). After editing anything under `worker/src/`, run `pnpm build:worker` (the dev app runs `worker/dist/`) and start a new chat to see the change.

## Protocol changes

A new or changed command or event touches all of these together:

1. `worker/src/protocol.ts` (`WorkerCommand` / events).
2. Rust payloads and handlers in `src-tauri/src/worker.rs` and `commands.rs`.
3. `WorkerEvent` in `src/types.ts`.
4. The event switch in `App.tsx` (the `worker-event` listener).

Then run `pnpm test:worker`. Commands whose outcome the caller awaits go through `worker::request` and are listed in `REQUEST_COMMANDS`; the rest are fire-and-forget, and the worker's events tell the UI what happened.

## The command queue

Commands run one at a time on a serial queue, and a prompt holds the queue until its run settles. So anything that has to reach a *running* run bypasses the queue, or it deadlocks behind that run. The bypass list lives in the stdin dispatch at the bottom of `index.ts`:

- `abort`, `extension_ui_response`, `browser_response`, `computer_response`, `web_response`, `chat_response`, `canvas_response`: answers the running run is waiting on.
- `queue_message`, `dequeue`: the user's steering and follow-up messages.
- `goal_control` except `set`: pause, resume and clear act on a live loop. `set` starts a run, so it queues.
- `watch_subagent`, `tool_image`, `message_image`, `visual_document`: read-only peeks; panels and saved visuals can open while a run holds the queue.
- `set_subagents` when disabling: cancels children even if the parent is blocked waiting for them.

Before the first `init` settles there is no session to act on, so a bypass command is held on `initSettled` rather than bounced with "Worker is not initialized": the host sends `init` and the first prompt back to back, and a stop or a follow-up sent in that window must survive it. The gate opens one macrotask later — after the microtasks queued behind `init` have run — so a held stop finds the just-started first run live and stops it through the ordinary path, and a held follow-up finds a streaming session to queue onto.

Settings pushed with `set_*` (tools, prompts, execution policy, sub-agents, MCP, skills, commands, memory, computer use) deliberately **queue**, so a run never sees its tools, prompt or skills change under it. They apply from the next message; nothing respawns.

## Model-less chats

Saved history opens without a model or worker. When a worker starts for a chat whose configured model left its connection (removed in Settings, or dropped by a subscription), `init` restores the session on a stand-in model (`missingModelPlaceholder` in `model-runtime.ts`), the snapshot carries `modelMissing`, and the transcript, tree and checkpoints stay readable. Everything that needs the model — prompts, `/compact`, commit-message generation, the goal verifier — is refused with "This chat's model is no longer configured. Pick another to continue." Picking a model stops the old worker through `configure_task` (the model is in the fingerprint); the next worker records the durable switch when it starts.

## Custom connection request shaping

Settings' refreshable Models.dev catalogue is reference metadata owned by Rust, not part of a
worker. `catalog-models.ts` retains Pi's pinned, offline request-compatibility lookup; downloaded
catalogue data cannot supply `compat`, headers or credentials. Subscription lists still use Pi.

A custom connection's `models.json` re-declares its models, so Pi's request shaping
(`forceAdaptiveThinking`, `supportsStore`, `maxTokensField`…) is lost unless `builtinCompat`
(`catalog-models.ts`) re-derives it: Google's strict OpenAI-compatible hosts get a fixed shape,
otherwise the bundled catalogue's entry is copied over. Two stream-level instrumentations wrap
the runtime's `stream`/`streamSimple`/`streamDeferred` (`model-runtime.ts`):
`instrumentOpenCodeHeaders` adds OpenCode's session headers, and
`instrumentGoogleThoughtSignatures` (`thought-signatures.ts`) round-trips the `extra_content`
vendor extension Google's thinking models put on each `tool_calls` delta — required back
verbatim or the next request 400s. It rides Pi's own hooks (`onProviderStreamEvent` in,
`onPayload` out, both chained to the SDK's) and persists the signature in the tool call's
`thoughtSignature` field — which the session file keeps — as `{"extra_content": …}`, a marker
only this instrumentation writes. Don't repurpose that field or strip it on save.

## Saved history

`session-reader.ts` opens saved history without a chat worker. It shares the live message
normalizer and transcript annotations. Display history follows the whole current branch,
including compaction boundaries; Pi's compacted projection still governs model context and
context statistics. Both views honor context edits and omissions. It receives only a host-owned session path and chat metadata, runs with a
cleared environment (only PATH and offline flags remain), never loads extensions or credentials,
and never rewrites the file (legacy migrations happen in memory). Cold snapshots use revision 0; a new worker's
full ready snapshot establishes its own delta chain. Ready workers remain authoritative; a
reader that finishes after a worker becomes ready discards its cold snapshot.

## Snapshots and deltas

The transcript reaches the host as a full `snapshot`, then as `snapshot_delta`s at each message boundary, with `partial` frames (~60 fps) for the streaming message.

- Normalized messages are cached by Pi's own message objects. An unchanged message reuses the *same* normalized object, which keeps the diff O(changes) and lets the renderer memoize transcript rows by identity. Never mutate a cached object.
- A delta is an optimization, never a requirement: when a change can't be expressed as "remove these ids, then replace or append these messages", `diffMessages` (`delta.ts`) returns undefined and a full snapshot goes out. Compaction forces a full snapshot. Keep that fallback.
- A new field that changes mid-run needs `SnapshotDelta` (worker) and `applySnapshotDelta` (`src/chat-utils.ts`) updates.
- Snapshots carry small image previews, never originals; the lightbox fetches the original with `tool_image`, or `message_image` for one the user attached to a message. Every tool result's images get previews on its result block (`images`, in the result's own order, which is what `tool_image` indexes by), whichever tool returned them; one that can't be previewed is marked `failed` rather than left pending.
- Compaction rows use Pi's saved entry ids and summaries. Their after-count estimates Pi's
  projection at that entry, so later messages cannot change an old boundary's count. Tool-result
  images and saved sub-agent transcripts remain fetchable throughout the displayed branch.
- `compaction` is live snapshot/delta state, cleared on every compaction end. It carries the
  `startedAt` the stage's timer mounts from (a manual compaction's equals the `run_state`
  running frame's). Manual compaction's `run_state` has `operation: "compaction"`; it never
  starts a prompt clock or records a turn duration.

## Tools

- Never pass `tools:` to the chat's `createAgentSession`: Pi treats it as a registry filter and erases extension tools. Apply the user's denylist with `setActiveToolsByName`, and re-apply it after anything that can re-activate tools (extension `session_start`, `navigateTree`, a roster change).
- The Settings › Tools denylist covers Pi's tools and package tools. Built-in extension tools are exempt (a disabled `plan_mode_complete` would silently break Plan mode), except the ones in `SWITCHABLE_BUILTIN_TOOLS` (browser, `web_fetch`, the paired `show_visual` / `preview_visual`, `suggest_task`, and the three canvas tools, which switch together).
- Sub-agents, memory, computer use and web search have their own settings and keep their tools out through `inactiveTools()`; they never appear on the denylist. `web_search` is offered while `set_web` (or `init`'s `web`) names an active provider, and reaches sub-agents through its own clause in `childToolNames`.
- MCP tools (`mcp__<server>__<tool>`) have their own per-server and per-tool switches and stay out while their server is off or unreachable. Keep the built-in and MCP switch rules separate.
- `powershell` is removed from the registry; `grep`/`find` are reported unavailable when ripgrep/fd are missing (the worker is offline and can't download them).

### Shell check-ins

`builtin/bash-jobs.ts` wraps Pi's shell tool as an SDK override (`customTools`), preserving the
ordinary bash denylist and Plan-mode policy. SDK overrides win over extension tools of the same
name. Pi still owns process spawning, the shell environment, UTF-8 output, truncation/spill files
and process-group cancellation.

A bash call waits at most 60 seconds (`yieldTimeout` may shorten it, never lengthen it). If still
running it returns output, an opaque session-local job id and progress metadata; `bash_job`
inspects, waits for up to another 60 seconds, or stops that same job. `timeout` remains an optional
hard execution deadline. Output does not reset the check-in clock. Raw output byte counts and
time since the last output distinguish active and silent jobs, and updates only reach an active
wait, never a tool that has yielded. There are at most eight live jobs and 32 retained results.

Jobs belong to one agent run. Their original abort signal remains linked after yielding; Stop
and shutdown explicitly stop all jobs too. The first built-in's `agent_settled` handler cleans
up unfinished jobs before the host reports idle or the goal loop starts another round. Job ids
are ephemeral, not restored from history. Each child has a private manager and automatically
gets `bash_job` only when its filtered tool list includes bash; this companion cannot execute
shell text or target an arbitrary process. Read-only guards still check the original command.
The user's Terminal and project Run processes are separate and unchanged.

### Images

A gateway in front of a provider rejects an oversized request body with HTTP 413 for every later
turn, whatever the model, so a chat's images are bounded twice:

- **Each image**, when it enters a session. Every custom model's `models.json` declares
  `inputLimits.images.resize` from `image-limits.ts`, and Pi applies it to prompt images and to
  every tool result's images (`read` of a picture, browser and computer screenshots, MCP). Pi
  adds a dimension note whenever it resizes, so screenshot coordinates stay mappable. The same
  limits run over the composer's attachments in `prepareImages`.
- **The request**, on every model call. `builtin/image-budget.ts` runs on the `context` hook with
  a total byte budget for a chat's images. Older tool images become a placeholder first, then
  older images the user attached; the newest image always survives, so the tool that just
  finished still reaches the model. Removals happen in whole chunks so the rewritten prefix
  stays byte-identical between calls and the prompt cache keeps hitting.

The budget never changes the session: it rewrites only the copy Pi sends, so the transcript, the
lightbox and `tool_image` keep every original. Sub-agents load it too.

Before the budget, parent chats also run `builtin/image-references.ts`. It matches each
request's user and tool-result images to one saved source in Pi's session projection, by role,
timestamp and image contents (plus tool-call id for results), then adds complete
`wackcode-image://entry/<entryId>/<imageIndex>` references to the request copy. Unmatched or
ambiguous images get no reference. The agent can select these in Markdown replies in either
area; pruning pixels leaves the references usable. Nothing is added to saved source messages,
and sub-agents do not load this extension. `read_chat` reduces reply images to descriptive
markers rather than passing another chat's references along.

## Built-in extensions (`worker/src/builtin/`)

WackCode's own extensions load as inline factories, so they bypass the package trust gate by construction. They are: `ask-user-question`, `auto-title`, `bash-jobs`, `browser`, `canvas`, `chat-refs`, `computer-use`, `goal`, `image-budget`, `image-references`, `mcp`, `memory`, `plan-mode`, `skill-creator`, `subagents`, `suggest-task`, `todo`, `visual`, `web-fetch`, `web-search`, and `chat-mode`, a guard with no tools of its own that only Chat mode chats load.

- Adding or renaming one also updates `BUILTIN_EXTENSIONS` in `src/components/PackagesSection.tsx`, which shows them so nobody installs a duplicate package.
- Sub-agents never get browser, computer-use, MCP, memory, visual, canvas or suggestion tools, or skills. A child session loads its role's tool allowlist, its own shell check-in extension and the extensions the built-in hands it.
- Foreground sub-agent transcripts are saved on the `subagent` result's `details` but kept out of snapshots (`withoutTranscripts` in `normalizeMessage`) and live card updates (`snapshotDetails`). The side panel gets them only through `watch_subagent` frames.

### Visual replies (`visual.ts`)

`show_visual` lets the agent answer part of a turn with a small HTML document — a chart, a
comparison, a diagram, a mockup of proposed UI, a tiny widget — that the desktop draws where the
call landed (`src/components/VisualCard.tsx`). It takes a `title` (60 characters) and an `html`
body (128 KiB). It measures at 360, 560 and 720 px in one offscreen render request, returning
`{ v: 1, title, heights? }` with width/height pairs; a rendering failure still publishes without
measurements. Markup stays in the saved call's arguments. CSS `vh` units are refused in styles
and style attributes; the guide asks for content-sized pages, transparent roots and fixed-height
charts, with no outer card, side padding or title banner.

`preview_visual` takes `html`, an optional width (240–1600, default 720), and an optional
`appearance` (`light` or `dark`). It uses that same renderer with reply styling, returning the
content height and script errors plus a PNG only for models with vision. The guide asks the
agent to preview first, fix what it sees, and publish without announcing or restating the page.

Both tools inline absolute local image paths after checking file signatures and, in Chat mode,
scratchpad confinement (4 MiB per file, 8 MiB per finished page). When images were inlined,
`show_visual` saves `details.inlined = true` and `details.html`; the shared message normalizer
removes the HTML from live and cold snapshots. `visual_document` fetches the saved document by
tool-call id from the current branch, bypassing the command queue, so images survive the source
file moving or disappearing. These images reach the frame as data URLs.

- Both tools are read-only, so Plan and Ultra Plan let them through; Chat mode allows them
  (`CHAT_BUILTIN_TOOLS`), where its guide carries the same words as the tool's
  `promptGuidelines` (Pi drops those once a chat prompt is set). Sub-agents never get it: a child
  has no transcript to draw in.
- They switch together (`SWITCHABLE_BUILTIN_TOOLS`), so Settings › Tools can take them away; the
  Packages built-in card carries that switch. Saved visuals still render — the switch is a
  capability, not a display preference.
- The document runs in a sandboxed frame with no network at all, so the guidelines say to inline
  every asset and to colour from the app's CSS variables (including semantic colours, six chart
  colours and shadcn aliases). `wackcode.send(text)` posts the message
  the renderer gates (see [security.md](security.md#visual-replies)).
- `read_chat` collapses the call to `[visual: Title]`, and a message a visual sent to
  `[sent from visual: Title]`.

### Canvas (`canvas/`)

A canvas is a board of artboards — full HTML mockups at real sizes — that the user pans and zooms
in the side panel's Canvas tab, for comparing UI alternatives (themes, layouts, desktop beside phone). Three
tools, all read-only, so Plan and Ultra Plan allow them (the plan-mode helper list):

- **`canvas_guide`** is the authoring guide (`guide.ts`): basics, exploring directions, theme
  variants, fidelity to the project, craft, checking work, an example. It is served on demand so
  the long rules never ride in the system prompt; the tools' own `promptGuidelines` only say when
  to reach for the canvas and to read the guide first. `/canvas` puts the whole guide in its prompt.
- **`canvas`** starts a canvas (`title`) or changes one (`canvas` id): up to 6 artboards per call
  added or revised (an existing `id` changes only the fields given), and `remove`. An artboard has
  a `name`, a `group` (its row), a `width`, an optional `height` (absent: fit the content) and
  either `html` or `basedOn` another artboard plus `css` appended after it, so theme variants cost
  a few hundred bytes and follow their base; a variant that gives no width takes its base's size
  (recorded in `applied`, since the arguments lack it). Limits are in `state.ts` (64 KiB per artboard,
  192 KiB per call, 24 per canvas; a base is never a variant and can't be removed under one).
- **`canvas_look`** asks the host to render artboards offscreen, 4 at a time (more are named for
  a next call rather than refused, since Pi rejects a call that breaks a schema's `maxItems`) (`canvas_request`, answered
  by `canvas_response`, which bypasses the queue like the browser channel) and returns a
  screenshot of each when the model has vision, a report (rendered size, overflow, script
  errors), and optionally the resolved markup — what an agent needs to implement a chosen design
  after compaction.

**The branch is the record.** The markup stays in the call's arguments, which Pi saves; the
result's `details` (`{ v: 1, canvas, title, applied: [{ index, id, op, basedOn?, width?, height? }], removed }`)
say which artboard each argument became, with no HTML. `canvasesFromBranch` folds the accepted
calls, so rewinds, forks and restarts need nothing else, and the renderer folds the snapshot the
same way (`src/canvas-state.ts`). Both suites run the same `fold-cases.json`; keep the two folds
identical.

- Code chats only: never in `chatFactories`, and `start_canvas` and `/canvas` are refused in Chat
  mode. Sub-agents never get the tools (`childToolNames`): a child has no canvas.
- **An exploration changes no project file**: the guide and `/canvas`'s prompt say so, because a
  live run once "fixed" a mockup by editing the project's own tokens.
- `/canvas [request]` is an app command (`start_canvas`), like `/skill-creator` but allowed in
  every mode. It is refused while switched off in Settings › Commands or while the tools are off.
- `read_chat` collapses a call to `[canvas: Title — names]`. Nothing joins the fingerprint.

### Task suggestions (`suggest-task.ts`)

`suggest_task` lets the agent file a follow-up it noticed while working — a bug, a rough edge, a
missing bit of polish — without doing it and without burying it in its final message. It takes a
`title` (80 characters), a `kind` (`bug`, `polish` or `improvement`) and a self-contained
`description` (2,000 characters), and returns `{ v: 1, kind, title }`: the brief stays in the
call's arguments, which Pi saves, so cold history needs nothing extra. The desktop draws one
card per accepted suggestion under the turn's answer (`src/components/SuggestionTray.tsx`).

- **It only records.** It is read-only, so Plan and Ultra Plan allow it (the plan-mode helper
  list): noticing a bug while planning is exactly when it helps. It writes nothing to disk and
  changes no file.
- **Guards, because a model that can file tasks files too many.** At most
  `MAX_SUGGESTIONS_PER_RUN` (5) per run, reset when the run settles (`agent_end`), and a title
  already suggested on the session branch is refused. The branch is the record: accepted calls
  are the tool results carrying versioned `details`, so the check follows a rewind and survives
  a restart. The comparison ignores case and spacing.
- **Scope.** Code chats only: it is never in `chatFactories` and never on the Chat mode
  allowlist. Sub-agents never get it (`childToolNames`): a child has no transcript to draw a
  card in, so it reports what it noticed and the parent decides. It is switchable
  (`SWITCHABLE_BUILTIN_TOOLS`), so Settings › Tools can take it away; saved cards still render.
- **Starting one is the renderer's job.** "Start here" and "Start in new chat" send the title
  and brief as a literal prompt with a generated `<suggested-task-v1>` tail (`src/suggestions.ts`),
  never through the tool. A new chat records `TaskRecord.originSuggestion`, which is what the
  origin's card turns into a link. Dismissal lives in `TaskRecord.dismissedSuggestions`; neither
  is ever part of a run's input.
- `read_chat` collapses the call to `[suggested task: Title]` and a started one's tail to
  `[started suggested task: Title]`.

### Background sub-agents

`subagent` defaults to blocking; `background: true` returns child job IDs immediately.
`subagent_job` inspects, waits for, or stops selected IDs (omitted IDs capture all outstanding
children at invocation time). One chat-wide scheduler admits at most eight unfinished children,
with the configured concurrency shared across all launches. Children capture their role, tools,
policy, model and connection before waiting for a slot. Settings affect subsequent launches;
disabling the feature cancels outstanding jobs. Captured credentials remain in redaction until
the final output and transcript have been sanitized.

Background jobs own their signals and never update a launch tool after it returns. The worker
coalesces completed results into hidden `wackcode-subagent-results` custom messages, steering a
streaming parent at a safe tool boundary or starting a continuation on the serial queue after
settlement. A rejected delivery loses nothing: its claim is rolled back (unless a stop owns the
outcomes) and retried with backoff until the message lands. Explicit waits reserve their targets and
consume results without an extra automatic continuation. Stop cancels jobs, shell processes and queued result delivery, including while the
parent is idle. User messages can start alongside children; explicit Steer keeps its cancellation
behaviour. Goal verification, plan submission and skill preview wait for outstanding children
and result delivery.

`workActivity` accompanies run-state, snapshots and deltas: parent activity is separate from
aggregate busy status. The host keeps Git/reaper/history guards busy while children remain.
Background cards and capped transcripts are stored as versioned `wackcode-subagent-background`
custom entries linked to the launch tool ID. Live and cold projections overlay the original
immutable tool result on the current branch. Unfinished cold cards become interrupted; opening
history or restarting a worker never relaunches them. Only watched transcripts stream. Completed
background usage is appended once as Pi usage entries; status/wait calls carry no extra usage,
and the published usage ledger is unchanged.

### Team

`SubagentRuntimeConfig.team` is the per-chat Team flag; Rust sets it when the chat's `TaskRecord.team`
model resolves (crew plus its connection's credential), so it rides the same `set_subagents` push as
everything else. While it's on, the system prompt gets the delegation addendum (`builtin/team.ts`,
overridable in Settings › Prompts), every built-in agent's model is overridden to the crew's, the
`subagent` schema gains `job` (a task form that continues a finished child by jobId, refusing one
that is gone, busy, undelivered or used twice in a call), and `subagent_job` status cards report
turns and recent activity. Finished children stay alive in `KeptSessions` (8 per chat, LRU) for a
later `job` continuation; they are dropped on `configure` when Team is off or the feature is
disabled, and never survive a worker restart. A continuation reuses its first jobId — the launch's
own ID stays valid via an alias — and `SubagentResult.round` counts rounds from two. The host's
`runSubagent` gets `request.continuation` (the retained history) alongside a freshly resolved
role. Each round rebuilds its Pi session from that in-memory history with the current roster,
tool switches, access policy and credentials; a disabled role cannot be continued, and keeping
context never preserves revoked access. Active rounds are pinned against LRU eviction. The
switch itself is `set_task_team` in Rust: refused while the chat runs, and no-op-quiet when the
chat has no live worker.

## Multi-run loops (`/goal`)

A built-in that iterates past one run queues its next turn from its `agent_settled` handler via `pi.sendUserMessage`. The run is already inactive there, so this starts a fresh nested run. The worker suppresses `run_state: idle` while `willContinue()` is true, so the chat never flashes idle between rounds. The verifier is a separate no-tools call on the chat's own model.

## Skill creator (`/skill-creator`)

A guided workflow for creating and improving Agent Skills (Anthropic's skill-creator inspired the loop; the guide and tooling are WackCode's own). The `skill_creator` worker command starts a Build-mode run on the bundled guide prompt (`builtin/skill-creator/guide.ts`); the model then works through one sequential tool, `skill_creator`:

- `prepare` asks the host (`skill_creator_request`, answered by `skill_creator_response`; both bypass the prompt queue like the browser channel) to create a managed draft under `agent/<task>/skill-creator/<draft-id>/`, snapshotting an existing skill into `original/` when improving one. The draft identity persists as a `wackcode-skill-creator` session entry, so the workflow is branch-scoped: a rewind to before it removes it (`session_tree`/`session_compact` re-derive the state), and the snapshot mirrors it in `skillCreator`.
- `preview` has the host validate the draft with Pi's own keyless skill scan, hash it, and write an immutable review snapshot. The result carries versioned `details` (`SkillPreviewDetails`, hard-capped like the sub-agent card's) and `terminate: true`, so the review card is the turn's outcome; the desktop renders it from the tool result like a Plan card, and only the newest preview on the branch is actionable.

The tool exists only while the workflow is active on the branch (or the command's run is live), is Build-only, and is withheld while `/skill-creator` is switched off (its `app:` key in the command denylist drives `inactiveTools`). The agent never writes into `~/.agents/skills`: publication is the card's native Save button (`publish_skill_draft` in Rust), which re-verifies the reviewed revision under the shared skill-library lock (`skills::library_guard`, also held by Settings' save/delete/import/copy), commits with exclusive rename (new skills) or `RENAME_SWAP` (folder updates), and journals a receipt that `skill_draft_status` reconciles after an interrupted save. `set_skills` then carries the new skill to live workers on their next turn.

## Coding prompt

`builtin/code-prompt/prompt.ts` owns the coding core and `buildCodeGuide`. The preamble is the
Settings › Prompts override, then the loader's trusted project/global `SYSTEM.md`, then
`DEFAULT_CODE_PROMPT`. The addendum is the app-owned guide, the loader's `APPEND_SYSTEM.md`
text, then the memory index. Pi still supplies project instructions, skills and the cwd.

Pi omits its tools, rules and documentation sections whenever a custom preamble is set. The
guide re-serves every active tool's `promptSnippet` and deduplicated `promptGuidelines` from
`session.getToolDefinition`, including trusted package tools, so custom personas retain tool
rules. Capability sections describe only enabled tools, and the trust rule is always present.
The guide is not editable; Settings edits replace only the core. Pi's own documentation block
is deliberately omitted. Sub-agent children keep their existing prompts.

The core plus guide with every built-in enabled is bounded at 23,000 characters (roughly
5,750 tokens by characters ÷ 4), before project context, skills, memory or user-supplied tool
metadata. Active tools' rules are preserved in full rather than truncated.

While Settings › Git's commit attribution is on (`init.git`, live via `set_git`; Chat mode is
sent null and ignores `set_git`), the guide asks the agent to end every commit message with
`Co-authored-by: <model> (WackCode) <WACKCODE_COAUTHOR_EMAIL>`. The address is the WackCode
Agent GitHub App's bot noreply email, which GitHub links by email; the app has no key or
permissions and nothing signs in as it. The name is the session model's `coAuthorLabel`, so the
run-start refresh below makes each run's trailer name the model that did the work.
"(WackCode)" names the app, never `appearance.agentName`. Commits made from the Changes panel
and by sub-agents get no trailer.

Like Chat mode's guide, the coding guide is refreshed between runs when its date, active tools,
model or guidance change; it stays stable during a run. Settings remain queued and apply from the next
message. The Git-repository context is checked once at worker initialization, never for Chat mode.

## Coding compaction

`builtin/compaction/` handles `session_before_compact` in coding chats only. Pi still picks the
cut point and the kept tail, and its private default summarizer runs through
`runDefaultCompaction` so routing, credentials, retries and the usage ledger stay Pi's; WackCode
only adds instructions to it (attribute instructions to the user alone, record failed approaches).
Around Pi's summary it adds, in code: a line saying only the quoted user messages are the user's
words, the user's own messages from the compacted span (first request always, then the newest
within a budget), the todo list and the skills in use. `details.body` keeps Pi's part so the next
compaction updates it alone; WackCode also carries the file lists Pi drops for extension
compactions. Any failure falls back to Pi's default, and a package that handles
`session_before_compact` itself takes over. Chat mode keeps Pi's default.

## Chat mode (`init.kind === "chat"`)

A Chat mode chat is the same worker with a different prompt and far fewer tools. Everything that differs is decided in the worker (`chatKind` in `index.ts`), so an app-wide `set_*` broadcast can never widen such a chat. Read `builtin/chat-mode/` before changing any of it.

- **Built-ins:** `chatFactories` instead of `factories`: ask, auto-title, memory, web-fetch, web-search, browser, MCP and the guard. The built-ins it leaves out still have controllers, which report their resting state. They are created at module scope, before `init`, so this is a second list chosen at session creation and not a runtime switch.
- **Registry:** `excludeTools` also drops `bash`, and no shell tool is registered. (`excludeTools` is not the `tools:` allowlist, which stays forbidden.) Package resources are ignored.
- **Active set:** `activeToolNames()` adds the allowlist (`chatToolAllowed`) on top of the usual rules. The user's denylist and MCP switches still narrow it.
- **Enforcement:** the guard's `tool_call` hook blocks anything off the allowlist and any file-tool path outside the scratchpad, fail-closed. The active set alone is not enforcement: a navigation restores the transcript's declared tools.
- **Paths:** `confine` mirrors Pi's own path resolution, which Pi does not export, then compares real paths. It rewrites an allowed `path` to the scratchpad-relative spelling it checked. A Pi upgrade that changes how file tools resolve a path must update it; `policy.test.ts` pins the cases.
- **Prompt:** built in `withAppLayers`, without flipping a loader flag. The chat persona replaces Pi's, which also drops Pi's tool, rules and docs sections; context files, skills and prompt templates are served empty; the app's guide (`buildChatGuide`) is appended ahead of the memory index; Pi still adds the cwd, which is the scratchpad.
- **The guide carries what `promptGuidelines` cannot.** Pi drops every tool's guidelines once a custom prompt is set, so the rule that fetched content and search results are data, not instructions, lives in the guide, as does what web_search does and whether the chat can search at all. It is rebuilt at run start when the date or the active tools moved.
- **Commands:** the catalogue is empty; `execute_command`, `init_agents`, `skill_creator`, `goal_control` and `set_mode` are refused, and the coding-only `set_*` broadcasts are acknowledged and dropped. A typed `/goal` reaches the model as plain text.
- **Chat references:** `read_chat` is on its allowlist, so an `@` mention of another Chat mode chat can be read. That chat's own session is rendered by `chat-history.ts`; the host refuses a Code chat (`chat_refs.rs`), which is why the picker never offers one.

## `@` chat references (`read_chat`)

An `@` mention of a chat puts only that chat's **id** in the prompt: a generated
`<referenced-chats-v1>` section after the user's words (`src/chat-refs.ts` in the renderer).
Attaching a long chat therefore costs a sentence, and `builtin/chat-refs.ts` (`read_chat`) is
how the agent actually reads it.

- **Resolution is the host's.** The tool sends `chat_request` → `chat_refs.rs`, which decides
  who may read whom (never itself; a Chat mode chat only other Chat mode chats; the chat must
  exist and have a session file on disk) and answers with metadata plus that session file. The
  model never supplies a path.
- **Rendering is the worker's** (`chat-history.ts`): the current branch, root first, turns as
  `## User` / `## Assistant`, thinking dropped, images as `[image]`, tool calls as one-line
  markers, tool results only with `tools: true` and truncated. WackCode's own tail sections
  inside an old message collapse to markers, so one chat's attachments cannot swamp another's
  read. Long chats page by `offset` like `web_fetch`; the rendering is cached against the
  session file's size and mtime.
- It is read-only (allowed in Plan and Ultra Plan), never reaches sub-agents, and is not on the
  Settings › Tools denylist: a switched-off `read_chat` would make an `@` reference silently
  inert.

## Planning modes

- Modes are `build`, `plan` and `ultraplan`. Plan and Ultra Plan both count as planning: test `mode !== "build"` (`isPlanMode` in the frontend), never `mode === "plan"`.
- The Plan contract must stay **byte-identical**: saved chats recognise it by exact text, and an edit makes every existing Plan chat re-append it. The test pins its hash; don't just update the hash.
- Custom prompts (Settings › Prompts) layer on top of the built-ins. Keep the Settings defaults in `src/promptDefaults.ts` aligned with the worker's prompt builders.

- `executionPolicy` arrives in `init` and queued `set_execution_policy`, outside the fingerprint. Both overrides default off. Snapshots and deltas report the applied policy so a saved change never mislabels an active run.
- Removing planning restrictions bypasses only the read-only tool policy; completion, interview, approval and Build-only workflow gates remain. `access.ts` adds transient guidance after contract reconciliation, never a session entry or a change to the canonical prompt.
- Resolve a child from its saved role and the policy once per call. Normally read-only roles gain edit/write and unrestricted bash only with the child override and without a read-only planning parent. Editing roles remain refused by read-only planning. Disabled tools and the child-only tool inventory still apply. Result `readOnly` records that call's effective policy; saved roles and custom instructions stay intact.

## Ultra

- Ultra (`builtin/ultra.ts`) is the reasoning step above a model's own top level. The host stores it as the chat's `thinkingLevel` (`"ultra"`), but it is never a model capability and **never reaches Pi**: `piThinkingLevel` hands Pi `max`, which Pi clamps to the highest level the model record lists. `THINKING_LEVELS` (protocol.ts, commands.rs) stays Pi's list; Ultra is offered on any model with a level other than `off`.
- While on, the system prompt's addendum ends with `ultraPrompt()` (Settings › Prompts › Ultra, or `DEFAULT_ULTRA_PROMPT`), in coding and Chat mode chats alike. A level change respawns the worker through `configure_task`, so the flag is fixed for a worker's life; `set_prompts` edits apply from the next message.
- Sub-agents on the chat's model inherit `session.thinkingLevel`, the real clamped level, so they run at the top level without the instruction. A sub-agent's own model never takes Ultra.

## Memory

- The note format (`<type>_<slug>.md` with `type`/`title`/`description`/`modified` frontmatter) is one contract shared by `src-tauri/src/memory.rs` and `worker/src/builtin/memory/store.ts`. Change both together.
- The system prompt gets only the generated index (`memoryIndex`), refreshed before each run and never mid-run, so the prompt cache survives.
- Memory tools are gated by their own setting via `inactiveTools()` and stay out of sub-agents.

## Slash commands and skills

- A command's key (`commandKey` in `worker/src/slash.ts`) must stay identical across the Settings denylist, `commands-scan.ts` and the worker's catalog.
- WackCode owns eight names (`APP_COMMAND_NAMES`: `/init`, `/compact`, `/new`, `/name`, `/copy`, `/goal`, `/skill-creator`, `/canvas`). The first entry to claim any other name wins it; a later clash is renamed `<source>:<name>` (`resolveCommandNames`), and a switched-off command frees its name.
- Skill and command folders are re-read before every run, so edits on disk apply from the next message. Where they may load from is a security rule; see [security.md](security.md#pi-lockdown).

## Errors and redaction

Errors the worker reports pass `safeError`, which strips the credentials this worker holds (the chat's, and those of its sub-agents' connections and MCP servers). Rust redacts and bounds them again with `redact_and_limit`. Keep both layers.

`describeProviderError` (`provider-error.ts`) rewrites the one raw failure a user cannot act on —
a gateway's HTTP 413, which arrives as an nginx HTML page inside a JSON wrapper — into a sentence
that says what to do. It runs in `safeError` and in the message normalizer, so a live chat and its
saved history show the same sentence.
