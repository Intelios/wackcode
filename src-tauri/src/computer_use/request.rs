//! Parses and bounds-checks a worker's `computer_request`. The worker's schema only describes
//! the contract to the model; this is where every field is actually enforced. Errors are
//! sentences the model reads.

use super::keys::{self, Chord};
use serde_json::Value;

pub const MAX_ACTIONS: usize = 20;
pub const MAX_TEXT: usize = 4_000;
pub const MAX_WAIT_MS: u64 = 5_000;
/// Waits across one batch, so a batch can't hold the engine for long.
pub const MAX_TOTAL_WAIT_MS: u64 = 10_000;
const MAX_MENU_DEPTH: usize = 8;
const MAX_COORDINATE: f64 = 20_000.0;
const MAX_SCROLL_STEPS: f64 = 50.0;

#[derive(Debug, Clone, PartialEq)]
pub enum ComputerRequest {
    Apps,
    Open { app: String },
    Snapshot { app: String, window: Option<u32> },
    Screenshot { app: String, window: Option<u32> },
    Act { app: String, state_id: String, actions: Vec<Action>, expect: Option<Expect> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Ref(String),
    Pixel(f64, f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Press { target: String },
    Click { target: Target, button: Button, count: u8 },
    SetText { target: String, text: String },
    TypeText { target: Option<String>, text: String },
    Keypress { chord: Chord, keys: String },
    Scroll { target: Target, dx: f64, dy: f64 },
    Drag { from: (f64, f64), to: (f64, f64) },
    Menu { path: Vec<String> },
    Raise,
    Wait { ms: u64 },
}

impl Action {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Press { .. } => "press",
            Self::Click { .. } => "click",
            Self::SetText { .. } => "setText",
            Self::TypeText { .. } => "typeText",
            Self::Keypress { .. } => "keypress",
            Self::Scroll { .. } => "scroll",
            Self::Drag { .. } => "drag",
            Self::Menu { .. } => "menu",
            Self::Raise => "raise",
            Self::Wait { .. } => "wait",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expect {
    pub role: Option<String>,
    pub title_contains: Option<String>,
    pub value_equals: Option<String>,
    pub window_title_contains: Option<String>,
    pub exists: bool,
    pub timeout_ms: u64,
}

fn string_field(object: &Value, key: &str) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("{key} must be a string.")),
    }
}

fn required_string(object: &Value, key: &str, context: &str) -> Result<String, String> {
    string_field(object, key)?
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("{context} needs {key}."))
}

fn number_field(object: &Value, key: &str) -> Result<Option<f64>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|number| number.is_finite())
            .map(Some)
            .ok_or_else(|| format!("{key} must be a number.")),
    }
}

fn app_field(object: &Value) -> Result<String, String> {
    let app = required_string(object, "app", "This request")?;
    let app = app.trim().to_string();
    if app.chars().count() > 1_024 {
        return Err("app is too long.".into());
    }
    Ok(app)
}

fn window_field(object: &Value) -> Result<Option<u32>, String> {
    match number_field(object, "window")? {
        None => Ok(None),
        Some(number) if number >= 0.0 && number <= u32::MAX as f64 && number.fract() == 0.0 => Ok(Some(number as u32)),
        Some(_) => Err("window must be a window id from computer_apps.".into()),
    }
}

fn coordinate(object: &Value, x: &str, y: &str, kind: &str) -> Result<Option<(f64, f64)>, String> {
    match (number_field(object, x)?, number_field(object, y)?) {
        (None, None) => Ok(None),
        (Some(px), Some(py)) if (0.0..=MAX_COORDINATE).contains(&px) && (0.0..=MAX_COORDINATE).contains(&py) => Ok(Some((px, py))),
        (Some(_), Some(_)) => Err(format!("{kind}: {x} and {y} must be pixels inside the screenshot.")),
        _ => Err(format!("{kind} needs both {x} and {y}.")),
    }
}

