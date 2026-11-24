//! The tools that act: `computer_act` and `computer_batch`.
//!
//! Invariants:
//! - Look-only apps (browsers) are never acted on.
//! - A coordinate is read in the window's last full picture, shifted by any move since, and
//!   refused once the window has changed size. The deepest element the last walk places under it
//!   is what is acted on (Accessibility's own hit test misses background windows: lib.rs).
//! - After an action the app is read until it holds still (`settle`), at most
//!   [`SETTLE_LIMIT`], and the answer says what changed, so the model rarely needs to look again.
//! - A batch runs its steps in one turn of the lane, stops at the first failure, and carries at
//!   most [`BATCH_PICTURES`] pictures.

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;
use wack_agent::ToolOutput;

use super::{Call, Kind, app_data, look};
use crate::{
    backend::{Act, AppInfo, Button, Fingerprint, Target, Via, WindowInfo},
    change, geometry,
    keys::{self, Modifiers},
    ladder::{self, Step},
    outline::{self, Node},
    policy::Tier,
};

/// How long an action's effects are waited for.
const SETTLE_LIMIT: Duration = Duration::from_millis(1500);
/// How long the app must read the same to count as settled.
const SETTLE_STILL: Duration = Duration::from_millis(150);
const SETTLE_STEP: Duration = Duration::from_millis(50);
pub const BATCH_STEPS: usize = 20;
pub const BATCH_PICTURES: usize = 5;
/// The size slack between a window and its last picture (its border, rounding).
const SIZE_SLACK: f64 = 4.0;

#[derive(Deserialize, Default)]
#[serde(default)]
struct ActInput {
    app: String,
    window: Option<u32>,
    action: String,
    #[serde(rename = "ref")]
    reference: Option<String>,
    /// A control by its label, when exactly one has it.
    name: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    focused: bool,
    button: Option<String>,
    count: Option<u32>,
    modifiers: Vec<String>,
    text: Option<String>,
    replace: bool,
    keys: Option<String>,
    times: Option<u32>,
    direction: Option<String>,
    amount: Option<f64>,
    to: Option<[f64; 2]>,
    path: Option<Vec<[f64; 2]>>,
    menu: Option<Vec<String>>,
    summary: Option<String>,
}

fn modifiers(names: &[String]) -> Result<Modifiers, String> {
    let mut modifiers = Modifiers::default();
    for name in names {
        match name.trim().to_lowercase().as_str() {
            "cmd" | "command" | "⌘" => modifiers.command = true,
            "shift" | "⇧" => modifiers.shift = true,
            "alt" | "option" | "opt" | "⌥" => modifiers.option = true,
            "ctrl" | "control" | "⌃" => modifiers.control = true,
            "fn" | "function" => modifiers.function = true,
            other => return Err(format!("“{other}” isn't a modifier: use cmd, shift, option or control.")),
        }
    }
    Ok(modifiers)
}

/// The deepest element whose frame holds `point`, preferring one that can be acted on.
fn element_at(root: &Node, point: (f64, f64)) -> Option<&Node> {
    let mut best: Option<(&Node, bool, usize)> = None;
    fn walk<'a>(node: &'a Node, point: (f64, f64), depth: usize, best: &mut Option<(&'a Node, bool, usize)>) {
        let inside = node.frame.is_some_and(|frame| frame.has_area() && frame.contains(point.0, point.1));
        if inside {
            let actionable = outline::is_actionable(node);
            let better = match best {
                None => true,
                Some((_, best_actionable, best_depth)) => (actionable && !*best_actionable) || (actionable == *best_actionable && depth >= *best_depth),
            };
            if better {
                *best = Some((node, actionable, depth));
            }
        }
        for child in &node.children {
            walk(child, point, depth + 1, best);
        }
    }
    walk(root, point, 0, &mut best);
    best.map(|(node, _, _)| node)
}

/// The one control labelled `name` (any case), or why there isn't one.
fn by_name<'a>(root: &'a Node, name: &str, app: &str) -> Result<&'a Node, String> {
    let wanted = name.trim().to_lowercase();
    let mut found = Vec::new();
    root.each(&mut |node| {
        if outline::is_actionable(node) && outline::label(node).is_some_and(|label| label.trim().to_lowercase() == wanted) {
            found.push(node.handle);
        }
    });
    match found.as_slice() {
        [handle] => root.find(*handle).ok_or_else(|| "That control is gone.".to_string()),
        [] => Err(format!("No control in {app}'s window is labelled “{}”: take a computer_snapshot to see them.", name.trim())),
        many => {
            let list: Vec<String> = many.iter().filter_map(|handle| root.find(*handle)).map(|node| format!("[e{}] {}", node.handle, outline::describe(node))).collect();
            Err(format!("{} controls are labelled “{}”: use a ref instead.\n{}", many.len(), name.trim(), list.join("\n")))
        }
    }
}

