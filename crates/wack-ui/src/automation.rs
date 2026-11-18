//! The dev-only automation socket: lets a test or an agent look at the running app and drive it.
//!
//! It exists only in a build with the `automation` feature (`cargo dev`), it can never be part of a
//! release build (see the `compile_error!` below), and it listens only when `WACKCODE_AUTOMATION`
//! names a socket path. See docs/security.md and docs/testing.md.
//!
//! The protocol is one JSON object per line in, one JSON object per line out:
//!
//! ```text
//! {"cmd":"tree"}                                  the window size and every tagged element's bounds
//! {"cmd":"screenshot","path":"/abs/file.png"}     writes what the window is showing
//! {"cmd":"click","tag":"send"}                    also {"x":10,"y":20}; optional "count", "right"
//! {"cmd":"drag","x":10,"y":20,"to_x":300,"to_y":20}   presses, moves in steps, releases
//! {"cmd":"type","text":"hello"}                   types into whatever has focus
//! {"cmd":"key","keys":"cmd-n enter"}              space-separated keystrokes
//! {"cmd":"resize","width":900,"height":600}
//! {"cmd":"appearance","theme":"light"}            or "dark"
//! {"cmd":"browser","action":"open","url":"https://example.com"}   a tab of the chat in front, as the agent
//! {"cmd":"browser","action":"eval","expression":"document.title"}   in the agent's newest tab, or "tab"
//! {"cmd":"browser","action":"screenshot","path":"/abs/page.png"}   the page itself, through DevTools
//! {"cmd":"browser","action":"state"}                     every tab's address, title, console and network
//! {"cmd":"browser","action":"dialog","accept":true,"text":"reply"}   answers the page's dialog
//! {"cmd":"browser","action":"close"}
//! {"cmd":"quit"}
//! ```
//!
//! Every answer has `"ok"`; a failure also has `"error"`, a sentence saying what went wrong.

#[cfg(not(debug_assertions))]
compile_error!("The `automation` feature must never be enabled in a release build.");

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};

use gpui::{
    AnyWindowHandle, App, AppContext, AsyncApp, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput, Point, Window, point, px, size,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    tags,
    theme::{Appearance, Theme},
};

/// The environment variable that names the socket path. Unset means no socket.
pub const SOCKET_ENV: &str = "WACKCODE_AUTOMATION";

/// How long to let the app redraw before reading the screen or answering.
const SETTLE: Duration = Duration::from_millis(120);

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Command {
    Tree,
    Screenshot {
        path: PathBuf,
    },
    Click {
        tag: Option<String>,
        x: Option<f32>,
        y: Option<f32>,
        #[serde(default = "one")]
        count: usize,
        #[serde(default)]
        right: bool,
    },
    Drag {
        x: f32,
        y: f32,
        to_x: f32,
        to_y: f32,
    },
    Type {
        text: String,
    },
    Key {
        keys: String,
    },
    Resize {
        width: f32,
        height: f32,
    },
    Appearance {
        theme: String,
    },
    Browser(BrowserArgs),
    Quit,
}

/// `{"cmd":"browser",…}`: what to do, and the fields that action reads.
#[derive(Deserialize)]
struct BrowserArgs {
    action: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    expression: Option<String>,
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default)]
    tab: Option<u64>,
    #[serde(default)]
    accept: bool,
    #[serde(default)]
    text: Option<String>,
}

fn one() -> usize {
    1
}

type Request = (Command, mpsc::Sender<Value>);

/// Opens the socket if `WACKCODE_AUTOMATION` asks for one.
pub fn start(cx: &mut App) {
    let Some(path) = std::env::var_os(SOCKET_ENV).map(PathBuf::from) else {
        return;
    };
    let (requests, incoming) = async_channel::unbounded::<Request>();
    if let Err(message) = listen(path.clone(), requests) {
        eprintln!("{message}");
        return;
    }
    eprintln!("Automation socket: {}", path.display());

    cx.spawn(async move |cx| {
        while let Ok((command, reply)) = incoming.recv().await {
            let answer = match run(command, cx).await {
                Ok(answer) => answer,
                Err(message) => json!({ "ok": false, "error": message }),
            };
            reply.send(answer).ok();
        }
    })
    .detach();
}

