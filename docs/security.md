# Security boundaries

These are product guarantees, not implementation details: `README.md` promises them to users. Don't weaken any of them. If a change seems to need to, stop and raise it instead.

## Network

No telemetry, account, backend, updater, automatic model discovery, or traffic on launch or on a timer. The app contacts only:

- user-configured provider endpoints;
- Pi's subscription endpoints, after an explicit sign-in (tokens refresh during prompts, never on a timer);
- the npm registry and a package's own source, while the user browses or installs one;
- pi.dev's catalogue page while the user browses Settings › Skills (redirects within pi.dev only; falls back to the npm registry);
- public pages the agent reads with the built-in `web_fetch` during a run (GET only, public IPs only, user-switchable);
- pages explicitly opened in the per-chat browser preview, plus their assets, API and WebSocket traffic (including in background chats);
- HTTP/SSE MCP servers the user adds (their URL's origin only; headers refused over plain `http://` except to localhost);
- the user's Git remote and GitHub (through their own `gh` login), only when they fetch, pull, push or open a PR. Git mode also fetches once when it opens or switches repository (`git_fetch` with `background`, which can never raise a credential prompt); nothing fetches on a timer, on window focus, or while Git mode is closed.

Never add a network origin silently; call it out explicitly and update `README.md`.

**CSP** stays `default-src 'self'` with IPC-only `connect-src`. The renderer never fetches; remote calls go through Rust or the package-manager process.

## Credentials

- API keys and MCP header/env values (`mcp:<id>`) persist only in `secrets.json` (0600). OAuth credentials live only in per-provider 0600 `auth.json` files, never in the Pi CLI's own auth file.
- A worker receives credentials only over its stdin (`init`, `set_subagents`, `set_mcp`, the first prompt's `autoTitle`), and only the auth paths it needs.
- Credentials never go in argv, env vars, `wackcode.json`, Tauri events, or the renderer. The one exception is an MCP env value, which becomes its own stdio server's environment.
- Workers get the login-shell environment (`shell_env.rs`) with provider keys stripped afterwards (`strip_provider_env`). Keep that stripping in every process that inherits it.
- Keep both redaction layers: `safeError` in the worker and `redact_and_limit` in Rust.
- The macOS Keychain backend was removed on purpose; don't bring it back.

## Pi lockdown

- The worker keeps Pi's telemetry, version check, network model refresh and all auto-discovery disabled: every `no*` flag stays `true`, and the worker runs with `PI_OFFLINE`.
- Packages load only from explicit paths Rust passes for trusted packages, so a project's own `.pi/` never runs.
- User skill folders reach a worker only as the absolute roots `skills::payload` names (`~/.agents/skills`, then folders the user switched on). A project's own skill folders never load. Settings writes only inside `~/.agents/skills`.
- User commands (Settings › Commands) reach a worker only as `<app data>/commands` via `slash_commands::payload`, and Settings writes only inside it.
- Paths the renderer sends back for skills and commands are checked canonically against those folders (no symlink escapes); deletes go to the Trash.
- At launch, `subscription-models.js` re-lists each signed-in subscription's models from Pi's bundled catalogue. It runs offline in a stripped-env process, never refreshes a token, and prints only model metadata.
- Settings' command list comes from `commands-scan.js`, which executes trusted extension code in a keyless, stripped-env, offline process, like `skills-scan.js`.
- Project context files such as `AGENTS.md` load on purpose.

## Package trust

An installed extension is unsandboxed code inside the worker that holds the API key. `trusted_at` is set only by a user-confirmed `install_package` or `trust_package`. Never default it, and keep `sync_packages` from trusting packages that arrive by any other route. Untrusted or switched-off resources never reach a worker at all (the resource gate in `worker.rs`).

## Asset protocol

Enabled only for `$APPDATA/backgrounds/*`. Only `choose_background_image` writes there: the native picker opens in Rust and the file is checked by magic bytes. The renderer never passes it a path. Don't widen the scope.

## Computer use

Off by default. Rust `computer_use/` is the security boundary; the worker's tools only describe requests.

- Every request re-checks the setting, the block list (`policy.rs`) and the chat's in-memory app grants. Grants are per chat and never written to disk.
- The access card is answered renderer → `computer_use_respond_access`, never through the worker.
- WackCode itself (its own pid, any `com.wackcode.*` bundle id, dev or installed) is never observed or driven.
- Keys go only to the target pid, never to the HID stream. Only a hit-tested pointer action may use the foreground, after the user is idle, and it restores the previous app and cursor.
- Secure text fields are never read or typed into.
- All AX handles live on the engine thread (`engine.rs`).
- Sub-agents never get computer tools, and `computer_response` bypasses the worker's queue like `browser_response`.

## Browser preview

Pages get an ephemeral per-chat data store discarded at exit, no Tauri capability, no asset protocol, and no renderer storage. Page text, console output and element names are untrusted tool data.