/// A picture point to a global point, through the window's last full picture.
fn picture_point(call: &Call<'_>, app: &AppInfo, window: &WindowInfo, x: f64, y: f64) -> Result<(f64, f64), String> {
    let name = &app.identity.name;
    let shot = call
        .chat
        .last_shot(app.pid, window.id)
        .ok_or_else(|| format!("x and y are in the pixels of the window's last picture: take a computer_screenshot of {name}'s window first, or act by ref."))?;
    let frame = window.frame.ok_or("The window has no frame now: it may be minimized.")?;
    let at_shot = shot.window.unwrap_or(frame);
    if (frame.width - at_shot.width).abs() > SIZE_SLACK || (frame.height - at_shot.height).abs() > SIZE_SLACK {
        return Err(ladder::advice(Step::Click, &ladder::Reason::Resized, name));
    }
    let (lx, ly) = geometry::to_window((shot.bounds.width, shot.bounds.height), shot.frame, x, y)
        .ok_or_else(|| format!("({x}, {y}) is outside the picture, which is {}×{}.", shot.frame.0, shot.frame.1))?;
    // The window may have moved since: carry the point with it.
    Ok((shot.bounds.x + lx + (frame.x - at_shot.x), shot.bounds.y + ly + (frame.y - at_shot.y)))
}

/// Where the action lands, and the ref whose value the answer reports.
async fn target(call: &Call<'_>, app: &AppInfo, window: &WindowInfo, input: &ActInput) -> Result<Option<(Target, Option<u32>)>, String> {
    if let Some(raw) = &input.reference {
        let reference = outline::parse_ref(raw).ok_or_else(|| format!("“{raw}” isn't a ref: refs look like e12."))?;
        if !call.chat.was_shown(app.pid, reference) {
            return Err(format!("e{reference} hasn't been shown to you in {}: take a computer_snapshot first.", app.identity.name));
        }
        return Ok(Some((Target::Ref(reference), Some(reference))));
    }
    if let Some(name) = input.name.as_deref().filter(|name| !name.trim().is_empty()) {
        let tree = call.chat.desktop().tree(app.pid, window.id, None).await?;
        let node = by_name(&tree.root, name, &app.identity.name)?;
        let reference = node.handle as u32;
        call.chat.saw(app.pid, &[reference]);
        return Ok(Some((Target::Ref(reference), Some(reference))));
    }
    if let (Some(x), Some(y)) = (input.x, input.y) {
        let point = picture_point(call, app, window, x, y)?;
        let tree = call.chat.desktop().tree(app.pid, window.id, None).await?;
        let element = element_at(&tree.root, point).map(|node| node.handle as u32);
        if let Some(element) = element {
            call.chat.saw(app.pid, &[element]);
        }
        return Ok(Some((Target::At { point, element }, element)));
    }
    if input.x.is_some() || input.y.is_some() {
        return Err("Give both x and y.".into());
    }
    if input.focused {
        return Ok(Some((Target::Focused, None)));
    }
    Ok(None)
}

fn step_of(action: &str) -> Step {
    match action {
        "type" => Step::Type,
        "key" => Step::Key,
        "scroll" => Step::Scroll,
        "drag" => Step::Drag,
        _ => Step::Click,
    }
}

/// Reads the app until it holds still after an action, at most [`SETTLE_LIMIT`].
async fn settle(call: &Call<'_>, pid: i32, target: Option<u32>) -> Fingerprint {
    let started = Instant::now();
    let mut last = call.chat.desktop().fingerprint(pid, target).await;
    let mut still_since = Instant::now();
    while started.elapsed() < SETTLE_LIMIT {
        tokio::time::sleep(SETTLE_STEP).await;
        let now = call.chat.desktop().fingerprint(pid, target).await;
        if now == last {
            if still_since.elapsed() >= SETTLE_STILL {
                break;
            }
        } else {
            last = now;
            still_since = Instant::now();
        }
    }
    last
}

