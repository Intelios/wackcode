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

- `abort`, `extension_ui_response`, `browser_response`, `computer_response`: answers the running run is waiting on.
- `queue_message`, `dequeue`: the user's steering and follow-up messages.
- `goal_control` except `set`: pause, resume and clear act on a live loop. `set` starts a run, so it queues.
- `watch_subagent`, `tool_image`: read-only peeks; the panel opens on a child while the call running it holds the queue.

Before the first `init` settles there is no session to act on, so a bypass command is held on `initSettled` rather than bounced with "Worker is not initialized": the host sends `init` and the first prompt back to back, and a stop or a follow-up sent in that window must survive it. The gate opens one macrotask later — after the microtasks queued behind `init` have run — so a held stop finds the just-started first run live and stops it through the ordinary path, and a held follow-up finds a streaming session to queue onto.

Settings pushed with `set_*` (tools, prompts, sub-agents, MCP, skills, commands, memory, computer use) deliberately **queue**, so a run never sees its tools, prompt or skills change under it. They apply from the next message; nothing respawns.

## Model-less chats

Saved history opens without a model or worker. When a worker starts for a chat whose configured model left its connection (removed in Settings, or dropped by a subscription), `init` restores the session on a stand-in model (`missingModelPlaceholder` in `model-runtime.ts`), the snapshot carries `modelMissing`, and the transcript, tree and checkpoints stay readable. Everything that needs the model — prompts, `/compact`, commit-message generation, the goal verifier — is refused with "This chat's model is no longer configured. Pick another to continue." Picking a model stops the old worker through `configure_task` (the model is in the fingerprint); the next worker records the durable switch when it starts.

## Saved history

`session-reader.ts` opens saved history without a chat worker. It shares the live message
normalizer and transcript annotations, and uses the pinned Pi session projection for compaction
and branch context. It receives only a host-owned session path and chat metadata, runs with a
cleared environment (only PATH and offline flags remain), never loads extensions or credentials,
and never rewrites the file (legacy migrations happen in memory). Cold snapshots use revision 0; a new worker's
full ready snapshot establishes its own delta chain. Ready workers remain authoritative; a
reader that finishes after a worker becomes ready discards its cold snapshot.

## Snapshots and deltas

The transcript reaches the host as a full `snapshot`, then as `snapshot_delta`s at each message boundary, with `partial` frames (~60 fps) for the streaming message.

- Normalized messages are cached by Pi's own message objects. An unchanged message reuses the *same* normalized object, which keeps the diff O(changes) and lets the renderer memoize transcript rows by identity. Never mutate a cached object.
- A delta is an optimization, never a requirement: when a change can't be expressed as "remove these ids, then replace or append these messages", `diffMessages` (`delta.ts`) returns undefined and a full snapshot goes out. Compaction forces a full snapshot. Keep that fallback.
- A new field that changes mid-run needs `SnapshotDelta` (worker) and `applySnapshotDelta` (`src/chat-utils.ts`) updates.
- Snapshots carry small image previews, never originals; the lightbox fetches the original with `tool_image`, or `message_image` for one the user attached to a message.

## Tools

- Never pass `tools:` to the chat's `createAgentSession`: Pi treats it as a registry filter and erases extension tools. Apply the user's denylist with `setActiveToolsByName`, and re-apply it after anything that can re-activate tools (extension `session_start`, `navigateTree`, a roster change).
- The Settings › Tools denylist covers Pi's tools and package tools. Built-in extension tools are exempt (a disabled `plan_mode_complete` would silently break Plan mode), except the ones in `SWITCHABLE_BUILTIN_TOOLS` (browser, `web_fetch`).
- Sub-agents, memory and computer use have their own settings and keep their tools out through `inactiveTools()`; they never appear on the denylist.
- MCP tools (`mcp__<server>__<tool>`) have their own per-server and per-tool switches and stay out while their server is off or unreachable. Keep the built-in and MCP switch rules separate.
- `powershell` is removed from the registry; `grep`/`find` are reported unavailable when ripgrep/fd are missing (the worker is offline and can't download them).

## Built-in extensions (`worker/src/builtin/`)

WackCode's own extensions load as inline factories, so they bypass the package trust gate by construction. They are: `ask-user-question`, `auto-title`, `browser`, `computer-use`, `goal`, `mcp`, `memory`, `plan-mode`, `subagents`, `todo`, `web-fetch`.

- Adding or renaming one also updates `BUILTIN_EXTENSIONS` in `src/components/PackagesSection.tsx`, which shows them so nobody installs a duplicate package.
- Sub-agents never get browser, computer-use, MCP or memory tools, or skills. A child session loads nothing but its role's tool allowlist and the extensions the built-in hands it.
- Sub-agent transcripts are saved on the `subagent` result's `details` but kept out of snapshots (`withoutTranscripts` in `normalizeMessage`) and live card updates (`snapshotDetails`). The side panel gets them only through `watch_subagent` frames.

## Multi-run loops (`/goal`)

A built-in that iterates past one run queues its next turn from its `agent_settled` handler via `pi.sendUserMessage`. The run is already inactive there, so this starts a fresh nested run. The worker suppresses `run_state: idle` while `willContinue()` is true, so the chat never flashes idle between rounds. The verifier is a separate no-tools call on the chat's own model.

## Planning modes

- Modes are `build`, `plan` and `ultraplan`. Plan and Ultra Plan both count as planning: test `mode !== "build"` (`isPlanMode` in the frontend), never `mode === "plan"`.
- The Plan contract must stay **byte-identical**: saved chats recognise it by exact text, and an edit makes every existing Plan chat re-append it. The test pins its hash; don't just update the hash.
- Custom prompts (Settings › Prompts) layer on top of the built-ins. Keep the Settings defaults in `src/promptDefaults.ts` aligned with the worker's prompt builders.

## Memory

- The note format (`<type>_<slug>.md` with `type`/`title`/`description`/`modified` frontmatter) is one contract shared by `src-tauri/src/memory.rs` and `worker/src/builtin/memory/store.ts`. Change both together.
- The system prompt gets only the generated index (`memoryIndex`), refreshed before each run and never mid-run, so the prompt cache survives.
- Memory tools are gated by their own setting via `inactiveTools()` and stay out of sub-agents.

## Slash commands and skills

- A command's key (`commandKey` in `worker/src/slash.ts`) must stay identical across the Settings denylist, `commands-scan.ts` and the worker's catalog.
- WackCode owns six names (`APP_COMMAND_NAMES`: `/init`, `/compact`, `/new`, `/name`, `/copy`, `/goal`). The first entry to claim any other name wins it; a later clash is renamed `<source>:<name>` (`resolveCommandNames`), and a switched-off command frees its name.
- Skill and command folders are re-read before every run, so edits on disk apply from the next message. Where they may load from is a security rule; see [security.md](security.md#pi-lockdown).

## Errors and redaction

Errors the worker reports pass `safeError`, which strips the credentials this worker holds (the chat's, and those of its sub-agents' connections and MCP servers). Rust redacts and bounds them again with `redact_and_limit`. Keep both layers.
