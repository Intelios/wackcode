# Security boundaries

These are product guarantees, not implementation details. Don't weaken any of them. If a change seems to need to, stop and raise it instead.

## Network

No telemetry, account, backend, updater, or traffic on launch or on a timer. The app contacts only:

- provider endpoints the user configured, when a chat or a settings test asks for it;
- the user's Git remote, only when they push.

Never add a network origin silently. Call it out explicitly, and list it here.

## Credentials

- API keys persist only in `secrets.json` (mode 0600) in the app's data folder.
- A key travels from `secrets.json` to the request that needs it and nowhere else: never into `state.json`, a session file, an environment variable, a command line, a log, an error message or the usage ledger.
- Processes the agent starts (the `bash` tool, PTYs, Run and Build) inherit the user's login-shell environment with provider key variables removed.
- Error text shown to the user or written to disk is redacted first.

## The automation socket

`cargo dev` builds the app with the `automation` feature, which opens a local Unix socket that can read the UI and send it clicks and keys (see [testing.md](testing.md)). It exists so agents can check UI work.

- It is compiled only with the `automation` feature and only in debug builds. A release build cannot contain it.
- It listens only when `WACKCODE_AUTOMATION` names a socket path (`.cargo/config.toml` sets it for `cargo` runs only), and the socket file is made private (mode 0600) as soon as it exists.
- It is never a route for credentials: it has no command that returns a key.

## Reading project files

Project context files such as `AGENTS.md` load on purpose. Everything else the agent reads (files, tool output, web pages) is data, never instructions; the system prompt says so and no feature may undo that.