fn listen(path: PathBuf, requests: async_channel::Sender<Request>) -> Result<(), String> {
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|error| {
        format!(
            "Could not open the automation socket at {}: {error}",
            path.display()
        )
    })?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("Could not make the automation socket private: {error}"))?;
    std::thread::Builder::new()
        .name("automation".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream, &requests);
            }
        })
        .map_err(|error| format!("Could not start the automation thread: {error}"))?;
    Ok(())
}

fn serve(stream: UnixStream, requests: &async_channel::Sender<Request>) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Command>(&line) {
            Ok(command) => {
                let (reply, answer) = mpsc::channel();
                if requests.send_blocking((command, reply)).is_err() {
                    break;
                }
                answer.recv().unwrap_or_else(
                    |_| json!({ "ok": false, "error": "The app closed before answering." }),
                )
            }
            Err(error) => json!({ "ok": false, "error": format!("Not a valid command: {error}") }),
        };
        if writeln!(writer, "{answer}").is_err() {
            break;
        }
    }
}

async fn run(command: Command, cx: &mut AsyncApp) -> Result<Value, String> {
    let window = cx
        .update(|cx| cx.windows().first().copied())
        .ok_or("WackCode has no window open.")?;
    match command {
        Command::Tree => {
            settle(window, cx).await;
            on_window(window, cx, |window, cx| {
                let viewport = window.viewport_size();
                let tags: Vec<Value> = tags::tags(window, cx)
                    .into_iter()
                    .map(|tag| {
                        json!({
                            "name": tag.name,
                            "x": f32::from(tag.bounds.origin.x),
                            "y": f32::from(tag.bounds.origin.y),
                            "width": f32::from(tag.bounds.size.width),
                            "height": f32::from(tag.bounds.size.height),
                        })
                    })
                    .collect();
                json!({
                    "ok": true,
                    "window": {
                        "width": f32::from(viewport.width),
                        "height": f32::from(viewport.height),
                        "scale": window.scale_factor(),
                    },
                    "tags": tags,
                })
            })
        }
        Command::Screenshot { path } => {
            if !path.is_absolute() {
                return Err("The screenshot path must be absolute.".into());
            }
            settle(window, cx).await;
            let image = on_window(window, cx, |window, _| window.render_to_image())?
                .map_err(|error| format!("Could not capture the window: {error}"))?;
            image
                .save(&path)
                .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
            Ok(
                json!({ "ok": true, "path": path, "width": image.width(), "height": image.height() }),
            )
        }
        Command::Click {
            tag,
            x,
            y,
            count,
            right,
        } => {
            let position = match (tag, x, y) {
                (Some(name), _, _) => on_window(window, cx, |window, cx| {
                    let tags = tags::tags(window, cx);
                    tags.iter()
                        .rev()
                        .find(|tag| tag.name == name)
                        .map(|tag| tag.bounds.center())
                        .ok_or_else(|| {
                            let names: Vec<&str> = tags.iter().map(|tag| tag.name).collect();
                            format!(
                                "Nothing on screen is tagged \"{name}\". Tags on screen: {}.",
                                names.join(", ")
                            )
                        })
                })??,
                (None, Some(x), Some(y)) => point(px(x), px(y)),
                _ => return Err("A click needs a tag, or both x and y.".into()),
            };
            let button = if right {
                MouseButton::Right
            } else {
                MouseButton::Left
            };
            on_window(window, cx, |window, cx| {
                click(position, button, count, window, cx)
            })?;
            settle(window, cx).await;
            Ok(json!({ "ok": true, "x": f32::from(position.x), "y": f32::from(position.y) }))
        }
        Command::Drag { x, y, to_x, to_y } => {
            let modifiers = Modifiers::default();
            let at = |step: usize| point(px(x + (to_x - x) * step as f32 / DRAG_STEPS as f32), px(y + (to_y - y) * step as f32 / DRAG_STEPS as f32));
            on_window(window, cx, |window, cx| {
                window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: at(0), pressed_button: None, modifiers }), cx);
                window.dispatch_event(
                    PlatformInput::MouseDown(MouseDownEvent {
                        button: MouseButton::Left,
                        position: at(0),
                        modifiers,
                        click_count: 1,
                        first_mouse: false,
                    }),
                    cx,
                );
            })?;
            // Each step is its own frame, as a real drag is: handlers drawn for one frame see the
            // next move.
            for step in 1..=DRAG_STEPS {
                settle_briefly(window, cx).await;
                on_window(window, cx, |window, cx| {
                    window.dispatch_event(
                        PlatformInput::MouseMove(MouseMoveEvent { position: at(step), pressed_button: Some(MouseButton::Left), modifiers }),
                        cx,
                    );
                })?;
            }
            settle_briefly(window, cx).await;
            on_window(window, cx, |window, cx| {
                window.dispatch_event(
                    PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: at(DRAG_STEPS), modifiers, click_count: 1 }),
                    cx,
                );
            })?;
            settle(window, cx).await;
            Ok(json!({ "ok": true }))
        }
        Command::Type { text } => {
            on_window(window, cx, |window, cx| {
                for character in text.chars() {
                    let key = character.to_string();
                    let keystroke = Keystroke {
                        modifiers: Modifiers::default(),
                        key: key.clone(),
                        key_char: Some(key),
                    };
                    window.dispatch_keystroke(keystroke, cx);
                }
            })?;
            settle(window, cx).await;
            Ok(json!({ "ok": true }))
        }
        Command::Key { keys } => {
            let keystrokes = keys
                .split_whitespace()
                .map(|key| {
                    Keystroke::parse(key)
                        .map_err(|error| format!("\"{key}\" is not a keystroke: {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            on_window(window, cx, |window, cx| {
                for keystroke in keystrokes {
                    window.dispatch_keystroke(keystroke, cx);
                }
            })?;
            settle(window, cx).await;
            Ok(json!({ "ok": true }))
        }
        Command::Resize { width, height } => {
            on_window(window, cx, |window, _| {
                window.resize(size(px(width), px(height)))
            })?;
            settle(window, cx).await;
            Ok(json!({ "ok": true }))
        }
        Command::Appearance { theme } => {
            let appearance = match theme.as_str() {
                "light" => Appearance::Light,
                "dark" => Appearance::Dark,
                other => return Err(format!("Unknown theme \"{other}\": use light or dark.")),
            };
            cx.update(|cx| Theme::set_appearance(appearance, cx));
            settle(window, cx).await;
            Ok(json!({ "ok": true }))
        }
        Command::Browser(args) => browser(window, cx, args).await,
        Command::Quit => {
            cx.update(|cx| cx.quit());
            Ok(json!({ "ok": true }))
        }
    }
}

