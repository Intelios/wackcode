//! `screen: true`: full-screen control for `computer_act` and `computer_screenshot`.
//!
//! Invariants:
//! - Acting asks for the screen first (`ChatComputer::screen_access`: a card the first time in a
//!   run); a screen picture doesn't, since it shows only the apps the chat may use already.
//! - The app is brought to the front before anything is posted, and every event is gated
//!   ([`go`]): the screen still held, no password field with the keyboard, the point on the
//!   app's own window (or, for keys and typing, the app still in front).

use std::{collections::HashMap, sync::Arc, time::Duration};

use base64::Engine as _;
use wack_agent::ToolOutput;
use wack_llm::Image;

use super::{Call, SCREEN_PID};
use crate::{
    backend::{Act, AppInfo, Done, Target, Via, WindowInfo},
    computer::LastShot,
    keys,
    ladder::{self, Step},
    outline::Node,
    screen::{self, Go, ScreenInput},
};

/// How long the app gets to come to the front.
const FRONT_LIMIT: Duration = Duration::from_millis(1500);

/// The running apps the chat may use (allowed or look-only), never WackCode.
pub(super) async fn allowed_apps(call: &Call<'_>) -> Vec<AppInfo> {
    let own = call.chat.desktop().own().pid;
    call.chat.desktop().running().await.into_iter().filter(|app| app.pid != own && matches!(call.chat.access_now(&app.identity), Ok(Some(_)))).collect()
}

/// A picture of the display, showing only the apps the chat may use.
pub(super) async fn picture(call: &Call<'_>, near: Option<(f64, f64)>, scale: f64) -> Result<ToolOutput, String> {
    let apps = allowed_apps(call).await;
    if apps.is_empty() {
        return Err("No app this chat may use is running, so a picture of the screen would be empty. Allow an app first (computer_apps).".into());
    }
    let shot = call.chat.desktop().screen_picture(near, apps.iter().map(|app| app.pid).collect(), scale).await?;
    call.chat.remember_shot(SCREEN_PID, 0, LastShot { frame: shot.frame, bounds: shot.bounds, window: None });
    let names: Vec<&str> = apps.iter().map(|app| app.identity.name.as_str()).collect();
    let text = format!(
        "The screen ({:.0}×{:.0} points), showing only the apps this chat may use: {}. Everything else is left out. Picture {}×{}; with `screen: true`, computer_act's x and y refer to this {}×{} frame.",
        shot.bounds.width,
        shot.bounds.height,
        names.join(", "),
        shot.picture.0,
        shot.picture.1,
        shot.frame.0,
        shot.frame.1
    );
    let data = base64::engine::general_purpose::STANDARD.encode(&shot.jpeg);
    Ok(ToolOutput::with_images(text, vec![Image::new("image/jpeg", data).labelled("screenshot of the screen")]))
}

/// A point in the last screen picture, as a global point.
pub(super) fn picture_point(call: &Call<'_>, x: f64, y: f64) -> Result<(f64, f64), String> {
    let shot = call.chat.last_shot(SCREEN_PID, 0).ok_or("With `screen: true`, x and y are in the last picture of the screen: take a computer_screenshot with `screen: true` first, or act by ref.")?;
    let (lx, ly) = crate::geometry::to_window((shot.bounds.width, shot.bounds.height), shot.frame, x, y)
        .ok_or_else(|| format!("({x}, {y}) is outside the picture of the screen, which is {}×{}.", shot.frame.0, shot.frame.1))?;
    Ok((shot.bounds.x + lx, shot.bounds.y + ly))
}

/// The middle of the element `target` names, as a global point.
async fn point_of(call: &Call<'_>, app: &AppInfo, window: &WindowInfo, target: &Target, tree: Option<&Node>) -> Result<(f64, f64), String> {
    let middle = |node: &Node| node.frame.filter(|frame| frame.has_area()).map(|frame| frame.center());
    match target {
        Target::At { point, .. } => Ok(*point),
        Target::Ref(reference) => {
            if let Some(point) = tree.and_then(|tree| tree.find(*reference as usize)).and_then(middle) {
                return Ok(point);
            }
            let tree = call.chat.desktop().tree(app.pid, window.id, Some(*reference)).await?;
            middle(&tree.root).ok_or_else(|| format!("e{reference} has no place on screen to point at."))
        }
        Target::Focused => {
            let mut found = None;
            if let Some(tree) = tree {
                tree.each(&mut |node| {
                    if node.focused && found.is_none() {
                        found = middle(node);
                    }
                });
            }
            found.ok_or_else(|| "Nothing with keyboard focus has a place on screen: aim with a ref or x and y.".to_string())
        }
    }
}

