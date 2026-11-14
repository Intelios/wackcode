# Checking your work

## Tests

`cargo test --workspace` runs everything. Pure logic is tested in inline `#[cfg(test)]` modules next to the code, with `tempfile` for anything that touches disk.

## The gallery: looking at UI without the app

Every view state worth looking at is a **scene** in `crates/wack-ui/src/scenes.rs`: a view, in a known state, at a known size. The `gallery` binary renders scenes to PNG files.

```sh
cargo snap --list                              # what scenes exist
cargo snap shell --theme dark                  # writes target/gallery/shell-dark.png
cargo snap shell --theme light --out /tmp/a.png
cargo snap --all                               # every scene, light and dark
```

Rendering is headless: GPUI's test platform with the real Metal renderer and the real macOS text system. No window opens and nothing takes focus, so it works from any shell. Images are rendered at 2x.

When you change a view: add or update its scene, render it in both themes, and look at the result. A UI change is not verified until you have looked at it.

Elements that matter to tests carry a tag: end the element's builder chain with `.tag("title-strip")` (`crates/wack-ui/src/tags.rs`). `cargo snap <scene> --tags` prints each tag's bounds, and the automation socket aims clicks by tag, so tag anything a test needs to find.

## The dev app

`cargo dev` runs the app with the `automation` feature. Use it for what the gallery cannot show: real windows, focus, typing, IME, resizing, animation.

It opens a Unix socket at `target/automation.sock` (set by `WACKCODE_AUTOMATION` in `.cargo/config.toml`; the path must stay under about 100 characters). Send one JSON object per line and read one back:

```sh
cargo dev &                                    # from the repo root
ask() { echo "$1" | nc -U -w 5 target/automation.sock; }
ask '{"cmd":"tree"}'                           # window size and every tag's bounds
ask '{"cmd":"screenshot","path":"/abs/out.png"}'
ask '{"cmd":"click","tag":"send"}'             # or {"x":10,"y":20}; optional "count", "right"
ask '{"cmd":"type","text":"hello"}'            # into whatever has focus
ask '{"cmd":"key","keys":"cmd-n enter"}'
ask '{"cmd":"resize","width":900,"height":600}'
ask '{"cmd":"appearance","theme":"light"}'
ask '{"cmd":"quit"}'
```

A failure answers `{"ok":false,"error":"…"}` with a sentence saying what went wrong. Screenshots show what the app drew, at 2x, without the traffic lights (the system draws those). The full protocol is in `crates/wack-ui/src/automation.rs`.

The dev app keeps its data in `~/Library/Application Support/com.wackcode.desktop.dev/`, apart from the installed app's `com.wackcode.desktop`. The installed app's folder is the owner's real data: never read or write it.

## Real LLMs

Use only the connection named **Testing (AI Agents may use this too)**, added inside the dev app. Never read its key from `secrets.json`, and never use another connection: it may cost the owner money.