pub(super) async fn act(call: &Call<'_>, input: Value) -> Result<ToolOutput, String> {
    let input: ActInput = serde_json::from_value(input).map_err(|error| format!("computer_act didn't understand its input: {error}."))?;
    let (app, tier) = call.running_app(&input.app).await?;
    let name = app.identity.name.clone();
    if tier == Tier::LookOnly {
        return Err(format!("{name} is look-only: computer use reads it but never acts in it. Work on the web with the browser_* tools."));
    }
    let window = call.window(&app, input.window).await?;
    let desktop = call.chat.desktop();
    let action = input.action.trim().to_lowercase();
    let found = target(call, &app, &window, &input).await?;
    let needs_target = |what: &str| -> Result<(Target, Option<u32>), String> {
        found.ok_or_else(|| format!("{what} needs a `ref` (from computer_snapshot), a control's `name`, `x` and `y` (in the last computer_screenshot), or `focused: true`."))
    };
    let (act, watched) = match action.as_str() {
        "click" => {
            let (target, watched) = needs_target("click")?;
            let button = match input.button.as_deref().unwrap_or("left") {
                "left" => Button::Left,
                "right" => Button::Right,
                other => return Err(format!("“{other}” isn't a button: use left or right.")),
            };
            (Act::Click { target, button, count: input.count.unwrap_or(1).clamp(1, 3), modifiers: modifiers(&input.modifiers)? }, watched)
        }
        "type" => {
            let text = input.text.clone().filter(|text| !text.is_empty()).ok_or("type needs `text`.")?;
            let (target, watched) = found.unwrap_or((Target::Focused, None));
            (Act::Type { target, text, replace: input.replace }, watched)
        }
        "key" => {
            let raw = input.keys.as_deref().ok_or("key needs `keys`, like \"cmd+s\" or \"enter\".")?;
            let chord = keys::parse_chord(raw, &desktop.layout())?;
            (Act::Key { chord, times: input.times.unwrap_or(1).clamp(1, 50) }, None)
        }
        "scroll" => {
            let (target, watched) = needs_target("scroll")?;
            let amount = input.amount.unwrap_or(3.0).clamp(0.5, 30.0);
            let (dx, dy) = match input.direction.as_deref().unwrap_or("down") {
                "down" => (0.0, amount),
                "up" => (0.0, -amount),
                "right" => (amount, 0.0),
                "left" => (-amount, 0.0),
                other => return Err(format!("“{other}” isn't a direction: use up, down, left or right.")),
            };
            (Act::Scroll { target, dx, dy }, watched)
        }
        "drag" => {
            let (Some(x), Some(y)) = (input.x, input.y) else { return Err("drag starts at `x` and `y`, in the last computer_screenshot.".into()) };
            let mut points = vec![picture_point(call, &app, &window, x, y)?];
            for [px, py] in input.path.clone().unwrap_or_default() {
                points.push(picture_point(call, &app, &window, px, py)?);
            }
            if let Some([tx, ty]) = input.to {
                points.push(picture_point(call, &app, &window, tx, ty)?);
            }
            if points.len() < 2 {
                return Err("drag needs `to` [x, y], or a `path` of points.".into());
            }
            (Act::Drag { path: points }, None)
        }
        "menu" => {
            let path = input.menu.clone().filter(|path| !path.is_empty()).ok_or("menu needs `menu`, the path from the menu bar, like [\"File\", \"Save As…\"].")?;
            (Act::Menu { path }, None)
        }
        "focus" => {
            let (target, watched) = needs_target("focus")?;
            (Act::Focus { target }, watched)
        }
        other => return Err(format!("“{other}” isn't an action: use click, type, key, scroll, drag, menu or focus.")),
    };
    let before = desktop.fingerprint(app.pid, watched).await;
    let before_tree = desktop.glance(app.pid, window.id).await;
    let done = match desktop.act(app.pid, window.id, act).await {
        Ok(done) => done,
        Err(reason) => return Err(ladder::advice(step_of(&action), &reason, &name)),
    };
    let after = settle(call, app.pid, watched).await;
    let mut changes = change::report(&before, &after, app.pid, &name);
    // Inside the window: values that changed elsewhere, and anything that opened over it.
    if let (Some(before_tree), Some(after_tree)) = (before_tree, desktop.glance(app.pid, window.id).await) {
        let (lines, refs) = change::tree_report(&before_tree, &after_tree, watched, 5);
        call.chat.saw(app.pid, &refs);
        changes.extend(lines);
    }
    let what = match (action.as_str(), input.keys.as_deref()) {
        ("key", Some(keys)) => format!("pressed {}", keys.trim()),
        _ => done.what.clone(),
    };
    let mut lines = vec![format!("{what} in {name}.")];
    if let Some(replaced) = &done.replaced {
        lines.push(format!("It replaced: \"{replaced}\""));
    }
    if changes.is_empty() {
        lines.push("Nothing visible changed.".into());
    } else {
        lines.push("Then:".into());
        lines.extend(changes.iter().map(|change| format!("- {change}")));
    }
    let mut text = app_data(&lines.join("\n"));
    // Raw input is unverified, unless the change report shows what it did.
    if done.via == Via::Raw && changes.is_empty() {
        text.push_str(&format!("\n{}", done.note.as_deref().unwrap_or("Sent as raw input: check the result.")));
    } else if done.via == Via::Ax
        && let Some(note) = &done.note
    {
        text.push_str(&format!("\n{note}"));
    }
    Ok(text.into())
}