/// The gate asked before every event.
fn go(call: &Call<'_>, app: &AppInfo, point: Option<(f64, f64)>, names: HashMap<i32, String>) -> Go {
    let chat = call.chat.clone();
    let cancel = call.context.cancel.clone();
    let desktop = call.chat.desktop().clone();
    let (pid, own, name) = (app.pid, desktop.own().pid, app.identity.name.clone());
    Arc::new(move || {
        if cancel.is_cancelled() {
            return Err("The run was stopped.".into());
        }
        if !chat.holds_screen() {
            return Err("You don't have the screen any more: the user took it back, or the run ended.".into());
        }
        if desktop.secure_input() {
            return Err("A password field has the keyboard: ask the user to type it.".into());
        }
        match point {
            Some(point) => screen::gate_point(&desktop.screen_windows(), point, pid, own, &name, &names),
            None if desktop.frontmost() != Some(pid) => Err(format!("{name} isn't in front any more.")),
            None => Ok(()),
        }
    })
}

/// Does `act` in `app` with the real pointer and keyboard, taking the screen first.
pub(super) async fn perform(call: &Call<'_>, app: &AppInfo, window: &WindowInfo, act: Act, reason: &str, tree: Option<&Node>) -> Result<Done, String> {
    let desktop = call.chat.desktop().clone();
    let name = app.identity.name.clone();
    let running = desktop.running().await;
    let allowed: Vec<String> = running.iter().filter(|other| matches!(call.chat.access_now(&other.identity), Ok(Some(_)))).map(|other| other.identity.name.clone()).collect();
    call.chat.screen_access(reason, allowed, call.present, &call.context.cancel).await?;
    // In front, the app takes its document's commands and their shortcuts.
    if desktop.frontmost() != Some(app.pid) {
        desktop.bring_forward(app.pid).await;
        let started = std::time::Instant::now();
        while desktop.frontmost() != Some(app.pid) {
            if started.elapsed() > FRONT_LIMIT {
                return Err(format!("{name} wouldn't come to the front."));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    let names: HashMap<i32, String> = running.iter().map(|other| (other.pid, other.identity.name.clone())).collect();
    let (input, what) = match act {
        // Accessibility does these, now that the app is in front.
        Act::Menu { .. } | Act::Focus { .. } => return desktop.act(app.pid, window.id, act).await.map_err(|reason| ladder::advice(Step::Click, &reason, &name)),
        Act::Click { target, button, count, modifiers } => {
            let point = point_of(call, app, window, &target, tree).await?;
            (ScreenInput::Click { point, button, count, modifiers }, format!("clicked at ({:.0}, {:.0}) with the pointer", point.0, point.1))
        }
        Act::Scroll { target, dx, dy } => {
            let point = point_of(call, app, window, &target, tree).await?;
            (ScreenInput::Scroll { point, dx, dy }, "scrolled with the pointer".to_string())
        }
        Act::Drag { path } => (ScreenInput::Drag { path }, "dragged with the pointer".to_string()),
        Act::Key { chord, times } => (ScreenInput::Key { chord, times }, "pressed the keys".to_string()),
        Act::Type { target, text, replace } => {
            // Click into the field first, then select what is there if it is to be replaced.
            if !matches!(target, Target::Focused) {
                let point = point_of(call, app, window, &target, tree).await?;
                run(call, app, &ScreenInput::Click { point, button: crate::backend::Button::Left, count: 1, modifiers: keys::Modifiers::default() }, &names).await?;
            }
            if replace {
                let chord = keys::parse_chord("cmd+a", &desktop.layout())?;
                run(call, app, &ScreenInput::Key { chord, times: 1 }, &names).await?;
            }
            let count = text.chars().count();
            (ScreenInput::Type { text }, format!("typed {count} characters with the keyboard"))
        }
    };
    run(call, app, &input, &names).await?;
    Ok(Done { what, via: Via::Raw, note: Some("Done with the real pointer and keyboard: check the result.".into()), replaced: None })
}

/// Posts one input after checking where it lands.
async fn run(call: &Call<'_>, app: &AppInfo, input: &ScreenInput, names: &HashMap<i32, String>) -> Result<(), String> {
    let point = input.point();
    let gate = go(call, app, point.filter(|_| !matches!(input, ScreenInput::Drag { .. })), names.clone());
    // A drag checks where it starts; after that it may cross other windows on purpose.
    if let (ScreenInput::Drag { .. }, Some(start)) = (input, point) {
        let own = call.chat.desktop().own().pid;
        screen::gate_point(&call.chat.desktop().screen_windows(), start, app.pid, own, &app.identity.name, names)?;
    }
    call.chat.desktop().screen_input(input.clone(), gate).await
}
