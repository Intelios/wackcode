# Security boundaries

These are product guarantees, not implementation details: `README.md` promises them to users. Don't weaken any of them. If a change seems to need to, stop and raise it instead.

## Read-only policies

Plan / Ultra Plan and read-only sub-agents enforce inspection-only shell commands and block editing tools by default. This is a behavioral policy, not a filesystem sandbox: even allowed tests can run project hooks or write caches.

Settings › Tools has two explicit, app-wide overrides, both off by default and confirmed with a danger dialog on every enable. They persist until disabled and apply between turns without stopping active runs. Removing planning restrictions gives the parent its normal enabled tools, including trusted package tools, MCP and permitted browser/computer actions; plan review and approval still apply. Removing child restrictions gives normally read-only roles edit/write and unrestricted bash, filtered through tool switches. Read-only planning still restricts those children and refuses editing roles.

These overrides allow changes outside the project and unrestricted shell commands with the user's permissions. They do not change credential handling, package trust, Pi lockdown, computer-use controls, or the child extension/tool inventory. Child prompts retain custom instructions and explicit task constraints.

## Network

No telemetry, account, backend, updater, automatic model discovery, or traffic on launch or on a timer. The app contacts only:

- user-configured provider endpoints;
- Pi's subscription endpoints, after an explicit sign-in (tokens refresh during prompts, never on a timer);
- the npm registry and a package's own source, while the user browses or installs one;
- pi.dev's catalogue page while the user browses Settings › Skills (redirects within pi.dev only; falls back to the npm registry);
- `https://models.dev/catalog.json`, only after **Settings › Providers › Refresh catalog**. Rust downloads public metadata without credentials or cookies, refuses redirects, and bounds the request to 20 seconds and 16 MiB. Catalogue URLs are matching hints only; they never replace connection endpoints. Opening Settings, launching the app and starting a chat do not contact Models.dev;
- public pages the agent reads with the built-in `web_fetch` during a run (GET only, public IPs only, user-switchable);
- the search provider the user picks in Settings › Web, during a run or a Test search: a preset's fixed API (`api.deepseek.com`, `api.search.brave.com`, `api.tavily.com`) or the URL the user entered (SearXNG, Custom). Rust (`web.rs`) makes these requests, so the key never reaches a worker; it contacts only that endpoint, refuses redirects, and sends a key only over https (or to this Mac). With "Read pages through <provider>" on, every `web_fetch` URL goes to the provider's fetch endpoint instead, after the same public-address checks in the worker and again in Rust. Off until the user adds a provider and switches it on;
- pages explicitly opened in the per-chat browser preview, plus their assets, API and WebSocket traffic (including in background chats);
- HTTP/SSE MCP servers the user adds (their URL's origin only; headers refused over plain `http://` except to localhost);
- the user's Git remote and GitHub (through their own `gh` login), only when they push or open a PR. Reading changes and switching branches use local refs; the app never fetches automatically.

Never add a network origin silently; call it out explicitly and update `README.md`.

**CSP** stays `default-src 'self'` with IPC-only `connect-src`, plus the one `frame-src
wcvisual:` that lets a visual reply's frame load (see [Visual replies](#visual-replies)). The
renderer never fetches; remote calls go through Rust or the package-manager process.

## Credentials

- API keys, MCP header/env values (`mcp:<id>`) and search provider keys (`search:<id>`) persist only in `secrets.json` (0600). A search provider that borrows a connection's key is linked by id and reads it at send time, never copying it, and only when the provider's endpoint has the same origin as that connection (`web::linkable_connection`); Custom and SearXNG can never borrow one. A connection a search provider borrows from can't be deleted. OAuth credentials live only in per-provider 0600 `auth.json` files, never in the Pi CLI's own auth file.
- A worker receives credentials only over its stdin (`init`, `set_subagents`, `set_mcp`, a first run's `autoTitle`), and only the auth paths it needs. `set_web` carries none: a worker asks Rust to search (`web_request`).
- Credentials never go in argv, env vars, `wackcode.json`, Tauri events, or the renderer. The one exception is an MCP env value, which becomes its own stdio server's environment.
- Workers get the login-shell environment (`shell_env.rs`) with provider keys stripped afterwards (`strip_provider_env`). Keep that stripping in every process that inherits it.
- Keep both redaction layers: `safeError` in the worker and `redact_and_limit` in Rust.
- The macOS Keychain backend was removed on purpose; don't bring it back.

## Chat mode's scratchpad

A Chat mode chat's file tools (`read`, `write`, `edit`, `ls`, `grep`, `find`) are confined to its scratch folder, and it has no shell. The worker enforces this at call time (`worker/src/builtin/chat-mode/`), fail-closed, comparing real paths so `..`, `~`, absolute paths and symlinks out of the folder are all refused.

This is a behavioral policy, not a filesystem sandbox, and `README.md` says so. It does not cover:

- **MCP servers**, which run with the user's login-shell environment and do whatever their tools do.
- **The browser**, which loads what it is pointed at.
- **A hard link** placed inside the scratchpad, which the path check cannot tell from a file.

Don't describe it as more than this, and don't add a tool to Chat mode's allowlist (`policy.ts`) that can reach the filesystem or run a process without routing its paths through `confine`.

Chat mode is also where the app has no permission prompts by design: there is nothing to approve, because there is no path outside the scratchpad to ask about. Keep it that way (see `AGENTS.md`).

The one cross-chat read is `read_chat`, and it takes no path: the worker sends a chat id to the
host (`chat_refs.rs`), which refuses anything that is not a saved chat with a session file —
never the reading chat itself, and for a Chat mode chat only another Chat mode chat, so project
material from a Code chat cannot reach a scratchpad-confined chat. The worker then renders that
one session read-only (`worker/src/chat-history.ts`); nothing else from the chat crosses.

A Chat mode worker loads no package, even a trusted one, and receives no sub-agent settings: that payload carries other connections' credentials.

## Pi lockdown

- The worker keeps Pi's telemetry, version check, network model refresh and all auto-discovery disabled: every `no*` flag stays `true`, and the worker runs with `PI_OFFLINE`.
- Packages load only from explicit paths Rust passes for trusted packages, so a project's own `.pi/` never runs.
- User skill folders reach a worker only as the absolute roots `skills::payload` names (`~/.agents/skills`, then folders the user switched on). A project's own skill folders never load. Settings writes only inside `~/.agents/skills`.
- `/skill-creator` drafts live under `agent/<task>/skill-creator/` in app data, never in a skill root, so a draft never loads as a skill. Publication is a user action (`publish_skill_draft`): it re-verifies the reviewed revision and writes `~/.agents/skills` under the shared library lock. The agent's own tools never write the library.
- User commands (Settings › Commands) reach a worker only as `<app data>/commands` via `slash_commands::payload`, and Settings writes only inside it.
- Paths the renderer sends back for skills and commands are checked canonically against those folders (no symlink escapes); deletes go to the Trash.
- At launch, `subscription-models.js` re-lists each signed-in subscription's models from Pi's bundled catalogue. It runs offline in a stripped-env process, never refreshes a token, and prints only model metadata.
- Settings' command list comes from `commands-scan.js`, which executes trusted extension code in a keyless, stripped-env, offline process, like `skills-scan.js`.
- Project context files such as `AGENTS.md` load on purpose.

## Package trust

An installed extension is unsandboxed code inside the worker that holds the API key. `trusted_at` is set only by a user-confirmed `install_package` or `trust_package`. Never default it, and keep `sync_packages` from trusting packages that arrive by any other route. Untrusted or switched-off resources never reach a worker at all (the resource gate in `worker.rs`).

## Visual replies

`show_visual` lets the model author HTML and JavaScript, so it runs in its own browsing context,
never in the renderer. Rust (`src-tauri/src/visuals.rs`) is the boundary:

- **Staging.** The renderer hands the document to `stage_visual`, which keeps it in memory (128
  entries / 48 MiB, oldest evicted, at most 8 MiB per document) and returns a `wcvisual://localhost/<token>` URL. Nothing is
  written to disk, no path is involved, and the app's own webview is the only one served: the
  handler 404s for any other webview, any other method, and any unknown or non-hex token.
- **The frame's own CSP.** The document is served with `default-src 'none'; script-src
  'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; connect-src 'none';
  form-action 'none'; base-uri 'none'`. It has no network of any kind — no scripts, fonts,
  frames or fetches — so a visual cannot exfiltrate what it is shown. Only inline scripts (the
  model's own) and inline styles run.
- **A navigation guard, natively.** A CSP belongs to the document that leaves, not the one that
  arrives, so `location.href = "https://…"` inside a visual would load a remote page in its frame:
  a network request the user never asked for. `visuals::navigation_allowed` refuses every
  navigation outside the app's own origins (the bundled `tauri://`, this run's dev server or
  `frontendDist`) and a `wcvisual:` frame — the app gains **no new network origin**, and no
  visual can become one. It is registered as a small Tauri plugin (`navigation_plugin`), because
  this window comes from `tauri.conf.json` and a config-built window carries no navigation
  handler of its own. A plugin's hook runs for every webview, so it rules only the main webview
  (visual frames are iframes inside it); the browser preview and the canvas renderer keep their
  own handlers. Do not remove it, and do not add an origin to the list for a feature that does
  not need to navigate.
- **The wrapper's theme values.** A staged document splices the renderer's CSS variables into one
  `<style>`. Each value is checked against a fixed character set and length, so no value can end
  the rule or the style element; a value that fails refuses the whole staging call.
- **The iframe is `sandbox="allow-scripts"`**: an opaque origin, no forms, no popups, no top
  navigation, no access to this app's DOM, `localStorage` or cookies. Tauri's initialization
  scripts (the IPC bridge, the invoke key, `__TAURI_INTERNALS__`) are main-frame only, and an IPC
  request additionally needs the per-launch invoke key and a parseable `Origin` (an opaque origin
  sends `null`), which the frame never has.
- **The only way out is `postMessage`**, and the card verifies `event.source` against its own
  frame: a content height, a link the user clicked (http(s) only, opened outside the app) and a
  bounded `wackcode.send` text. A send is refused unless the visual belongs to the newest answer,
  runs one per visual per second, and goes out as an ordinary literal message with a
  `<visual-action-v1>` tail, so model-authored script can never steer the chat on its own — the
  user always presses the button, and Pi never expands it as a slash command.

**Local images** in `show_visual` and `preview_visual` are inlined in the worker. Absolute image
paths in quoted attributes or CSS `url(...)` are read only after Chat mode's `confine("read")`
check; coding chats have their ordinary read access. File signatures (including an SVG root),
not extensions, decide whether bytes are images. A renamed text file is refused. Each image is
limited to 4 MiB and the finished HTML to 8 MiB. Missing, invalid and out-of-scratchpad images
refuse the tool call. No asset-protocol scope or network access is added. Published copies stay
in that chat's local session; `visual_document` retrieves only a requested visual on the current
branch, and snapshots omit its HTML. Deleting the chat deletes the saved copies.
For a vision model, `preview_visual` returns a screenshot through the ordinary tool-image
pipeline, so those pixels reach the configured model when it checks the result; the frame
itself still makes no network requests.

**Canvas artboards** are visual documents too (`kind: artboard`), in the same frame under the
same CSP and navigation guard. Their bridge is narrower: no `wackcode.send`, no link forwarding,
only a size, script errors and Esc. `canvas_look` renders them offscreen in Rust
(`canvas_render.rs`). The same renderer measures replies at three widths and serves
`preview_visual`: its `preview` kind uses reply layout over the theme background and reports
script errors, but cannot send or forward links. Only `stage_for_render` can stage a preview;
the renderer-facing `stage_visual` refuses it.

- **One render webview** (`canvas-render`), shared by artboards and previews, a child of the main window parked offscreen,
  incognito, allowed to navigate only to `wcvisual:` and `about:blank`, with downloads and new
  windows refused and the Tauri globals removed.
- **Audiences.** Documents staged for it are served to it alone, and the renderer's documents
  only to the main webview; the shell page that frames a render (`/render/<token>`, its own
  `default-src 'none'` policy with `frame-src wcvisual:`) exists only for the render webview.
  Each render's document is dropped once drawn.
- **The model's markup never runs in its main frame**: the shell frames it with the same
  `sandbox="allow-scripts"` opaque origin it has on the canvas, and only reads its reports.
- **Saving an artboard** (`export_artboard`) renders a PNG through the same webview, or wraps the
  markup as a page, and writes it only where the user picks in the native save panel opened in
  Rust; the renderer sends the document and never a path, and no new capability or origin is
  involved.
- **No capability.** Tauri counts a registered scheme such as `wcvisual:` as a local origin and
  matches a capability by webview *or* window label, so `capabilities/default.json` names only
  the `main` webview (no `windows`): no child of the main window — the render webview or a
  browser preview — gets a permission. `canvas_render.rs` pins this in a test. Don't add the
  window back.

The `frame-src wcvisual:` directive in `tauri.conf.json` is the only CSP change these features
need; `default-src`, `connect-src`, `script-src` and the asset protocol's scope are untouched.
Do not loosen the frame's own CSP: a visual with network access would be a new origin the user
never chose.

## Asset protocol

Enabled only for `$APPDATA/backgrounds/*`. Only `choose_background_image` writes there: the native picker opens in Rust and the file is checked by magic bytes. The renderer never passes it a path. Don't widen the scope.

## Images in replies

Assistant prose can include `wackcode-image://entry/<entryId>/<imageIndex>` references to images
already saved in that chat. These are identifiers, not a registered protocol: the renderer
resolves them against the current branch and displays only its saved thumbnails. Full images
use the existing `message_image` and `tool_image` commands, which check the branch again.
Remote URLs, file paths and model-supplied data URLs stay blocked; no network, filesystem or
asset-protocol access is added. Chat mode can reuse its attachments and tool images, while new
local files still pass through its confined `read` tool. Other chats receive descriptive image
markers through `read_chat`, never reusable image references.

## Computer use

Off by default. Rust `computer_use/` is the security boundary; the worker's tools only describe requests.

- Every request re-checks the setting, the block list (`policy.rs`) and app access. “Allow for this chat” grants and denials stay in memory; an explicit “Always allow” saves the host-resolved canonical path and bundle id in `wackcode.json`, across coding chats and restarts. A per-chat denial and every block still win. Chat mode and sub-agents never get computer use.
- The access card is answered renderer → `computer_use_respond_access`, never through the worker. Only an answer to a live host-raised card can create a saved approval; Settings can revoke one, never grant one. Failed saves leave effective permissions unchanged and the card retryable.
- Settings › Computer use lists saved approvals with their paths. Revoking one clears matching in-memory approvals and stops active computer use before further actions; switching the feature off keeps saved approvals but permits no access. An app moved to another path asks again. Rebuilding or replacing an app at its approved path with the same bundle id retains approval — deliberate for testing development builds, not a code-signing guarantee.
- WackCode itself — the running app's own pid, its own bundle id (`com.wackcode.desktop`, `…desktop.dev` inside the dev app) and its own paths — is never observed or driven. The other build's app (installed ↔ dev) is a normal ask-first target: it can never reach this app's windows or its access card.
- Keys go only to the target pid, never to the HID stream. Only a hit-tested pointer action may use the foreground, after the user is idle, and it restores the previous app and cursor.
- Secure text fields are never read or typed into.
- All AX handles live on the engine thread (`engine.rs`).
- Sub-agents never get computer tools, and `computer_response` bypasses the worker's queue like `browser_response`.
- The agent cursor is decoration: it ignores mouse input and has no accessibility children or hit-test target. Only its exact registered window number is skipped by pointer hit tests, never WackCode's other windows. Its coordinates stay in Rust, its labels never contain typed text or key contents, and single-window captures continue to exclude it. Showing it never activates an app or moves the system pointer.

## Browser preview

Pages get an ephemeral per-chat data store discarded at exit, no Tauri capability, no asset protocol, and no renderer storage. Page text, console output and element names are untrusted tool data. User-picked browser context uses the same isolated view: native pointer interception and client-world DOM inspection add no page capability or renderer network access. Captured DOM is inert, bounded text; scripts, styles, event handlers, arbitrary data attributes, form values and editable contents are excluded. Screenshot pixels are shared only when the user sends their attachment.
