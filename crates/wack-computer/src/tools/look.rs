//! The tools that find and look: `computer_apps`, `computer_open`, `computer_snapshot`,
//! `computer_screenshot` and `computer_wait`.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;
use wack_agent::ToolOutput;
use wack_llm::Image;

use super::{Call, app_data, window_words};
use crate::{
    backend::{AppInfo, Found, WindowInfo},
    computer::LastShot,
    native::capture::Zoom,
    outline::{self, MAX_LISTED, MAX_OUTLINE_BYTES, Node},
    policy::{self, Tier},
};

/// How long computer_open waits for a started app's first window.
const WINDOW_WAIT: Duration = Duration::from_secs(10);
const WAIT_DEFAULT_MS: u64 = 5000;
const WAIT_MAX_MS: u64 = 30_000;
const WAIT_STEP: Duration = Duration::from_millis(250);
/// A window may differ this much in size from its last picture (its border, rounding) and still
/// count as the same size.
const SIZE_SLACK: f64 = 4.0;

fn parse<T: for<'de> Deserialize<'de>>(input: Value, tool: &str) -> Result<T, String> {
    serde_json::from_value(input).map_err(|error| format!("{tool} didn't understand its input: {error}."))
}

fn parse_ref(raw: &str) -> Result<u32, String> {
    outline::parse_ref(raw).ok_or_else(|| format!("“{raw}” isn't a ref: refs look like e12."))
}

#[derive(Deserialize)]
struct AppsInput {
    #[serde(default)]
    query: Option<String>,
}

pub(super) async fn apps(call: &Call<'_>, input: Value) -> Result<ToolOutput, String> {
    let input: AppsInput = parse(input, "computer_apps")?;
    let desktop = call.chat.desktop();
    let own = desktop.own();
    let query = input.query.as_deref().map(str::trim).filter(|query| !query.is_empty()).map(str::to_lowercase);
    let matches = |name: &str, bundle_id: Option<&str>| {
        query.as_deref().is_none_or(|query| name.to_lowercase().contains(query) || bundle_id.is_some_and(|id| id.to_lowercase().contains(query)))
    };
    let mut running: Vec<AppInfo> = desktop.running().await.into_iter().filter(|app| app.pid != own.pid).filter(|app| matches(&app.identity.name, app.identity.bundle_id.as_deref())).collect();
    running.sort_by(|a, b| b.frontmost.cmp(&a.frontmost).then_with(|| a.identity.name.to_lowercase().cmp(&b.identity.name.to_lowercase())));
    let accessible = desktop.permissions().accessibility;
    let mut lines = Vec::new();
    for app in &running {
        let (state, allowed) = access_word(call, &app.identity);
        let mut line = format!("- {}", app.identity.name);
        if let Some(id) = &app.identity.bundle_id {
            line.push_str(&format!(" ({id})"));
        }
        line.push_str(&format!(": {state}"));
        if app.frontmost {
            line.push_str(", in front");
        }
        if app.hidden {
            line.push_str(", hidden");
        }
        lines.push(line);
        if allowed && accessible {
            match desktop.windows(app.pid).await {
                Ok(windows) if windows.is_empty() => lines.push("    no windows".into()),
                Ok(windows) => lines.extend(windows.iter().map(|window| format!("    {}", window_words(window)))),
                Err(error) => lines.push(format!("    windows unreadable: {error}")),
            }
        }
    }
    let mut text = String::new();
    if lines.is_empty() {
        text.push_str(if query.is_some() { "No running app matches.\n" } else { "No apps are running.\n" });
    } else {
        text.push_str("Running apps, with this chat's access:\n");
        text.push_str(&app_data(&lines.join("\n")));
        text.push('\n');
    }
    if let Some(query) = &query {
        let running_paths: Vec<_> = running.iter().filter_map(|app| app.identity.path.clone()).collect();
        let installed: Vec<String> = desktop
            .installed(query, 10)
            .await
            .into_iter()
            .filter(|app| !app.path.as_ref().is_some_and(|path| running_paths.contains(path)))
            .map(|app| {
                let (state, _) = access_word(call, &app);
                format!("- {}{}: {state}", app.name, app.bundle_id.as_deref().map(|id| format!(" ({id})")).unwrap_or_default())
            })
            .collect();
        if !installed.is_empty() {
            text.push_str("Installed, not running (computer_open starts one):\n");
            text.push_str(&app_data(&installed.join("\n")));
            text.push('\n');
        }
    }
    if !accessible {
        text.push_str("Windows aren't listed: WackCode doesn't have the Accessibility permission. The user can allow it in Settings › Computer use.\n");
    }
    Ok(text.trim_end().to_string().into())
}

