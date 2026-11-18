# CEF

The browser pane is Chromium, embedded through CEF (the Chromium Embedded Framework) with the [`cef`](https://github.com/tauri-apps/cef-rs) crate. Like GPUI, it is pinned and moved on purpose.

## What is pinned

The root `Cargo.toml` pins `cef` and `cef-dll-sys` to one exact version, for example `=156.6.0`. The part after `+` in the crate's full version (`156.6.0+156.0.30`) is the CEF release, and that release fixes the Chromium version (156.0.8078.96 for this one). Both crates must share the version.

The first build downloads that CEF distribution (about 100 MB compressed) once, into `target/cef/` (`CEF_PATH` in `.cargo/config.toml`). This is a build-time download from CEF's own build server (cef-builds.spotifycdn.com). The app never contacts that server.

## Why the app is a bundle

On macOS, CEF only runs from inside an `.app` bundle. The bundle holds two things:

- The framework, at `Contents/Frameworks/Chromium Embedded Framework.framework`.
- Five helper apps (`WackCode Helper.app`, `… (GPU)`, `… (Renderer)`, `… (Plugin)`, `… (Alerts)`). Chromium runs its renderer, GPU and utility processes from these.

The helpers are one binary, `apps/wackcode-helper`, copied five times. `apps/xtask` assembles the bundle (`bundle.rs` holds the layout and its invariants):

| Command | What it does |
|---|---|
| `cargo dev` | Builds the app with the `automation` feature and writes `target/debug/WackCode Dev.app` (identifier `com.wackcode.desktop.dev`, dev icon). Then it runs the app in its own place, so the terminal and Ctrl-C behave as before. |
| `cargo xtask bundle` | A release build in `target/release/WackCode.app`, signed ad hoc from the inside out. Notarization is out of scope. |

The framework is copied into the bundle only when the CEF distribution changes; the executables are copied on every run. `cargo run -p wackcode` still starts the bare binary, but that build has no browser.

The app loads the framework first thing in `main`, before any other thread exists (`wack_browser::cef::load_library`): loading it swaps the process's malloc zones for Chromium's allocator, and a thread freeing memory while that happens crashes the app (seen once, loading it lazily on the first page, 2026-11-18; loading at launch is the owner's call, the same day). Loading starts nothing: Chromium itself starts, with its helper processes, the first time a tab is shown or used.

## Bumping

A bump is its own task and its own commit, never folded into feature work. WackCode has no updater, so the bundled Chromium only gets security fixes when it is bumped. Bump it for every Chromium stable release that carries security fixes.

1. Change the version of both `cef` and `cef-dll-sys` in the root `Cargo.toml`.
2. Run `cargo update -p cef -p cef-dll-sys` and `cargo check --workspace`. Fix whatever the new bindings broke.
3. Run `cargo test --workspace`.
4. Run `cargo dev` and use the browser. Check that `target/debug/WackCode Dev.cef-source` names the new distribution.
5. Run `cargo deny check licenses`.
6. Delete the old version's folder from `target/cef/`.

## Licences

The `cef` crates are Apache-2.0 or MIT. CEF itself is BSD-3-Clause, and Chromium is BSD-3-Clause plus the third-party licences listed in its credits. The distribution in `target/cef/` carries `LICENSE.txt` and the Chromium credits. A release bundle must ship them too; this is to do before the first release that includes the browser.