#[derive(Deserialize)]
struct BatchStep {
    tool: String,
    #[serde(default)]
    input: Value,
}

#[derive(Deserialize)]
struct BatchInput {
    steps: Vec<BatchStep>,
}

pub(super) async fn batch(call: &Call<'_>, input: Value) -> Result<ToolOutput, String> {
    let input: BatchInput = serde_json::from_value(input).map_err(|error| format!("computer_batch didn't understand its input: {error}."))?;
    if input.steps.is_empty() {
        return Err("computer_batch needs at least one step.".into());
    }
    if input.steps.len() > BATCH_STEPS {
        return Err(format!("A batch takes at most {BATCH_STEPS} steps."));
    }
    let mut text = String::new();
    let mut images = Vec::new();
    for (index, step) in input.steps.into_iter().enumerate() {
        let number = index + 1;
        let kind = match Kind::from_name(&step.tool) {
            Some(kind) if !matches!(kind, Kind::Batch | Kind::Open | Kind::Apps) => kind,
            _ => return Err(format!("{text}Step {number}: a batch runs computer_act, computer_snapshot, computer_screenshot and computer_wait, not “{}”. Nothing after it ran.", step.tool)),
        };
        if kind == Kind::Screenshot && images.len() >= BATCH_PICTURES {
            return Err(format!("{text}Step {number}: a batch carries at most {BATCH_PICTURES} pictures. Nothing after it ran."));
        }
        if call.context.cancel.is_cancelled() {
            return Err(format!("{text}Stopped before step {number}."));
        }
        let result = match kind {
            Kind::Act => act(call, step.input).await,
            Kind::Snapshot => look::snapshot(call, step.input).await,
            Kind::Screenshot => look::screenshot(call, step.input).await,
            Kind::Wait => look::wait(call, step.input).await,
            Kind::Apps | Kind::Open | Kind::Batch => unreachable!("refused above"),
        };
        match result {
            Ok(output) => {
                text.push_str(&format!("Step {number} ({}): {}\n", step.tool, output.text));
                images.extend(output.images);
            }
            Err(error) => return Err(format!("{text}Step {number} ({}) failed: {error}\nNothing after it ran.", step.tool)),
        }
    }
    Ok(ToolOutput::with_images(text.trim_end(), images))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;

    #[test]
    fn the_deepest_actionable_element_under_a_point_wins() {
        let button = Node { handle: 3, role: "AXButton".into(), frame: Some(Rect::new(10.0, 10.0, 50.0, 20.0)), ..Node::default() };
        let label = Node { handle: 4, role: "AXStaticText".into(), frame: Some(Rect::new(12.0, 12.0, 30.0, 10.0)), ..Node::default() };
        let group = Node { handle: 2, role: "AXGroup".into(), frame: Some(Rect::new(0.0, 0.0, 100.0, 100.0)), children: vec![Node { children: vec![label], ..button }], ..Node::default() };
        let root = Node { handle: 1, role: "AXWindow".into(), frame: Some(Rect::new(0.0, 0.0, 200.0, 200.0)), children: vec![group], ..Node::default() };
        assert_eq!(element_at(&root, (15.0, 15.0)).map(|node| node.handle), Some(3), "the button, not the text inside it");
        assert_eq!(element_at(&root, (80.0, 80.0)).map(|node| node.handle), Some(2), "nothing actionable: the deepest");
        assert_eq!(element_at(&root, (500.0, 500.0)), None);
    }

    #[test]
    fn a_name_finds_exactly_one_control() {
        let button = |handle: usize, title: &str| Node { handle, role: "AXButton".into(), title: Some(title.into()), ..Node::default() };
        let label = Node { handle: 9, role: "AXStaticText".into(), title: Some("2".into()), ..Node::default() };
        let root = Node { handle: 1, role: "AXWindow".into(), children: vec![button(2, "1"), button(3, "2"), button(4, "Save"), button(5, "Save"), label], ..Node::default() };
        assert_eq!(by_name(&root, " 2 ", "Calculator").unwrap().handle, 3, "the button, not the text");
        let many = by_name(&root, "save", "TextEdit").unwrap_err();
        assert!(many.contains("[e4]") && many.contains("[e5]"), "{many}");
        assert!(by_name(&root, "Open", "TextEdit").unwrap_err().contains("No control"));
    }

    #[test]
    fn modifiers_read_any_spelling() {
        let parsed = modifiers(&["Cmd".into(), "option".into(), "⇧".into()]).unwrap();
        assert!(parsed.command && parsed.option && parsed.shift && !parsed.control);
        assert!(modifiers(&["hyper".into()]).is_err());
    }
}