/// This chat's access to an app, in a word, and whether it may be looked at now.
fn access_word(call: &Call<'_>, app: &policy::AppIdentity) -> (String, bool) {
    let settings = call.chat.computer().settings();
    if let Err(reason) = policy::assess(app, &call.chat.desktop().own(), &settings.never) {
        return (format!("blocked ({})", reason.trim_end_matches('.')), false);
    }
    match call.chat.access_now(app) {
        Ok(Some(Tier::Full)) => ("allowed".into(), true),
        Ok(Some(Tier::LookOnly)) => ("look-only".into(), true),
        Ok(None) if policy::tier(app) == Tier::LookOnly => ("ask (look-only)".into(), false),
        Ok(None) => ("ask".into(), false),
        Err(_) => ("denied until the user's next message".into(), false),
    }
}

#[derive(Deserialize)]
struct OpenInput {
    app: String,
    #[serde(default)]
    file: Option<String>,
}

/// A file the call may open: inside the chat's folder, after following links.
fn chat_file(cwd: &Path, raw: &str) -> Result<PathBuf, String> {
    let joined = if Path::new(raw).is_absolute() { PathBuf::from(raw) } else { cwd.join(raw) };
    let file = joined.canonicalize().map_err(|_| format!("There is no file at {}.", joined.display()))?;
    let folder = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    if !file.starts_with(&folder) {
        return Err("computer_open opens files from the chat's folder only.".into());
    }
    Ok(file)
}

pub(super) async fn open(call: &Call<'_>, input: Value) -> Result<ToolOutput, String> {
    let input: OpenInput = parse(input, "computer_open")?;
    let desktop = call.chat.desktop();
    let file = input.file.as_deref().map(|raw| chat_file(&call.context.cwd, raw)).transpose()?;
    let (identity, running) = match desktop.resolve(&input.app).await? {
        Found::Running(info) => (info.identity, true),
        Found::Installed(identity) => (identity, false),
    };
    let tier = call.chat.access(&identity, !running, call.present, &call.context.cancel).await?;
    if tier == Tier::LookOnly && (file.is_some() || !running) {
        return Err(format!("{} is look-only: computer use doesn't open it or open files in it. Work on the web with the browser_* tools.", identity.name));
    }
    let name = identity.name.clone();
    let pid = match (running, &file) {
        (true, None) => identity.pid.ok_or("That app's process can't be found.")?,
        _ => desktop.launch(&identity, file.clone()).await?,
    };
    // A started app shows its window a moment after it runs; wait for one on screen.
    let deadline = Instant::now() + WINDOW_WAIT;
    let mut windows: Vec<WindowInfo> = Vec::new();
    loop {
        if let Ok(found) = desktop.windows(pid).await {
            windows = found;
        }
        let shown = windows.iter().any(|window| window.on_screen);
        let opened = file.as_ref().is_none_or(|file| windows.iter().any(|window| file.file_name().is_some_and(|name| window.title.contains(&*name.to_string_lossy()))));
        if (shown && opened) || Instant::now() >= deadline || call.context.cancel.is_cancelled() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let verb = if running { "is running" } else { "started in the background" };
    let mut text = format!("{name} {verb}");
    if let Some(file) = &file {
        text.push_str(&format!(", opening {}", file.display()));
    }
    if windows.is_empty() {
        text.push_str(". It has no window yet.");
    } else {
        text.push_str(". Its windows:\n");
        text.push_str(&app_data(&windows.iter().map(|window| format!("- {}", window_words(window))).collect::<Vec<_>>().join("\n")));
    }
    Ok(text.into())
}

#[derive(Deserialize)]
struct SnapshotInput {
    app: String,
    #[serde(default)]
    window: Option<u32>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default, rename = "ref")]
    reference: Option<String>,
    #[serde(default)]
    menu: Option<Vec<String>>,
}