fn click(
    position: Point<Pixels>,
    button: MouseButton,
    count: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let modifiers = Modifiers::default();
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            button,
            position,
            modifiers,
            click_count: count,
            first_mouse: false,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            button,
            position,
            modifiers,
            click_count: count,
        }),
        cx,
    );
}

fn on_window<R>(
    window: AnyWindowHandle,
    cx: &mut AsyncApp,
    update: impl FnOnce(&mut Window, &mut App) -> R,
) -> Result<R, String> {
    cx.update(|cx| cx.update_window(window, |_, window, cx| update(window, cx)))
        .map_err(|_| "The window closed.".to_string())
}

/// How many moves a drag is made of.
const DRAG_STEPS: usize = 8;

async fn settle_briefly(window: AnyWindowHandle, cx: &mut AsyncApp) {
    on_window(window, cx, |window, _| window.refresh()).ok();
    cx.background_executor().timer(Duration::from_millis(30)).await;
}

/// Asks for a redraw, then waits for it, so what is read next reflects what was just done.
async fn settle(window: AnyWindowHandle, cx: &mut AsyncApp) {
    on_window(window, cx, |window, _| window.refresh()).ok();
    cx.background_executor().timer(SETTLE).await;
}

/// Drives the engine as the agent would, before the Browser pane and the tools exist to.
async fn browser(window: AnyWindowHandle, cx: &mut AsyncApp, args: BrowserArgs) -> Result<Value, String> {
    let BrowserArgs { action, url, expression, path, tab, accept, text } = args;
    use base64::Engine as _;
    use wack_browser::{ChatTabs, Owner};

    let tabs: ChatTabs = on_window(window, cx, |window, cx| {
        window.root::<crate::shell::Shell>().flatten().and_then(|shell| shell.read(cx).current_tabs(cx))
    })?
    .ok_or("Open a chat first: browser tabs belong to a chat.")?;
    let target = || tab.or_else(|| tabs.latest_of(&Owner::Agent).map(|tab| tab.id)).ok_or("No agent tab is open.");
    match action.as_str() {
        "open" => {
            let url = url.ok_or("\"open\" needs a \"url\".")?;
            let opened = tabs.open(Owner::Agent, &url).await?;
            opened.set_agent_driving(true);
            Ok(json!({ "ok": true, "tab": opened.id }))
        }
        "eval" => {
            let expression = expression.ok_or("\"eval\" needs an \"expression\".")?;
            let params = json!({ "expression": expression, "returnByValue": true, "awaitPromise": true, "userGesture": true });
            let result = tabs.cdp(target()?, "Runtime.evaluate", params).await?;
            Ok(json!({ "ok": true, "result": result }))
        }
        "screenshot" => {
            let path = path.ok_or("\"screenshot\" needs a \"path\".")?;
            if !path.is_absolute() {
                return Err("The screenshot path must be absolute.".into());
            }
            let started = std::time::Instant::now();
            let shot = tabs.cdp(target()?, "Page.captureScreenshot", json!({ "format": "png" })).await?;
            let data = shot["data"].as_str().ok_or("The page sent no image.")?;
            let png = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| format!("The image was damaged: {e}"))?;
            std::fs::write(&path, &png).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
            Ok(json!({ "ok": true, "path": path, "bytes": png.len(), "ms": started.elapsed().as_millis() as u64 }))
        }
        "state" => {
            let list: Vec<Value> = tabs
                .tabs()
                .iter()
                .map(|tab| {
                    let state = tab.state();
                    json!({
                        "tab": tab.id,
                        "owner": format!("{:?}", tab.owner),
                        "live": tab.is_live(),
                        "url": state.url,
                        "title": state.title,
                        "loading": state.loading,
                        "page": state.page,
                        "dialog": state.dialog.as_ref().map(|d| d.message.clone()),
                        "load_error": state.load_error,
                        "refused": state.refused,
                        "console": state.console.iter().map(|(_, e)| format!("[{}] {}", e.level.name(), e.text)).collect::<Vec<_>>(),
                        "network": state.network.iter().map(|(_, e)| format!("{} {} {:?} {:?}", e.method, e.url, e.status, e.failed)).collect::<Vec<_>>(),
                    })
                })
                .collect();
            Ok(json!({ "ok": true, "tabs": list }))
        }
        "dialog" => {
            tabs.answer_dialog(target()?, accept, text).await?;
            Ok(json!({ "ok": true }))
        }
        "close" => {
            tabs.close(target()?);
            Ok(json!({ "ok": true }))
        }
        other => Err(format!("Unknown browser action \"{other}\": use open, eval, screenshot, state, dialog or close.")),
    }
}
