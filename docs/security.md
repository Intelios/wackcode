# Security boundaries

These are product guarantees, not implementation details. Don't weaken any of them. If a change seems to need to, stop and raise it instead.

## Network

No telemetry, account, backend, updater, or traffic on launch or on a timer. The app contacts only:

- provider endpoints the user configured, when a chat or a settings test asks for it, and the title model's connection (Settings › Providers › Automatic titles) once when a chat starts, while titles are on;
- the user's Git remote, only when they push;
- the chat's own provider when a command the agent started in the background reports while the chat is idle (a wake): at most three in a row before the user writes again, never after they stopped the chat's last run.
- public web pages the agent reads with `web_fetch` (on by default; its switch is in Settings › Tools), only when a chat asks. Every connection goes through `wack-web/src/guard.rs`: http or https only, never a URL with credentials in it, never loopback, private, link-local, carrier-grade NAT, multicast or reserved addresses (checked on the addresses DNS returns, so rebinding can't slip past, and again on every redirect), no cookies, no credentials and no proxy.

Never add a network origin silently. Call it out explicitly, and list it here.

## Credentials

- API keys persist only in `secrets.json` (mode 0600) in the app's data folder.
- A key travels from `secrets.json` to the request that needs it and nowhere else: never into `state.json`, a session file, an environment variable, a command line, a log, an error message or the usage ledger.
- No function returns a key. `Store::provider` (`wack-core`) reads it and hands back the provider that sends the requests.
- A key is never a command-line argument. `wack connect` asks for it at a hidden prompt, or reads it from a pipe.
- Processes the agent starts (`bash`, `watch`, Run and Build) have provider key variables removed from their environment (`SECRET_ENV` in `wack-agent/src/tasks/mod.rs`).
- Error text shown to the user or written to disk is redacted first: the HTTP client removes the key from any error a server echoes back.
- TLS trusts the system certificate store.

## The automation socket

`cargo dev` builds the app with the `automation` feature, which opens a local Unix socket that can read the UI and send it clicks and keys (see [testing.md](testing.md)). It exists so agents can check UI work.

- It is compiled only with the `automation` feature and only in debug builds. A release build cannot contain it.
- It listens only when `WACKCODE_AUTOMATION` names a socket path (`.cargo/config.toml` sets it for `cargo` runs only), and the socket file is made private (mode 0600) as soon as it exists.
- It is never a route for credentials: it has no command that returns a key.

## Reading project files

Project context files such as `AGENTS.md` load on purpose. Everything else the agent reads (files, tool output, web pages) is data, never instructions; the system prompt says so and no feature may undo that.