pub(super) async fn snapshot(call: &Call<'_>, input: Value) -> Result<ToolOutput, String> {
    let input: SnapshotInput = parse(input, "computer_snapshot")?;
    let (app, _) = call.running_app(&input.app).await?;
    let desktop = call.chat.desktop();
    let name = &app.identity.name;
    if let Some(path) = input.menu {
        let items = desktop.menus(app.pid, path.clone()).await?;
        let place = if path.is_empty() { "menu bar".to_string() } else { format!("{} menu", path.join(" › ")) };
        let note = "Items marked › open a submenu; choose one with computer_act's `menu` action.";
        return Ok(format!("{name}'s {place}:\n{}\n{note}", app_data(&items.iter().map(|item| format!("- {item}")).collect::<Vec<_>>().join("\n"))).into());
    }
    let window = call.window(&app, input.window).await?;
    let from = match input.reference.as_deref() {
        Some(raw) => {
            let reference = parse_ref(raw)?;
            if !call.chat.was_shown(app.pid, reference) {
                return Err(format!("e{reference} hasn't been shown to you in {name}: take a computer_snapshot first."));
            }
            Some(reference)
        }
        None => None,
    };
    let tree = desktop.tree(app.pid, window.id, from).await?;
    let outline = outline::render(&tree.root, &mut |handle| handle as u32, input.query.as_deref(), MAX_OUTLINE_BYTES);
    call.chat.saw(app.pid, &outline.shown);
    let mut text = format!("{name} · {}\n", window_words(&window));
    if outline.text.trim().is_empty() {
        text.push_str(if input.query.is_some() { "No line matches the query." } else { "The window shows nothing Accessibility can read: try computer_screenshot." });
    } else {
        text.push_str(&app_data(&outline.text));
    }
    if outline.filtered > 0 {
        text.push_str(&format!("\n{} other elements didn't match.", outline.filtered));
    }
    if outline.truncated {
        text.push_str("\nThe outline was cut short: narrow it with `query` or `ref`.");
    }
    if tree.truncated {
        text.push_str("\nThe app's tree is too large to read whole: outline part of it with `ref`.");
    }
    Ok(text.into())
}

#[derive(Deserialize)]
struct ScreenshotInput {
    app: String,
    #[serde(default)]
    window: Option<u32>,
    #[serde(default)]
    region: Option<[f64; 4]>,
    #[serde(default)]
    scale: Option<f64>,
}

pub(super) async fn screenshot(call: &Call<'_>, input: Value) -> Result<ToolOutput, String> {
    if !call.context.seen.settings.is_none_or(|settings| settings.accepts_images) {
        return Err("This model can't read pictures: use computer_snapshot to read the window.".into());
    }
    let input: ScreenshotInput = parse(input, "computer_screenshot")?;
    let (app, _) = call.running_app(&input.app).await?;
    let desktop = call.chat.desktop();
    let name = &app.identity.name;
    let window = call.window(&app, input.window).await?;
    if window.minimized {
        return Err(format!("{name}'s {} is minimized, so it can't be pictured. computer_snapshot still reads it.", window_words(&window)));
    }
    let scale = input.scale.unwrap_or(1.0).clamp(0.1, 1.0);
    let label = format!("{name} window \"{}\"", outline::clip(&window.title, 60));
    let Some(region) = input.region else {
        let shot = desktop.picture(window.id, scale, None).await?;
        call.chat.remember_shot(app.pid, window.id, LastShot { frame: shot.frame, bounds: shot.bounds, window: window.frame });
        let mut text = format!(
            "{name} · {} · picture {}×{}; x and y refer to a {}×{} frame.",
            window_words(&window),
            shot.picture.0,
            shot.picture.1,
            shot.frame.0,
            shot.frame.1
        );
        match desktop.tree(app.pid, window.id, None).await {
            Ok(tree) => {
                let list = outline::element_list(&tree.root, &mut |handle| handle as u32, shot.bounds, shot.frame, MAX_LISTED);
                call.chat.saw(app.pid, &list.shown);
                if !list.text.is_empty() {
                    text.push_str("\nElements in view:\n");
                    text.push_str(&app_data(&list.text));
                }
            }
            Err(error) => text.push_str(&format!("\nIts elements couldn't be listed: {error}")),
        }
        let data = base64::engine::general_purpose::STANDARD.encode(&shot.jpeg);
        return Ok(ToolOutput::with_images(text, vec![Image::new("image/jpeg", data).labelled(format!("screenshot of the {label}"))]));
    };
    let last = call.chat.last_shot(app.pid, window.id).ok_or_else(|| format!("A region is in the pixels of the window's last full picture: take a full computer_screenshot of {name}'s window first."))?;
    if let Some(frame) = window.frame
        && ((frame.width - last.bounds.width).abs() > SIZE_SLACK || (frame.height - last.bounds.height).abs() > SIZE_SLACK)
    {
        return Err("The window changed size since its last picture: take a new full computer_screenshot.".into());
    }
    let zoom = Zoom { rect: region, frame: last.frame, size: (last.bounds.width, last.bounds.height) };
    let shot = desktop.picture(window.id, scale, Some(zoom)).await?;
    let [x0, y0, x1, y1] = region;
    let text = format!(
        "{name} · {} · zoom into [{x0:.0}, {y0:.0}, {x1:.0}, {y1:.0}], picture {}×{}. Coordinates still refer to the last full picture ({}×{}).",
        window_words(&window),
        shot.picture.0,
        shot.picture.1,
        last.frame.0,
        last.frame.1
    );
    let data = base64::engine::general_purpose::STANDARD.encode(&shot.jpeg);
    Ok(ToolOutput::with_images(text, vec![Image::new("image/jpeg", data).labelled(format!("zoom into the {label}"))]))
}