fn target(object: &Value, kind: &str) -> Result<Target, String> {
    let reference = string_field(object, "ref")?.filter(|text| !text.trim().is_empty());
    match (reference, coordinate(object, "x", "y", kind)?) {
        (Some(_), Some(_)) => Err(format!("{kind} takes either ref or x/y, not both.")),
        (Some(reference), None) => Ok(Target::Ref(reference.trim().to_string())),
        (None, Some((x, y))) => Ok(Target::Pixel(x, y)),
        (None, None) => Err(format!("{kind} needs a ref or screenshot x/y.")),
    }
}

fn text_field(object: &Value, kind: &str, allow_empty: bool) -> Result<String, String> {
    let text = string_field(object, "text")?.ok_or_else(|| format!("{kind} needs text."))?;
    if !allow_empty && text.is_empty() {
        return Err(format!("{kind} needs text."));
    }
    if text.chars().count() > MAX_TEXT {
        return Err(format!("{kind} text can be at most {MAX_TEXT} characters; split it across actions."));
    }
    Ok(text)
}

fn parse_action(value: &Value) -> Result<Action, String> {
    if !value.is_object() {
        return Err("Each action must be an object with a kind.".into());
    }
    let kind = required_string(value, "kind", "An action")?;
    Ok(match kind.as_str() {
        "press" => Action::Press { target: required_string(value, "ref", "press")?.trim().to_string() },
        "click" => {
            let button = match string_field(value, "button")?.as_deref() {
                None | Some("left") => Button::Left,
                Some("right") => Button::Right,
                Some("middle") => Button::Middle,
                Some(other) => return Err(format!("click button “{other}” must be left, right or middle.")),
            };
            let count = match number_field(value, "count")? {
                None => 1,
                Some(count) if (1.0..=3.0).contains(&count) && count.fract() == 0.0 => count as u8,
                Some(_) => return Err("click count must be 1, 2 or 3.".into()),
            };
            Action::Click { target: target(value, "click")?, button, count }
        }
        "setText" => Action::SetText { target: required_string(value, "ref", "setText")?.trim().to_string(), text: text_field(value, "setText", true)? },
        "typeText" => Action::TypeText {
            target: string_field(value, "ref")?.map(|text| text.trim().to_string()).filter(|text| !text.is_empty()),
            text: text_field(value, "typeText", false)?,
        },
        "keypress" => {
            let keys = required_string(value, "keys", "keypress")?;
            Action::Keypress { chord: keys::parse_chord(&keys)?, keys }
        }
        "scroll" => {
            let dx = number_field(value, "dx")?.unwrap_or(0.0);
            let dy = number_field(value, "dy")?.unwrap_or(0.0);
            if dx == 0.0 && dy == 0.0 {
                return Err("scroll needs dx or dy.".into());
            }
            if dx.abs() > MAX_SCROLL_STEPS || dy.abs() > MAX_SCROLL_STEPS {
                return Err(format!("scroll steps can be at most {MAX_SCROLL_STEPS} each way."));
            }
            Action::Scroll { target: target(value, "scroll")?, dx, dy }
        }
        "drag" => Action::Drag {
            from: coordinate(value, "x", "y", "drag")?.ok_or("drag needs x and y.")?,
            to: coordinate(value, "toX", "toY", "drag")?.ok_or("drag needs toX and toY.")?,
        },
        "menu" => {
            let path = match value.get("path") {
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|item| item.as_str().map(str::trim).filter(|text| !text.is_empty() && text.chars().count() <= 200).map(str::to_string))
                    .collect::<Option<Vec<_>>>()
                    .ok_or("menu path must be a list of menu titles.")?,
                _ => return Err("menu needs a path, e.g. [\"File\", \"Save\"].".into()),
            };
            if path.is_empty() || path.len() > MAX_MENU_DEPTH {
                return Err(format!("menu path needs 1 to {MAX_MENU_DEPTH} titles."));
            }
            Action::Menu { path }
        }
        "raise" => Action::Raise,
        "wait" => match number_field(value, "ms")? {
            Some(ms) if (0.0..=MAX_WAIT_MS as f64).contains(&ms) => Action::Wait { ms: ms as u64 },
            Some(_) => return Err(format!("wait ms must be between 0 and {MAX_WAIT_MS}.")),
            None => return Err("wait needs ms.".into()),
        },
        other => return Err(format!("“{other}” isn't an action. Use press, click, setText, typeText, keypress, scroll, drag, menu, raise or wait.")),
    })
}

