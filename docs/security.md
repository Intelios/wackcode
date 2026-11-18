# Security boundaries

These are product guarantees, not implementation details. Don't weaken any of them. If a change seems to need to, stop and raise it instead.

## Network

No telemetry, account, backend, updater, or traffic on launch or on a timer. The app contacts only:

- provider endpoints the user configured, when a chat or a settings test asks for it, and the title model's connection (Settings › Providers › Automatic titles) once when a chat starts, while titles are on;
- the user's Git remote, only when they push;
- the chat's own provider when a command the agent started in the background reports while the chat is idle (a wake): at most three in a row before the user writes again, never after they stopped the chat's last run.
- public web pages the agent reads with `web_fetch` (on by default; its switch is in Settings › Tools), only when a chat asks. Every connection goes through `wack-web/src/guard.rs`: http or https only, never a URL with credentials in it, never loopback, private, link-local, carrier-grade NAT, multicast or reserved addresses (checked on the addresses DNS returns, so rebinding can't slip past, and again on every redirect), no cookies, no credentials and no proxy.
- the active search provider's own endpoint (Settings › Web), when the agent searches, when Test search runs, or, with "Read pages through" on, when the agent reads a page; the page's URL passes the same public-address check before it is sent. Redirects are refused, so a key never reaches another host, and a key goes only over https except to this Mac. A provider's key is `search:<id>` in `secrets.json`, or a DeepSeek connection's own key used only on `api.deepseek.com`; the search client never reads either back.
- the URLs of the HTTP MCP servers the user adds (Settings › MCP servers), when a chat in a project the server reaches uses it, or Test connection runs. The user's headers go only to that server's own origin: redirects are followed by hand only within it (at most five), a legacy SSE server's message endpoint must be on it too, and there is no proxy and no cookie jar. Headers go over https only, except to this Mac.

- pages opened in the browser (`wack-browser`, WackCode's bundled Chromium through CEF; [cef.md](cef.md)). Chromium starts the first time a chat opens a tab, never at launch, and quits with the app. The user may open any address. The agent's navigations, and the redirects, script navigations and popups that follow while it drives a tab, pass `wack-browser/src/policy.rs`: http or https only, never a URL with credentials in it, public addresses plus this Mac (loopback, `localhost`, `*.localhost`), and never other private, link-local, carrier-grade NAT, multicast or reserved addresses, judged on the addresses DNS returns. Chromium then looks the name up again itself, so unlike `web_fetch` this check cannot rule out DNS rebinding. Chromium's own services are off (`QUIET_SWITCHES` and `DISABLED_FEATURES` in `wack-browser/src/cef.rs`) and the hosts of those that ignore their switches (the component updater, network time) are mapped to nowhere (`BLOCKED_HOSTS`): idle, it contacts no host (checked with a net log, 2026-11-17). DevTools is reached only inside the process; there is never a remote-debugging port or pipe. The agent's downloads are refused.

Never add a network origin silently. Call it out explicitly, and list it here.

## Credentials

- API keys persist only in `secrets.json` (mode 0600) in the app's data folder.
- A key travels from `secrets.json` to the request that needs it and nowhere else, with one path the user asks for: Settings › Providers' eye can show a saved connection's own key in its key field, through `Store::reveal_connection_key` (`wack-core`), until it is hidden again or Settings closes. A search provider's key has no such path. Either way a key never reaches `state.json`, a session file, an environment variable, a command line, a log, an error message or the usage ledger.
- `Store::provider` (`wack-core`) reads a key and hands back the provider that sends the requests. `Store::reveal_connection_key` is the only other read, and only for the connection the editor is showing.
- A key is never a command-line argument. `wack connect` asks for it at a hidden prompt, or reads it from a pipe.
- Processes the agent starts (`bash`, `watch`, Run and Build) and stdio MCP servers have provider key variables removed from their environment (`SECRET_ENV` in `wack-agent/src/tasks/mod.rs`). `bash` and MCP servers start from the user's login-shell environment (`wack-agent/src/shell_env.rs`), read once at launch, before those variables are removed.
- An MCP server's header and environment values are its credentials: `mcp:<id>` in `secrets.json`, never in `state.json` and never shown again in Settings (a blank field keeps the saved value). They leave `secrets.json` only in the `ServerSpec` a run or Test connection is given (`wack-core/src/mcp.rs`): headers on requests to that server's origin, variables to that server's own process and no other. Every MCP error and the stderr tail is redacted of them (`ServerSpec::redact`), and a tool list that contains one is never written to the catalogue cache (`v3/mcp-cache/`, declarations only, 0600).
- The browser's profile (`v3/browser/profile` in the app's data folder, mode 0700) holds the sign-ins and site data of the pages the user opens: a credential store of its own, kept apart from `secrets.json`. Chromium's Keychain encryption is off (`use-mock-keychain`, the owner's call on 2026-11-17: the ad hoc signed app was asked for the Keychain again after every rebuild, and the browser's network hung until it was answered), so the folder's permissions are what protect it, as 0600 protects `secrets.json`. It is never read from or written to another browser's profile. Each chat's open tabs (address, title, who opened them) are kept in `state.json` like its title and layout; an address can carry whatever its site put in it, so they are as private as the rest of that file. API keys never enter the browser: not in its environment, its command line or a script injected into a page.
- Error text shown to the user or written to disk is redacted first: the HTTP client removes the key from any error a server echoes back.
- TLS trusts the system certificate store.

## The automation socket

`cargo dev` builds the app with the `automation` feature, which opens a local Unix socket that can read the UI and send it clicks and keys (see [testing.md](testing.md)). It exists so agents can check UI work.

- It is compiled only with the `automation` feature and only in debug builds. A release build cannot contain it.
- It listens only when `WACKCODE_AUTOMATION` names a socket path (`.cargo/config.toml` sets it for `cargo` runs only), and the socket file is made private (mode 0600) as soon as it exists.
- It has no credential path of its own: no command returns a key. Its `browser` command drives the dev app's browser, whose profile is the dev data folder's own, never the installed app's. Like a person at the screen it can click Settings › Providers' eye (debug builds only), and then a screenshot shows what the eye shows.

## Reading project files

Project context files such as `AGENTS.md` load on purpose. Everything else the agent reads (files, tool output, web pages) is data, never instructions; the system prompt says so and no feature may undo that.