#[derive(Deserialize)]
struct WaitInput {
    app: String,
    #[serde(default)]
    window: Option<u32>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    gone: bool,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

/// The elements under `root` that match, with their descriptions.
fn matching<'a>(root: &'a Node, text: Option<&str>, role: Option<&str>) -> Vec<&'a Node> {
    let mut found = Vec::new();
    let text = text.map(str::to_lowercase);
    let role = role.map(str::to_lowercase);
    fn walk<'a>(node: &'a Node, text: Option<&str>, role: Option<&str>, found: &mut Vec<&'a Node>) {
        let role_ok = role.is_none_or(|role| outline::display_role(&node.role, node.subrole.as_deref()).to_lowercase() == role || node.role.to_lowercase() == role);
        let text_ok = text.is_none_or(|text| {
            !node.is_secure() && [&node.title, &node.description, &node.value, &node.placeholder].iter().any(|field| field.as_deref().is_some_and(|field| field.to_lowercase().contains(text)))
        });
        if role_ok && text_ok {
            found.push(node);
        }
        for child in &node.children {
            walk(child, text, role, found);
        }
    }
    walk(root, text.as_deref(), role.as_deref(), &mut found);
    found
}

pub(super) async fn wait(call: &Call<'_>, input: Value) -> Result<ToolOutput, String> {
    let input: WaitInput = parse(input, "computer_wait")?;
    let text = input.text.as_deref().map(str::trim).filter(|text| !text.is_empty());
    let role = input.role.as_deref().map(str::trim).filter(|role| !role.is_empty());
    if text.is_none() && role.is_none() {
        return Err("computer_wait needs `text`, `role` or both: what to wait for.".into());
    }
    let (app, _) = call.running_app(&input.app).await?;
    let desktop = call.chat.desktop();
    let name = &app.identity.name;
    let timeout = Duration::from_millis(input.timeout_ms.unwrap_or(WAIT_DEFAULT_MS).clamp(100, WAIT_MAX_MS));
    let started = Instant::now();
    let what = match (text, role) {
        (Some(text), Some(role)) => format!("a {role} with “{text}”"),
        (Some(text), None) => format!("“{text}”"),
        (None, Some(role)) => format!("a {role}"),
        (None, None) => unreachable!("checked above"),
    };
    loop {
        // Window titles count too: a window opening is often what's awaited.
        let windows = desktop.windows(app.pid).await.unwrap_or_default();
        let titled: Vec<&WindowInfo> = match (text, role) {
            (Some(text), None) => windows.iter().filter(|window| window.title.to_lowercase().contains(&text.to_lowercase())).collect(),
            _ => Vec::new(),
        };
        let window = match input.window {
            Some(id) => windows.iter().find(|window| window.id == id).cloned(),
            None => call.window(&app, None).await.ok(),
        };
        let tree = match &window {
            Some(window) => desktop.tree(app.pid, window.id, None).await.ok(),
            None => None,
        };
        let found: Vec<&Node> = tree.as_ref().map(|tree| matching(&tree.root, text, role)).unwrap_or_default();
        let present = !found.is_empty() || !titled.is_empty();
        let waited = started.elapsed().as_millis();
        if present != input.gone {
            if input.gone {
                return Ok(format!("{what} is gone from {name} after {waited} ms.").into());
            }
            let mut lines: Vec<String> = titled.iter().map(|window| window_words(window)).collect();
            let shown: Vec<u32> = found.iter().take(10).map(|node| node.handle as u32).collect();
            call.chat.saw(app.pid, &shown);
            lines.extend(found.iter().take(10).map(|node| format!("[e{}] {}", node.handle, outline::describe(node))));
            if found.len() > 10 {
                lines.push(format!("… {} more", found.len() - 10));
            }
            return Ok(format!("{what} showed in {name} after {waited} ms:\n{}", app_data(&lines.join("\n"))).into());
        }
        if started.elapsed() >= timeout {
            let state = if input.gone { "is still there" } else { "hasn't shown" };
            return Err(format!("{what} {state} in {name} after {} ms.", timeout.as_millis()));
        }
        tokio::select! {
            () = tokio::time::sleep(WAIT_STEP) => {}
            () = call.context.cancel.cancelled() => return Err("Stopped.".into()),
        }
    }
}
