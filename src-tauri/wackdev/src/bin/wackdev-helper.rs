//! The dev-app driver's engine, run by `scripts/wackdev/server.mjs` (the MCP server agents
//! call). It speaks one JSON object per line on stdio: `{ "id": n, "cmd": "...", ... }` in,
//! `{ "id": n, "ok": true, "result": … }` or `{ "id": n, "ok": false, "error": "…" }` out.
//!
//! Everything runs on this one thread: `AXUIElement` handles are not `Send`, and refs are only
//! element indexes, so the process must stay alive between calls. The Node side owns process
//! lifecycle, MCP and `pnpm dev:background`/`dev:stop`; this binary never starts, stops or even
//! names another app — `wackdev::driver::target` resolves the dev app, or it refuses.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use objc2_app_kit::NSApplication;
use objc2_core_foundation::CGPoint;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};
use wackdev::ax::{self, Element};
use wackdev::driver::{refs::Refs, target};
use wackdev::{capture, geometry::Rect, input, keys, outline, permissions};

/// The helper's own runtime for the one async call (`capture::window`); everything else is
/// synchronous Accessibility on this thread.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_time().build().expect("tokio runtime")
}

struct State {
    refs: Refs,
    rt: tokio::runtime::Runtime,
}

/// The dev app's window titled `DEV_WINDOW_TITLE` (the unnamed helper window never matches),
/// with its window-server number for captures.
fn dev_window(pid: i32) -> Result<(Element, u32), String> {
    // No `AXManualAccessibility`: it's for Electron/Chromium, and setting it on the dev app's
    // WKWebView removes its own windows from AXWindows until the app restarts.
    let app = ax::application(pid);
    let windows = ax::windows(&app);
    let named: Vec<&ax::AxWindow> = windows.iter().filter(|window| window.title == target::DEV_WINDOW_TITLE).collect();
    match named.as_slice() {
        [] => {
            let titles = windows.iter().map(|window| format!("“{}”", window.title)).collect::<Vec<_>>();
            Err(if windows.is_empty() {
                "The dev app has no windows yet; give it a moment, then try again.".into()
            } else {
                format!("The dev app has no window named “{}” (found {}).", target::DEV_WINDOW_TITLE, titles.join(", "))
            })
        }
        [window] => {
            let number = input::window_number(&window.element, pid).ok_or("The dev window can't be identified for capture.")?;
            Ok((window.element.clone(), number))
        }
        several => Err(format!("{} windows are named “{}”; close the extras.", several.len(), target::DEV_WINDOW_TITLE)),
    }
}

/// The window element every element command works on.
fn window_element() -> Result<Element, String> {
    let target = target::resolve()?;
    Ok(dev_window(target.pid)?.0)
}

/// Matches `name` against a node's title, then description, then placeholder.
fn node_name(node: &outline::Node) -> Option<&str> {
    node.title
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .or_else(|| node.description.as_deref().map(str::trim).filter(|text| !text.is_empty()))
        .or_else(|| node.placeholder.as_deref().map(str::trim).filter(|text| !text.is_empty()))
}

fn role_matches(node: &outline::Node, role: &str) -> bool {
    let wanted = role.strip_prefix("AX").unwrap_or(role);
    [Some(node.role.as_str()), node.subrole.as_deref()]
        .into_iter()
        .flatten()
        .any(|role| role.strip_prefix("AX").unwrap_or(role).eq_ignore_ascii_case(wanted))
}

/// Appends `(display role, element)` for nodes whose name matches — exact or case-insensitive
/// contains — optionally constrained by `role` (a node's subrole also answers).
fn collect_matches(node: &outline::Node, elements: &[Element], name: &str, role: Option<&str>, exact: bool, out: &mut Vec<(String, Element)>) {
    let needle = name.to_lowercase();
    let named = node_name(node).is_some_and(|text| if exact { text == name } else { text.to_lowercase().contains(&needle) });
    if named && role.is_none_or(|role| role_matches(node, role)) {
        out.push((node.subrole.as_deref().unwrap_or(&node.role).strip_prefix("AX").unwrap_or(&node.role).to_string(), elements[node.handle].clone()));
    }
    for child in &node.children {
        collect_matches(child, elements, name, role, exact, out);
    }
}

/// Walks `root` for `name`: exact matches win; when there are none, contains-matches.
fn matches_in(root: &Element, name: &str, role: Option<&str>) -> Vec<(String, Element)> {
    let (tree, elements) = ax::walk(root, &ax::SNAPSHOT_LIMITS);
    let mut exact = Vec::new();
    collect_matches(&tree, &elements, name, role, true, &mut exact);
    if !exact.is_empty() {
        return exact;
    }
    let mut partial = Vec::new();
    collect_matches(&tree, &elements, name, role, false, &mut partial);
    partial
}