fn parse_expect(value: &Value) -> Result<Option<Expect>, String> {
    if value.is_null() {
        return Ok(None);
    }
    if !value.is_object() {
        return Err("expect must be an object.".into());
    }
    let clip = |text: Option<String>| text.map(|text| text.chars().take(200).collect::<String>()).filter(|text| !text.is_empty());
    let expect = Expect {
        role: clip(string_field(value, "role")?),
        title_contains: clip(string_field(value, "titleContains")?),
        value_equals: string_field(value, "valueEquals")?.map(|text| text.chars().take(500).collect()),
        window_title_contains: clip(string_field(value, "windowTitleContains")?),
        exists: match value.get("exists") {
            None | Some(Value::Null) => true,
            Some(Value::Bool(exists)) => *exists,
            Some(_) => return Err("expect.exists must be true or false.".into()),
        },
        timeout_ms: match number_field(value, "timeoutMs")? {
            None => 1_500,
            Some(ms) if (0.0..=MAX_WAIT_MS as f64).contains(&ms) => ms as u64,
            Some(_) => return Err(format!("expect.timeoutMs must be between 0 and {MAX_WAIT_MS}.")),
        },
    };
    // Nothing to check (models sometimes send `{ exists: true }` alone): no postcondition,
    // rather than failing the actions over it.
    if expect.role.is_none() && expect.title_contains.is_none() && expect.value_equals.is_none() && expect.window_title_contains.is_none() {
        return Ok(None);
    }
    Ok(Some(expect))
}

