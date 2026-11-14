# GPUI

WackCode's UI is drawn with GPUI, the framework behind Zed. GPUI has no stable releases, so the app pins one revision and moves it on purpose.

## What is pinned

The root `Cargo.toml` pins three crates to one revision of [`zeronsh/zui`](https://github.com/zeronsh/zui):

- `gpui`: the framework.
- `gpui_platform`: picks the macOS platform layer.
- `gpui_tokio`: lets GPUI views await work running on the engine's Tokio runtime.

`zui` is a standalone extraction of GPUI from Zed's repository with a small patch line on top: backdrop blur, edge fade, transparent-window alpha and the macOS 26 under-window blur. Zeron, the app WackCode's look is measured against, is built on it. Using it gives those effects without carrying our own fork.

All three crates must share the same `rev`. Two different revisions in one build produce two incompatible copies of GPUI's types.

## Build requirements

- The `runtime_shaders` feature is on, so Metal shaders compile when the app launches. Building needs only the Xcode Command Line Tools. Without the feature, GPUI's build script needs the Metal toolchain from full Xcode.
- The `font-kit` feature is on for the binaries: it is what compiles the macOS text system.
- `[profile.dev.package."*"]` builds dependencies optimized. GPUI is unusably slow otherwise.

## Features we rely on that are not in a GPUI release

- `HeadlessAppContext` with `gpui_platform::current_headless_renderer()`: renders a window to an image with the real Metal renderer and no visible window. The gallery is built on it (`apps/gallery`). It needs GPUI's `test-support` feature, which only the gallery and the dev app's `automation` feature turn on.
- `WindowOptions::app_owns_titlebar_drag`: the shell draws the title strip and moves the window with `Window::start_window_move`.

## Text input

GPUI ships no text field, so WackCode has its own: `crates/wack-ui/src/text_field/`. `edit.rs` holds the text, selection, undo and IME composition as plain, tested Rust; `mod.rs` draws it and talks to macOS through GPUI's `EntityInputHandler`, which is what makes input methods, dictation and the character palette work. It started from zui's `examples/input.rs`.

`gpui-base` (Zeron's unstyled gpui-component) was the plan, but it pins an older zui revision than ours, and Cargo will not patch a git dependency to another revision of the same repository. Owning the field also means one fewer revision to move in step with.

## Bumping

A bump is its own task and its own commit. Never fold it into feature work.

1. Change `rev` on all three lines in the root `Cargo.toml`.
2. `cargo update -p gpui` and `cargo check --workspace`. Fix what the new revision broke.
3. `cargo test --workspace`.
4. `cargo snap --all` and compare the images with the previous ones.
5. `cargo dev` and use the app: type in every text field, resize the window, switch light and dark.
6. `cargo deny check licenses`.

## Patching

We do not have our own fork. Create one (a fork of `zui`) only when WackCode needs its first change to GPUI itself; then point the three `git` URLs at it and keep the patch line short and listed here.

To take a single fix from Zed, `zui` documents its own procedure: `git format-patch -1 <sha> --stdout` in a Zed checkout, then `git apply --3way` in the fork. Paths line up because the crate layout matches Zed's.

## Licences

GPUI and its crates are Apache-2.0. WackCode is AGPL-3.0, so code may also be adapted, with attribution, from Zeron (MIT), gpui-component (Apache-2.0) and Zed's GPL-3.0-or-later crates. `cargo deny check licenses` (configured in `deny.toml`) fails on any licence not on the allow list.