/// Resolves `ref` or `name` (+ `role`) to one element. Name lookups walk fresh; ambiguity
/// registers the candidates in the current generation so the error's "use a ref: e4-2, e7-9"
/// points at elements the agent can actually act on.
fn element(state: &mut State, args: &Value) -> Result<Element, String> {
    if let Some(reference) = args.get("ref").and_then(Value::as_str) {
        return state.refs.resolve(reference);
    }
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .ok_or("Give a `ref` from `tree`, or a `name` (optionally with `role`).")?;
    let role = args.get("role").and_then(Value::as_str);
    let root = window_element()?;
    let matches = matches_in(&root, name, role);
    match matches.len() {
        0 => Err(format!(
            "Nothing named “{name}”{} is in the dev window. Call `tree` to see what is.",
            role.map(|role| format!(" with role {role}")).unwrap_or_default()
        )),
        1 => Ok(matches.into_iter().next().expect("one match").1),
        count => {
            let refs = state.refs.offer(matches.iter().map(|(_, element)| element.clone()).collect());
            let roles = matches.iter().map(|(role, _)| role.clone()).collect::<Vec<_>>();
            Err(format!("{count} controls are named “{name}” ({}); use a ref: {}.", roles.join(", "), refs.join(", ")))
        }
    }
}

fn cmd_status() -> Value {
    let accessibility = permissions::accessibility();
    let screen_recording = permissions::is_supported() && permissions::screen_recording_preflight();
    match target::resolve() {
        Ok(target) => {
            let window = dev_window(target.pid).ok().map(|(element, number)| {
                json!({
                    "title": target::DEV_WINDOW_TITLE,
                    "number": number,
                    "frame": ax::frame(&element).map(|f| json!({"x": f.x, "y": f.y, "w": f.width, "h": f.height})),
                })
            });
            json!({
                "running": true,
                "pid": target.pid,
                "executable": target.executable,
                "window": window,
                "permissions": { "accessibility": accessibility, "screenRecording": screen_recording },
            })
        }
        Err(reason) => json!({
            "running": false,
            "reason": reason,
            "permissions": { "accessibility": accessibility, "screenRecording": screen_recording },
        }),
    }
}

fn cmd_tree(state: &mut State, args: &Value) -> Result<Value, String> {
    let root = match args.get("within").and_then(Value::as_str) {
        Some(within) => element(state, &json!({ "ref": within }))
            .or_else(|_| element(state, &json!({ "name": within })))?,
        None => window_element()?,
    };
    let (tree, elements) = ax::walk(&root, &ax::SNAPSHOT_LIMITS);
    let prefix = state.refs.begin();
    let rendered = outline::render(&tree, &prefix, outline::MAX_OUTLINE_BYTES);
    state.refs.register(rendered.refs.iter().map(|handle| elements[*handle].clone()).collect());
    let text = match args.get("filter").and_then(Value::as_str) {
        Some(filter) if !filter.trim().is_empty() => filter_outline(&rendered.text, filter),
        _ => rendered.text,
    };
    Ok(json!({ "outline": text, "elements": rendered.refs.len(), "truncated": rendered.truncated }))
}

/// Keeps a matching line plus the ancestors that gave it context; subtrees without a match
/// drop out whole.
fn filter_outline(text: &str, filter: &str) -> String {
    let needle = filter.to_lowercase();
    let lines: Vec<(usize, &str)> = text
        .lines()
        .map(|line| (line.chars().take_while(|ch| *ch == ' ').count() / 2, line))
        .collect();
    let mut keep = vec![false; lines.len()];
    let mut ancestors: Vec<(usize, usize)> = Vec::new();
    for (index, (depth, _)) in lines.iter().enumerate() {
        while ancestors.last().is_some_and(|(_, d)| *d >= *depth) {
            ancestors.pop();
        }
        if lines[index].1.to_lowercase().contains(&needle) {
            keep[index] = true;
            for (ancestor, _) in &ancestors {
                keep[*ancestor] = true;
            }
        }
        ancestors.push((index, *depth));
    }
    let mut out = String::new();
    for (index, (_, line)) in lines.iter().enumerate() {
        if keep[index] {
            out.push_str(line);
            out.push('\n');
        }
    }
    if out.is_empty() {
        format!("(no lines match “{filter}”)\n")
    } else {
        out
    }
}