pub fn parse(value: &Value) -> Result<ComputerRequest, String> {
    let op = value.get("op").and_then(Value::as_str).unwrap_or("");
    Ok(match op {
        "apps" => ComputerRequest::Apps,
        "open" => ComputerRequest::Open { app: app_field(value)? },
        "snapshot" => ComputerRequest::Snapshot { app: app_field(value)?, window: window_field(value)? },
        "screenshot" => ComputerRequest::Screenshot { app: app_field(value)?, window: window_field(value)? },
        "act" => {
            let actions = match value.get("actions") {
                Some(Value::Array(items)) if !items.is_empty() && items.len() <= MAX_ACTIONS => {
                    items.iter().enumerate().map(|(index, item)| parse_action(item).map_err(|error| format!("Action {}: {error}", index + 1))).collect::<Result<Vec<_>, _>>()?
                }
                _ => return Err(format!("computer_act needs 1 to {MAX_ACTIONS} actions.")),
            };
            let waits: u64 = actions.iter().map(|action| if let Action::Wait { ms } = action { *ms } else { 0 }).sum();
            if waits > MAX_TOTAL_WAIT_MS {
                return Err(format!("A batch can wait at most {MAX_TOTAL_WAIT_MS} ms in total."));
            }
            ComputerRequest::Act {
                app: app_field(value)?,
                state_id: required_string(value, "stateId", "computer_act")?.trim().to_string(),
                actions,
                expect: parse_expect(value.get("expect").unwrap_or(&Value::Null))?,
            }
        }
        _ => return Err("Unknown computer-use request.".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_each_operation() {
        assert_eq!(parse(&json!({ "op": "apps" })).unwrap(), ComputerRequest::Apps);
        assert_eq!(parse(&json!({ "op": "open", "app": " TextEdit " })).unwrap(), ComputerRequest::Open { app: "TextEdit".into() });
        assert_eq!(
            parse(&json!({ "op": "snapshot", "app": "TextEdit", "window": 42 })).unwrap(),
            ComputerRequest::Snapshot { app: "TextEdit".into(), window: Some(42) }
        );
        assert!(parse(&json!({ "op": "screenshot", "app": "X", "window": 1.5 })).is_err());
        assert!(parse(&json!({ "op": "snapshot" })).is_err());
        assert!(parse(&json!({ "op": "delete_everything" })).is_err());
    }

    #[test]
    fn parses_and_bounds_actions() {
        let request = parse(&json!({
            "op": "act", "app": "TextEdit", "stateId": "s3",
            "actions": [
                { "kind": "press", "ref": "e3-1" },
                { "kind": "click", "x": 10, "y": 20, "count": 2 },
                { "kind": "click", "ref": "e3-2", "button": "right" },
                { "kind": "typeText", "text": "hello" },
                { "kind": "keypress", "keys": "cmd+s" },
                { "kind": "scroll", "ref": "e3-4", "dy": 3 },
                { "kind": "drag", "x": 1, "y": 2, "toX": 3, "toY": 4 },
                { "kind": "menu", "path": ["File", "Save As…"] },
                { "kind": "raise" },
                { "kind": "wait", "ms": 200 }
            ],
            "expect": { "titleContains": "Saved" }
        }))
        .unwrap();
        let ComputerRequest::Act { actions, expect, .. } = request else { panic!("not an act") };
        assert_eq!(actions.len(), 10);
        assert_eq!(actions[1], Action::Click { target: Target::Pixel(10.0, 20.0), button: Button::Left, count: 2 });
        assert_eq!(expect.unwrap().timeout_ms, 1_500);
    }

    #[test]
    fn rejects_bad_actions_with_their_position() {
        let act = |action: Value| parse(&json!({ "op": "act", "app": "A", "stateId": "s1", "actions": [action] }));
        assert!(act(json!({ "kind": "click" })).unwrap_err().starts_with("Action 1:"));
        assert!(act(json!({ "kind": "click", "ref": "e1-1", "x": 1, "y": 1 })).is_err());
        assert!(act(json!({ "kind": "click", "x": 1 })).is_err());
        assert!(act(json!({ "kind": "click", "x": -1, "y": 1 })).is_err());
        assert!(act(json!({ "kind": "click", "ref": "e1-1", "count": 4 })).is_err());
        assert!(act(json!({ "kind": "typeText", "text": "" })).is_err());
        assert!(act(json!({ "kind": "typeText", "text": "x".repeat(MAX_TEXT + 1) })).is_err());
        assert!(act(json!({ "kind": "keypress", "keys": "ctrl+alt+cmd+." })).is_err());
        assert!(act(json!({ "kind": "scroll", "ref": "e1-1" })).is_err());
        assert!(act(json!({ "kind": "menu", "path": [] })).is_err());
        assert!(act(json!({ "kind": "wait", "ms": 9_000 })).is_err());
        assert!(act(json!({ "kind": "launch_missiles" })).is_err());
        assert!(parse(&json!({ "op": "act", "app": "A", "stateId": "s1", "actions": [] })).is_err());
        let many = vec![json!({ "kind": "raise" }); MAX_ACTIONS + 1];
        assert!(parse(&json!({ "op": "act", "app": "A", "stateId": "s1", "actions": many })).is_err());
        let waits = vec![json!({ "kind": "wait", "ms": 4_000 }); 3];
        assert!(parse(&json!({ "op": "act", "app": "A", "stateId": "s1", "actions": waits })).is_err());
    }

    #[test]
    fn expect_without_a_criterion_is_ignored() {
        let act = |expect: Value| parse(&json!({ "op": "act", "app": "A", "stateId": "s1", "actions": [{ "kind": "raise" }], "expect": expect }));
        let expect_of = |request: ComputerRequest| match request { ComputerRequest::Act { expect, .. } => expect, _ => panic!("not an act") };
        assert_eq!(expect_of(act(json!({})).unwrap()), None);
        assert_eq!(expect_of(act(json!({ "exists": true, "timeoutMs": 4000 })).unwrap()), None);
        assert!(act(json!("soon")).is_err());
        assert!(act(json!({ "role": "AXSheet", "exists": false, "timeoutMs": 800 })).is_ok());
        assert!(act(json!({ "role": "AXSheet", "timeoutMs": 60_000 })).is_err());
    }
}
