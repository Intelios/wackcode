//! Trusted input for `browser_act`: mouse events at a point and keys by name, sent through
//! DevTools' `Input` domain, so the page sees them as a person's.
//!
//! Points are CSS pixels in the page's viewport. Keys are named the way people write them
//! (`Enter`, `cmd+a`, `shift+Tab`); macOS's editing shortcuts are sent as the editing commands
//! Chromium runs for them, because a synthetic ⌘A otherwise selects nothing.

use serde_json::{Value, json};

/// DevTools' modifier bits.
pub const ALT: u32 = 1;
pub const CTRL: u32 = 2;
pub const META: u32 = 4;
pub const SHIFT: u32 = 8;

/// A key to press: what DevTools needs to make it look real.
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub key: String,
    pub code: String,
    pub key_code: u32,
    /// The text a press of it types, if any (`Enter` types "\r").
    pub text: Option<String>,
    pub modifiers: u32,
    /// Editing commands Chromium runs for it on macOS (`selectAll` for ⌘A).
    pub commands: Vec<&'static str>,
}

/// Modifier names in a key combination or the `modifiers` list.
pub fn modifier(name: &str) -> Option<u32> {
    match name.to_ascii_lowercase().as_str() {
        "cmd" | "command" | "meta" | "super" => Some(META),
        "ctrl" | "control" => Some(CTRL),
        "alt" | "option" | "opt" => Some(ALT),
        "shift" => Some(SHIFT),
        _ => None,
    }
}

/// Parses `cmd+shift+z`, `Enter`, `ArrowDown`, `a`.
pub fn parse_key(combo: &str) -> Result<Key, String> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).filter(|part| !part.is_empty()).collect();
    let Some((last, mods)) = parts.split_last() else {
        return Err("Name a key, like \"Enter\" or \"cmd+a\".".into());
    };
    let mut modifiers = 0;
    for part in mods {
        modifiers |= modifier(part).ok_or_else(|| format!("\"{part}\" is not a modifier: use cmd, ctrl, alt or shift."))?;
    }
    let mut key = named(last).ok_or_else(|| format!("\"{last}\" is not a key I know. Use a character, or a name like Enter, Tab, Escape, Backspace, ArrowDown, PageDown."))?;
    key.modifiers = modifiers;
    if modifiers & (META | CTRL | ALT) != 0 {
        key.text = None;
    } else if modifiers & SHIFT != 0
        && let Some(upper) = key.text.as_ref().filter(|text| text.chars().count() == 1 && text.chars().all(|c| c.is_ascii_lowercase())).map(|text| text.to_ascii_uppercase())
    {
        key.text = Some(upper.clone());
        key.key = upper;
    }
    if modifiers == META {
        key.commands = match key.key.to_ascii_lowercase().as_str() {
            "a" => vec!["selectAll"],
            "c" => vec!["copy"],
            "v" => vec!["paste"],
            "x" => vec!["cut"],
            "z" => vec!["undo"],
            _ => Vec::new(),
        };
    } else if modifiers == META | SHIFT && key.key.eq_ignore_ascii_case("z") {
        key.commands = vec!["redo"];
    }
    Ok(key)
}

fn named(name: &str) -> Option<Key> {
    let simple = |key: &str, code: &str, key_code: u32, text: Option<&str>| Key {
        key: key.into(),
        code: code.into(),
        key_code,
        text: text.map(Into::into),
        modifiers: 0,
        commands: Vec::new(),
    };
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(character(c));
    }
    Some(match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => simple("Enter", "Enter", 13, Some("\r")),
        "tab" => simple("Tab", "Tab", 9, None),
        "escape" | "esc" => simple("Escape", "Escape", 27, None),
        "backspace" => simple("Backspace", "Backspace", 8, None),
        "delete" | "del" => simple("Delete", "Delete", 46, None),
        "space" => simple(" ", "Space", 32, Some(" ")),
        "arrowup" | "up" => simple("ArrowUp", "ArrowUp", 38, None),
        "arrowdown" | "down" => simple("ArrowDown", "ArrowDown", 40, None),
        "arrowleft" | "left" => simple("ArrowLeft", "ArrowLeft", 37, None),
        "arrowright" | "right" => simple("ArrowRight", "ArrowRight", 39, None),
        "home" => simple("Home", "Home", 36, None),
        "end" => simple("End", "End", 35, None),
        "pageup" => simple("PageUp", "PageUp", 33, None),
        "pagedown" => simple("PageDown", "PageDown", 34, None),
        other => {
            let number: u32 = other.strip_prefix('f')?.parse().ok().filter(|n| (1..=12).contains(n))?;
            let name = format!("F{number}");
            Key { key: name.clone(), code: name, key_code: 111 + number, text: None, modifiers: 0, commands: Vec::new() }
        }
    })
}