fn cmd_find(state: &mut State, args: &Value) -> Result<Value, String> {
    let name = args.get("name").and_then(Value::as_str).ok_or("`find` needs a `name`.")?;
    let role = args.get("role").and_then(Value::as_str);
    let root = window_element()?;
    let matches = matches_in(&root, name, role);
    state.refs.begin();
    let entries: Vec<Value> = matches
        .into_iter()
        .map(|(role, element)| {
            let reference = state.refs.push(element.clone());
            json!({
                "ref": reference,
                "role": role,
                "enabled": ax::boolean(&element, "AXEnabled"),
                "frame": ax::frame(&element).map(|f| json!({"x": f.x, "y": f.y, "w": f.width, "h": f.height})),
            })
        })
        .collect();
    Ok(json!({ "matches": entries }))
}

fn press(element: &Element) -> Result<(), String> {
    let mut last = None;
    for action in ["AXPress", "AXConfirm", "AXPick"] {
        match ax::perform(element, action) {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
    }
    Err(ax::describe_error(last.expect("an action was tried"), "pressing it"))
}

fn cmd_press(state: &mut State, args: &Value) -> Result<Value, String> {
    let element = element(state, args)?;
    if ax::boolean(&element, "AXEnabled") == Some(false) {
        return Err(format!("{} is disabled.", ax::describe(&element)));
    }
    press(&element)?;
    Ok(json!({ "pressed": ax::describe(&element) }))
}

fn cmd_focus(state: &mut State, args: &Value) -> Result<Value, String> {
    let element = element(state, args)?;
    ax::set_bool(&element, "AXFocused", true).map_err(|error| ax::describe_error(error, "focusing it"))?;
    Ok(json!({ "focused": ax::describe(&element) }))
}

fn cmd_type(state: &mut State, args: &Value) -> Result<Value, String> {
    let target = target::resolve()?;
    let element = element(state, args)?;
    if ax::is_secure(&element) {
        return Err("That's a secure text field; the driver never types into one.".into());
    }
    if ax::boolean(&element, "AXEnabled") == Some(false) {
        return Err(format!("{} is disabled.", ax::describe(&element)));
    }
    let text = args.get("text").and_then(Value::as_str).ok_or("`type` needs `text`.")?;
    let _ = ax::set_bool(&element, "AXFocused", true);
    std::thread::sleep(Duration::from_millis(60));
    // Inserting as selected text is the reliable path for fields that keep an AXValue; real
    // keystrokes are the fallback (and fire every key handler, but only into that pid).
    let before = ax::string(&element, "AXValue");
    let via = if ax::set_string(&element, "AXSelectedText", text).is_ok() && ax::string(&element, "AXValue") != before {
        "ax"
    } else {
        input::text_to_pid(target.pid, text, &|| false)?;
        "pid"
    };
    if args.get("submit").and_then(Value::as_bool).unwrap_or(false) {
        input::key_to_pid(target.pid, &keys::parse_chord("Return")?)?;
    }
    Ok(json!({ "typed": text.chars().count(), "via": via }))
}

fn cmd_key(args: &Value) -> Result<Value, String> {
    let target = target::resolve()?;
    let chord = args.get("chord").and_then(Value::as_str).ok_or("`key` needs a `chord` like \"cmd+k\" or \"Return\".")?;
    let app = ax::application(target.pid);
    if ax::focused_is_secure(&app) {
        return Err("A secure text field has focus in the dev app; the driver won't press keys into it.".into());
    }
    input::key_to_pid(target.pid, &keys::parse_chord(chord)?)?;
    Ok(json!({ "pressed": chord }))
}

fn cmd_scroll(state: &mut State, args: &Value) -> Result<Value, String> {
    let element = element(state, args)?;
    let (dx, dy) = match args.get("direction").and_then(Value::as_str).unwrap_or("down") {
        "up" => (0.0, -3.0),
        "down" => (0.0, 3.0),
        "left" => (-3.0, 0.0),
        "right" => (3.0, 0.0),
        direction => return Err(format!("`direction` must be up, down, left or right — not “{direction}”.")),
    };
    // AX scroll bars first; where the webview ignores them (overlay scrollers), a wheel event
    // posted to the dev app's pid at the element's position scrolls it instead.
    if ax::scroll(&element, dx, dy)? {
        return Ok(json!({ "scrolled": true, "via": "ax" }));
    }
    let target = target::resolve()?;
    let frame = ax::frame(&element).ok_or("That element has no position on screen to scroll under.")?;
    let point = CGPoint { x: frame.x + frame.width / 2.0, y: frame.y + frame.height / 2.0 };
    input::scroll_to_pid(target.pid, point, dx, dy)?;
    Ok(json!({ "scrolled": true, "via": "pid" }))
}

fn cmd_wait_for(args: &Value) -> Result<Value, String> {
    let name = args.get("name").and_then(Value::as_str);
    let role = args.get("role").and_then(Value::as_str);
    if name.is_none() && role.is_none() {
        return Err("`wait_for` needs a `name` (optionally with `role`).".into());
    }
    let gone = args.get("gone").and_then(Value::as_bool).unwrap_or(false);
    let timeout = args.get("timeout_ms").and_then(Value::as_u64).unwrap_or(10_000).min(120_000);
    let deadline = Instant::now() + Duration::from_millis(timeout);
    let found = loop {
        // A resolving error (app mid-startup) counts as "not there" until the deadline.
        let found = wait_poll(name, role).unwrap_or(false);
        if found != gone || Instant::now() >= deadline {
            break found;
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    if found == gone {
        return Err(format!(
            "Timed out after {}s waiting for “{}” to {}.",
            timeout / 1000,
            name.or(role).unwrap_or("the control"),
            if gone { "disappear" } else { "appear" }
        ));
    }
    Ok(json!({ "found": found }))
}

fn wait_poll(name: Option<&str>, role: Option<&str>) -> Result<bool, String> {
    let root = window_element()?;
    if let Some(name) = name {
        let (tree, elements) = ax::walk(&root, &ax::SEARCH_LIMITS);
        let mut matches = Vec::new();
        collect_matches(&tree, &elements, name, role, false, &mut matches);
        Ok(!matches.is_empty())
    } else {
        let criteria = ax::Criteria { role, title_contains: None, value_equals: None };
        Ok(ax::find(&root, &criteria).is_some())
    }
}

fn cmd_shot(state: &mut State, args: &Value) -> Result<Value, String> {
    let target = target::resolve()?;
    let (_, number) = dev_window(target.pid)?;
    // The ref's frame and the capture's are both global points; `capture::window` localizes it.
    let crop = match args.get("ref").and_then(Value::as_str) {
        Some(reference) => {
            let element = state.refs.resolve(reference)?;
            let frame = ax::frame(&element).ok_or_else(|| format!("Ref {reference} has no position on screen."))?;
            Some(Rect { x: frame.x, y: frame.y, width: frame.width, height: frame.height })
        }
        None => None,
    };
    let capture = state.rt.block_on(capture::window(number, crop))?;
    let saved_to = match args.get("save_to").and_then(Value::as_str) {
        Some(path) => {
            std::fs::write(path, &capture.jpeg).map_err(|error| format!("Couldn't save the screenshot to {path}: {error}"))?;
            Some(path.to_string())
        }
        None => None,
    };
    Ok(json!({
        "jpeg": BASE64.encode(&capture.jpeg),
        "width": capture.width,
        "height": capture.height,
        "savedTo": saved_to,
    }))
}

fn dispatch(state: &mut State, cmd: &str, args: &Value) -> Result<Value, String> {
    match cmd {
        "status" => Ok(cmd_status()),
        "tree" => cmd_tree(state, args),
        "find" => cmd_find(state, args),
        "press" => cmd_press(state, args),
        "focus" => cmd_focus(state, args),
        "type" => cmd_type(state, args),
        "key" => cmd_key(args),
        "scroll" => cmd_scroll(state, args),
        "wait_for" => cmd_wait_for(args),
        "shot" => cmd_shot(state, args),
        "quit" => std::process::exit(0),
        other => Err(format!("Unknown command “{other}”.")),
    }
}

fn main() {
    // A bare binary has no window-server connection; ScreenCaptureKit asserts without one.
    // `NSApplication::load` opens it (and, with no bundle, never puts anything in the Dock).
    NSApplication::load();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut state = State { refs: Refs::default(), rt: runtime() };
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Err(error) => json!({ "id": Value::Null, "ok": false, "error": format!("Bad JSON: {error}") }),
            Ok(request) => {
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                let cmd = request.get("cmd").and_then(Value::as_str).unwrap_or("");
                match dispatch(&mut state, cmd, &request) {
                    Ok(result) => json!({ "id": id, "ok": true, "result": result }),
                    Err(error) => json!({ "id": id, "ok": false, "error": error }),
                }
            }
        };
        let mut out = stdout.lock();
        let _ = serde_json::to_writer(&mut out, &reply);
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    }
}
