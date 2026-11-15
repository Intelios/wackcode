# Checking your work

## Tests

`cargo test --workspace` runs everything. Pure logic is tested in inline `#[cfg(test)]` modules next to the code, with `tempfile` for anything that touches disk.

## The gallery: looking at UI without the app

Every view state worth looking at is a **scene** in `crates/wack-ui/src/scenes.rs`: a view, in a known state, at a known size. The `gallery` binary renders scenes to PNG files.

```sh
cargo snap --list                              # what scenes exist
cargo snap chat --theme dark                   # writes target/gallery/chat-dark.png
cargo snap chat --theme light --out /tmp/a.png
cargo snap chat --look nebula                  # in one of the ten themes (WackCode by default)
cargo snap chat --look all --theme light       # in every theme: target/gallery/chat-<theme>-light.png
cargo snap --all                               # every scene, light and dark
```

Scenes render from sample data in `crates/wack-ui/src/fixtures.rs`: a believable session, so lengths and rhythm are real. The `specimen` scene shows every text style and colour token in the theme it is rendered with. A view must look right in every theme, so try at least one vivid theme (Nebula, Solar) and Mono as well as the default, and a sweeping one (Lagoon, Verdant, Nebula, Solar) for anything filled with the accent.

Rendering is headless: GPUI's test platform with the real Metal renderer and the real macOS text system. No window opens and nothing takes focus, so it works from any shell. Images are rendered at 2x. The gallery draws a scene twice so pictures loaded by the first frame show, and renders with Reduce Motion on, so every movement is shown where it comes to rest.

Backdrop scenes (`*-backdrop*`) draw over a picture that `fixtures::backdrop` imports once into `target/gallery/backdrop/`: a real photo saved as `target/gallery/fixture-photo.jpg` if you put one there (`sips -s format jpeg <any picture> --out target/gallery/fixture-photo.jpg`), otherwise a generated dusk landscape. A real photo shows far better what the owner will see. The owner usually runs with a backdrop, so any view should be checked over one too.

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
ask '{"cmd":"drag","x":716,"y":250,"to_x":560,"to_y":250}'   # a dock edge, a divider, a pane tab
ask '{"cmd":"type","text":"hello"}'            # into whatever has focus
ask '{"cmd":"key","keys":"cmd-n enter"}'
ask '{"cmd":"resize","width":900,"height":600}'
ask '{"cmd":"appearance","theme":"light"}'
ask '{"cmd":"quit"}'
```

A failure answers `{"ok":false,"error":"…"}` with a sentence saying what went wrong. Screenshots show what the app drew, at 2x, without the traffic lights (the system draws those). The full protocol is in `crates/wack-ui/src/automation.rs`.

The dev app keeps its data in `~/Library/Application Support/com.wackcode.desktop.dev/`, apart from the installed app's `com.wackcode.desktop`. The installed app's folder is the owner's real data: never read or write it.

## The agent without the app

Agent behaviour is tested with a scripted provider (`wack_llm::testing::ScriptedProvider`): a test lists the replies the model will give, runs the loop, and checks the requests it sent and the session it left. See the tests in `crates/wack-agent/src/agent.rs`. A new loop behaviour gets a test there.

`cargo wack` runs the same agent from a shell, with the seven core tools, against a real or local provider:

```sh
cargo wack connect                             # asks for name, type, address, model; the key is typed hidden
cargo wack connections
cargo wack run "what does this project do?"    # works in the current folder
cargo wack run --connection "Name" --model id --effort high --cwd ../scratch "…"
cargo wack run --resume <session file> "carry on"
cargo wack run --mode plan "…"                 # or ultraplan; build is the default
cargo wack run --effort ultra "…"
cargo wack --data-dir /tmp/x connect …         # a throwaway data folder, for a local mock server
```

A debug build shares the dev app's data folder, so a connection added here is there for `cargo dev` too. While a run is going, a typed line is sent to the agent as a message, and Ctrl-C stops the run. When the agent asks a question, the next typed line answers it: an option's number (`2`, or `1,3` when it takes several), your own words, `w` to have Ultra Plan write its plan now, or an empty line to dismiss it. With input piped in, nobody can answer and the agent is told so. A submitted plan is printed when the run ends. The tools really run: use a scratch folder for anything that edits files.

## Real LLMs

Use only the connection named **Testing (AI Agents may use this too)**. The owner adds it, in the dev app or with `cargo wack connect`; never copy it from another folder. Never read its key from `secrets.json`, and never use another connection: it may cost the owner money.

```sh
cargo wack run --connection "Testing (AI Agents may use this too)" --cwd <scratch folder> "…"
```