/// The key that types `c` on a US layout (other characters are sent as text alone).
pub fn character(c: char) -> Key {
    let (code, key_code) = if c.is_ascii_alphabetic() {
        (format!("Key{}", c.to_ascii_uppercase()), c.to_ascii_uppercase() as u32)
    } else if c.is_ascii_digit() {
        (format!("Digit{c}"), c as u32)
    } else if c == ' ' {
        ("Space".into(), 32)
    } else {
        (String::new(), 0)
    };
    Key { key: c.to_string(), code, key_code, text: Some(c.to_string()), modifiers: 0, commands: Vec::new() }
}

/// The `Input.dispatchKeyEvent` calls for one press: down (typing its text) and up.
pub fn key_events(key: &Key) -> Vec<Value> {
    let mut down = json!({
        "type": if key.text.is_some() { "keyDown" } else { "rawKeyDown" },
        "key": key.key,
        "code": key.code,
        "windowsVirtualKeyCode": key.key_code,
        "nativeVirtualKeyCode": key.key_code,
        "modifiers": key.modifiers,
    });
    if let Some(text) = &key.text {
        down["text"] = json!(text);
        down["unmodifiedText"] = json!(text);
    }
    if !key.commands.is_empty() {
        down["commands"] = json!(key.commands);
    }
    let up = json!({
        "type": "keyUp",
        "key": key.key,
        "code": key.code,
        "windowsVirtualKeyCode": key.key_code,
        "nativeVirtualKeyCode": key.key_code,
        "modifiers": key.modifiers,
    });
    vec![down, up]
}

/// How `text` is typed: a key press per ASCII character, so key handlers see each one, and
/// anything else inserted whole, as an input method would.
#[derive(Debug, PartialEq)]
pub enum Typed {
    Keys(Vec<Key>),
    Insert(String),
}

pub fn typing(text: &str) -> Vec<Typed> {
    let mut parts: Vec<Typed> = Vec::new();
    for c in text.chars() {
        let printable = c.is_ascii() && !c.is_ascii_control();
        match (printable, parts.last_mut()) {
            (true, Some(Typed::Keys(keys))) => keys.push(character(c)),
            (true, _) => parts.push(Typed::Keys(vec![character(c)])),
            (false, _) if c == '\n' => parts.push(Typed::Keys(vec![named("Enter").expect("Enter is a key")])),
            (false, Some(Typed::Insert(text))) => text.push(c),
            (false, _) => parts.push(Typed::Insert(c.to_string())),
        }
    }
    parts
}

/// The mouse events of one click (or double click) at a point.
pub fn click_events(x: f64, y: f64, button: &str, count: u32, modifiers: u32) -> Vec<Value> {
    let mut events = vec![json!({ "type": "mouseMoved", "x": x, "y": y, "modifiers": modifiers })];
    for click in 1..=count {
        let common = json!({ "x": x, "y": y, "button": button, "clickCount": click, "modifiers": modifiers, "buttons": if button == "right" { 2 } else { 1 } });
        let mut pressed = common.clone();
        pressed["type"] = json!("mousePressed");
        let mut released = common;
        released["type"] = json!("mouseReleased");
        released["buttons"] = json!(0);
        events.push(pressed);
        events.push(released);
    }
    events
}

/// A drag from one point to another, in steps a page's drag handlers can follow.
pub fn drag_events(from: (f64, f64), to: (f64, f64)) -> Vec<Value> {
    let mut events = vec![
        json!({ "type": "mouseMoved", "x": from.0, "y": from.1 }),
        json!({ "type": "mousePressed", "x": from.0, "y": from.1, "button": "left", "clickCount": 1, "buttons": 1 }),
    ];
    for step in 1..=10 {
        let t = f64::from(step) / 10.;
        events.push(json!({ "type": "mouseMoved", "x": from.0 + (to.0 - from.0) * t, "y": from.1 + (to.1 - from.1) * t, "button": "left", "buttons": 1 }));
    }
    events.push(json!({ "type": "mouseReleased", "x": to.0, "y": to.1, "button": "left", "clickCount": 1, "buttons": 0 }));
    events
}
