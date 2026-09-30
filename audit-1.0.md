# WackCode 1.0 readiness audit

Audited 30 September 2026 against commit `1b719f2` ("Git Mode Improvements"). Line numbers refer to that commit.

**Baseline:** `pnpm check` compiles clean and all 1,086 tests pass (551 web, 322 worker, 213 Rust).

**Method:** read the core of all three layers (renderer, Rust host, worker), then drove the dev app to confirm each finding. Everything below was reproduced in the running app unless marked *(from code)*.

**Summary:** 12 reproduced bugs, several of which lose work or messages. Bugs 1–7 are the ones to fix before calling it 1.0.

---

## 1. Bugs and glitches

### Fix before 1.0

#### 1. Opening Settings › Packages stops every running chat

- **What happens:** a chat in the middle of `sleep 40` ended as "Stopped" the moment the Packages page opened. Nothing tells you why.
- **Cause:** `PackagesSection` refreshes on mount (`src/components/PackagesSection.tsx:195`). `refresh_packages` (`src-tauri/src/commands.rs:823`) calls `sync_packages`, which terminates every worker (`commands.rs:785`). The other package commands call `refuse_while_busy` first; this one doesn't.
- **Fix:** only restart workers when the resolved resource paths actually changed, and never for a plain refresh.

#### 2. Anything you do in the first second of a new chat goes wrong

The worker takes about 0.9 s to start. Three separate failures in that window:

- **A second message about 0.25 s after the first creates a second chat.** Both run in the same folder. The hero composer unfreezes too early: `transitioning` is cleared as soon as it has a `taskId` but before selection switches (`src/App.tsx:876`), so the second send still sees no selected chat.
- **A second message about 0.7 s after the first vanishes.** Not in the transcript, not in the composer, no error. `queue_message` skips the worker's queue (`worker/src/index.ts:2150`) and hits "Worker is not initialized" (`index.ts:1665`). Rust sends it fire-and-forget (`commands.rs:2515`), so the renderer clears the draft, and the error response is wiped by the next snapshot (`App.tsx:821`).
- **Stop during start-up is ignored.** `abort` bounces the same way and the run carries on. Confirmed against the worker directly: both commands sent 50–100 ms after spawn returned "Worker is not initialized".
- **Fix:** hold queue-bypassing commands until `init` finishes (as `watch_subagent` and `tool_image` already do), and keep the hero frozen until selection has switched.

#### 3. A chat whose model was removed from its connection can't be opened

- **What happens:** empty transcript, "Worker is not initialized" banner, model pill says "Choose model". 13 of the 59 chats in the dev data are in this state (all used `deepseek-ai/DeepSeek-V4-Flash`).
- **Cause:** `init` throws "Configured model was not found" (`worker/src/index.ts:1210`). `open_task` then sends `snapshot` (`commands.rs:2015`), which fails with the generic error and overwrites the real one.
- **Fix:** let a session load for reading without a valid model, and show "This chat's model is no longer configured. Pick another to continue."

#### 4. The error banner's Dismiss button does nothing

- **What happens:** once an error is saved on the chat, pressing Dismiss leaves the banner in place.
- **Cause:** the banner shows `runtime.error || task.lastError`, but Dismiss only clears `runtime.error` (`src/App.tsx:2563`).
- **Related:** the `run_state` handler sets `lastError` to `undefined` on idle in the renderer only (`App.tsx:724`), so the renderer and `wackcode.json` disagree and the error comes back after a restart.
- **Fix:** Dismiss clears `lastError` too, through a Rust command so it persists.

#### 5. Ctrl+N, Ctrl+O and Ctrl+, are treated as ⌘ shortcuts

- **What happens:** Ctrl+N in the terminal or composer opens a new chat and discards the draft. Ctrl+O, which is save in nano, would open the folder picker *(from code)*.
- **Cause:** `event.metaKey || event.ctrlKey` in the global handler (`src/App.tsx:2243`), and the terminal deliberately passes those keys up (`src/components/TerminalPanel.tsx:81`).
- **Fix:** `metaKey` only, in both places.

#### 6. `#` can't be typed in the terminal on a British keyboard

- **What happens:** Option+3 produces nothing. Other layouts lose `@ [ ] { } | \` the same way.
- **Cause:** `macOptionIsMeta: true` (`TerminalPanel.tsx:70`).
- **Fix:** default it off, or make it a setting.

#### 7. Links in replies do nothing when clicked

- **Cause:** `target="_blank"` with nothing to open it (`src/markdown-components.tsx:104`).
- **Fix:** handle the click and open `http`/`https` links through Rust. Validate the scheme there: `reveal_path` currently passes any string to `open` (`commands.rs:4112`).

### Fix soon

#### 8. A rewound message reappears in a different chat's composer

- **What happens:** after any rewind, going to New chat and then opening another chat refills the composer with the rewound text.
- **Cause:** `composerSeed` is global and keeps its nonce. Moving from the hero's seed to it counts as a new nonce, so the seed effect fires again (`src/App.tsx:2826`, `src/components/Composer.tsx:169`).
- **Fix:** key the seed to its chat, or clear it once applied.

#### 9. The Changes panel spins forever when a background chat runs tools

- **What happens:** the refresh icon and loading bar were still animating 20 s after every chat went idle.
- **Cause:** `refreshChanges` runs for the background chat on each `tool_execution_end` (`App.tsx:772`). It bumps the request counter and sets loading, but only clears loading for the selected chat (`App.tsx:621`).
- **Fix:** skip the refresh when the event's chat isn't the selected one. That also saves a full Git status per background tool call.

#### 10. Git mode History can hang and silently show stale commits

- **What happens:** with 2,103 unpushed commits, the `git rev-list` child hung and History kept showing the old three commits with no error.
- **Cause:** `run_git` polls `try_wait` without reading stdout, so any output over about 64 KB blocks until the 90 s timeout (`src-tauri/src/git.rs:325`). The error is then only rendered when the list is empty (`src/components/GitSidebar.tsx:239`).
- **Also affected** *(from code)*: the branch picker's `for-each-ref` (`git.rs:498`) in a repo with around 800 or more refs.
- **Fix:** drain stdout and stderr on threads, as `run` in `checkpoints.rs:138` already does. Show the log error even when commits are listed.

#### 11. Deleting a worktree chat leaves its branch behind

- **What happens:** the worktree is removed but `wackcode/<slug>-<id>` stays in your repository.
- **Cause:** `remove_worktree` never deletes the branch (`git.rs:91`).
- **Fix:** `git branch -d` afterwards, which only succeeds when nothing would be lost.

#### 12. Archived view: the confirm label is clipped

- **What happens:** "Unarchive?" shows as "Unarch" with the bin icon drawn over it.
- **Cause:** the sizing rule is `.task-item .row-menu.confirming` (`src/styles.css:208`), and archived rows are `.archived-row`.

### Smaller

- **Enter on a sidebar row leaks into the chat it opens.** The same keypress reaches that chat's terminal (an extra prompt line appeared each time). The row's handler doesn't `preventDefault` (`src/components/Sidebar.tsx:121`).
- **A user Stop sometimes renders as a red error.** Two saved chats show "This operation was aborted" in an error block instead of the quiet "Stopped" label.
- **The Ultra Plan placeholder wraps and is cut off** (`Composer.tsx:507`).
- **The Browser panel's "Take control" button is clipped** to "ke co" at about 380 px panel width.
- **Memory folders of removed projects stay listed** in Settings › Memory with no way to remove them.
- *(from code)* **Error redaction mangles ordinary words.** Any `sk-` counts as a key, so "task-list" or "ask-user-question" in an error becomes "[credential redacted]" (`src-tauri/src/worker.rs:1444`).
- *(from code)* **The stderr tail can panic.** `tail[tail.len() - 4_000..]` panics if the cut lands inside a multi-byte character, after which the worker's stderr is no longer drained (`worker.rs:359`).
- *(from code)* **A failed first message leaves an orphan chat** in the sidebar (`App.tsx:1474`).
- *(from code)* **The Changes panel shows "No Git repository" while loading** on each chat switch, because `changes` is undefined (`src/components/ChangesPanel.tsx:219`).
- *(from code)* **A corrupt `wackcode.json` stops the app launching**, with no window and no backup to fall back on (`src-tauri/src/lib.rs:240`, `storage.rs:19`).

---

## 2. Weaker than Codex / Claude Code

- **Drafts aren't per chat.** There is one composer state (`Composer.tsx:88`), so text typed in chat A shows up in chat B, and ⌘N discards it.
- **The transcript only exists while a worker is alive.** That is why bug 3 blanks a whole chat, and why reopening an idle chat waits on a worker start. Reading the session file directly would make history instant and independent of the model.
- **Sidebar.** Chats are listed oldest first, so the newest is at the bottom. There is no search and no timestamps.
- **Tool output is capped at the last 60 lines** with no "show all" or copy (`src/components/ToolRow.tsx:80`). Code blocks have no copy button.
- **Changes loads every file's full diff on every refresh.** One or two `git diff` processes per file, plus reading each file whole to detect binaries (`git.rs:108`, `:943`), after every tool call. Loading only the selected file's diff fixes it.
- **Right-click shows the stock WebKit menu** (Look Up, Search with Google, Share). There are no context menus of your own anywhere.

---

## 3. UI

### Settings has two generations of page

Computer use, Prompts and Appearance use a centred column with an illustrated header and look finished. The other nine are full-width with a plain heading, and their text runs about 1,300 px wide. The weakest:

- **Tools:** a bare list showing Pi's model-facing descriptions verbatim.
- **MCP servers:** the empty state is one grey line, next to the rich empty states on Commands and Packages.
- **Memory:** one large identical empty card per project, stacked.

Every page also repeats its name: the eyebrow "TOOLS" sits directly above the title "Tools".

### Elsewhere

- **Browser tools show raw names in the transcript** (`browser_open`, `browser_snapshot`), while computer use gets "Inspected TextEdit". They fall through to the default case (`src/tool-utils.ts:193`).
- **About 60 native `title` tooltips sit alongside your own `Tooltip`**, including the chat header buttons and the browser toolbar. They are slow and unstyled, and Changes never shows ⌘⇧C.
- **The image backdrop's lower half is masked out on the new-chat screen** (`styles.css:130`), but the crop editor shows the picture unmasked, so what you line up there isn't what you get.

### Minor

- Link focus rings are system blue, not the accent.
- The "Mono" theme sits alone on a second row.
- "Always on" built-ins look like disabled switches.
- The branch picker on the new-chat screen opens over the title.

### The standard to match

Git mode, the question card, the reasoning slider and the Computer use page feel finished. The rest of the app should be brought up to them.
