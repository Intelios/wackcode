//! A canvas and its artboards, and the one rule for changing them: `plan`.
//!
//! A canvas is a board of artboards: whole HTML mockups at a real CSS size, compared side by
//! side. Only the agent's `canvas` tool changes one, and every change goes through `plan`, which
//! validates a call against the canvases the chat has and works out the canvas after it. Nothing
//! is written here; the store saves what `plan` returns (`store.rs`).
//!
//! Ported from version 2 (`worker/src/builtin/canvas/state.ts`, its rules and its errors), with:
//! - targeted `edits` (exact, unique find-and-replace) so a revision need not resend a page;
//! - tweaks: levers the agent declares and the user moves with no model call (`Tweak`);
//! - limits raised for the bigger board (`MAX_*`).
//!
//! Invariants:
//! - A variant (`based_on`) takes its base's current markup with its own `css` appended, so
//!   revising a base restyles every variant. A base is never a variant, and can't be removed
//!   while a variant depends on it.
//! - A new variant that gives no width takes its base's size at that moment.
//! - Every change bumps the artboard's `rev`; a revision keeps its place on the board.
//! - Errors are sentences for the model, naming the field that failed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_CANVAS_TITLE: usize = 60;
pub const MAX_ARTBOARD_NAME: usize = 48;
pub const MAX_ARTBOARD_GROUP: usize = 40;
pub const MAX_ARTBOARD_NOTES: usize = 200;
pub const MAX_ARTBOARD_HTML_BYTES: usize = 1024 * 1024;
pub const MAX_ARTBOARD_CSS_BYTES: usize = 128 * 1024;
pub const MAX_ARTBOARDS_PER_CALL: usize = 8;
pub const MAX_CALL_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ARTBOARDS_PER_CANVAS: usize = 40;
pub const MAX_EDITS_PER_ARTBOARD: usize = 40;
pub const MAX_TWEAKS_PER_ARTBOARD: usize = 8;
pub const MIN_ARTBOARD_SIZE: u32 = 200;
pub const MAX_ARTBOARD_WIDTH: u32 = 2560;
pub const MAX_ARTBOARD_HEIGHT: u32 = 8000;
const MAX_ID_CHARS: usize = 32;

/// One canvas: its title and artboards, in creation order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    pub id: String,
    pub title: String,
    pub artboards: Vec<Artboard>,
    /// What the latest call added or revised, which `canvas_look` checks by default.
    #[serde(default)]
    pub last_changed: Vec<String>,
}

/// One artboard. Its markup and its variant css live beside the index (`store.rs`), so the index
/// stays small; here they are loaded alongside.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Artboard {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub width: u32,
    /// `None`: as tall as its content, measured when drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tweaks: Vec<Tweak>,
    /// The user's settings of `tweaks`, by tweak id; a missing one is at its default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tweak_values: BTreeMap<String, Value>,
    /// Bumped by every change, so a picture of an older revision is never shown as this one.
    #[serde(default)]
    pub rev: u64,
    #[serde(skip)]
    pub html: Option<String>,
    #[serde(skip)]
    pub css: Option<String>,
}

/// A lever the agent declares on an artboard: the user moves it in the canvas with no model
/// call, and the page reads it as a CSS custom property on `:root` (`var`) or an attribute on
/// `<html>` (`attr`, as `data-<attr>`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tweak {
    pub id: String,
    pub label: String,
    pub kind: TweakKind,
    pub default: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub var: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attr: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TweakKind {
    Color,
    Number,
    Toggle,
    Choice,
}

impl Tweak {
    /// The value the page gets: the user's setting, else the default.
    pub fn value<'a>(&'a self, values: &'a BTreeMap<String, Value>) -> &'a Value {
        values.get(&self.id).unwrap_or(&self.default)
    }

    /// A value as CSS or an attribute takes it: a number with its unit, a toggle as `on`/`off`.
    pub fn css_value(&self, value: &Value) -> String {
        match (self.kind, value) {
            (TweakKind::Number, Value::Number(n)) => format!("{}{}", trim_number(n.as_f64().unwrap_or(0.)), self.unit.as_deref().unwrap_or("")),
            (TweakKind::Toggle, Value::Bool(on)) => if *on { "on" } else { "off" }.to_string(),
            (_, Value::String(text)) => text.clone(),
            (_, other) => other.to_string(),
        }
    }

    /// Checks a value the user set; the sentence says why one is refused.
    pub fn accepts(&self, value: &Value) -> Result<(), String> {
        match (self.kind, value) {
            (TweakKind::Color, Value::String(text)) if is_colour(text) => Ok(()),
            (TweakKind::Number, Value::Number(n)) => {
                let n = n.as_f64().unwrap_or(f64::NAN);
                let within = n.is_finite() && self.min.is_none_or(|min| n >= min) && self.max.is_none_or(|max| n <= max);
                if within { Ok(()) } else { Err(format!("{} must be a number in its range.", self.label)) }
            }
            (TweakKind::Toggle, Value::Bool(_)) => Ok(()),
            (TweakKind::Choice, Value::String(text)) if self.options.contains(text) => Ok(()),
            _ => Err(format!("That is not a value {} takes.", self.label)),
        }
    }
}

fn trim_number(n: f64) -> String {
    if n.fract() == 0. && n.abs() < 1e12 { format!("{}", n as i64) } else { format!("{n}") }
}

/// A hex colour (`#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`): the one colour form tweaks take, so a
/// value can never smuggle CSS into the page.
pub fn is_colour(text: &str) -> bool {
    let Some(hex) = text.strip_prefix('#') else { return false };
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// What one entry of a call did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    pub index: usize,
    pub id: String,
    pub op: Op,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Add,
    Update,
}

/// A call `plan` accepted.
#[derive(Clone, Debug, PartialEq)]
pub struct Planned {
    pub canvas: Canvas,
    pub created: bool,
    pub applied: Vec<Applied>,
    pub removed: Vec<String>,
    /// Comment ids the agent marked resolved, checked by the store against the canvas's own.
    pub resolve_comments: Vec<String>,
}

/// A base's markup with the artboard's own css after it: the document the artboard shows.
pub fn resolve_html(canvas: &Canvas, artboard: &Artboard) -> String {
    let base = artboard.based_on.as_ref().and_then(|id| canvas.artboards.iter().find(|a| &a.id == id));
    let html = base.map_or(artboard.html.as_deref(), |base| base.html.as_deref()).unwrap_or("");
    match artboard.css.as_deref().filter(|css| !css.trim().is_empty()) {
        Some(css) => format!("{html}\n<style>{css}</style>"),
        None => html.to_string(),
    }
}

/// How the result and `canvas_look` name an artboard to the model.
pub fn describe(artboard: &Artboard) -> String {
    let size = match artboard.height {
        Some(height) => format!("{}×{height}", artboard.width),
        None => format!("{}×auto", artboard.width),
    };
    let variant = artboard.based_on.as_ref().map(|base| format!(", variant of {base}")).unwrap_or_default();
    let group = artboard.group.as_ref().map(|group| format!(" [{group}]")).unwrap_or_default();
    format!("{} — “{}”{group} {size}{variant}", artboard.id, artboard.name)
}

/// A lowercase id from a name: letters, digits and single hyphens.
pub fn slugify(value: &str) -> String {
    let mut slug = String::new();
    for c in value.chars().flat_map(fold_accent) {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(MAX_ID_CHARS).collect();
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() { "artboard".into() } else { slug }
}

/// Common accented Latin letters to their plain letter, so "Café" becomes "cafe".
fn fold_accent(c: char) -> Vec<char> {
    let plain = match c {
        'à'..='å' | 'À'..='Å' => 'a',
        'ç' | 'Ç' => 'c',
        'è'..='ë' | 'È'..='Ë' => 'e',
        'ì'..='ï' | 'Ì'..='Ï' => 'i',
        'ñ' | 'Ñ' => 'n',
        'ò'..='ö' | 'ø' | 'Ò'..='Ö' | 'Ø' => 'o',
        'ù'..='ü' | 'Ù'..='Ü' => 'u',
        'ý' | 'ÿ' | 'Ý' => 'y',
        'ß' => return vec!['s', 's'],
        'æ' | 'Æ' => return vec!['a', 'e'],
        other => other,
    };
    vec![plain]
}

fn unique_id(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|suffix: usize| {
            let room = MAX_ID_CHARS - suffix.to_string().len() - 1;
            let stem: String = base.chars().take(room).collect();
            format!("{}-{suffix}", stem.trim_end_matches('-'))
        })
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| base.to_string())
}

fn clean(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn size_of(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024. * 1024.))
    } else {
        format!("{} KiB", bytes.div_ceil(1024))
    }
}

/// The fields one entry sets, read and checked for type.
#[derive(Default)]
struct Fields {
    name: Option<String>,
    group: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    html: Option<String>,
    based_on: Option<String>,
    css: Option<String>,
    notes: Option<String>,
    edits: Option<Vec<(String, String)>>,
    tweaks: Option<Vec<Tweak>>,
}

fn read_fields(at: &str, entry: &serde_json::Map<String, Value>) -> Result<Fields, String> {
    let string = |key: &str| -> Result<Option<String>, String> {
        match entry.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(format!("{at}.{key} must be a string.")),
        }
    };
    let pixels = |key: &str| -> Result<Option<u32>, String> {
        match entry.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(n)) if n.as_f64().is_some_and(f64::is_finite) => {
                Ok(Some(n.as_f64().unwrap_or(0.).round().clamp(0., u32::MAX as f64) as u32))
            }
            Some(_) => Err(format!("{at}.{key} must be a number of pixels.")),
        }
    };
    let edits = match entry.get("edits") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) => {
            let mut edits = Vec::new();
            for (i, edit) in list.iter().enumerate() {
                let old = edit.get("old").and_then(Value::as_str);
                let new = edit.get("new").and_then(Value::as_str);
                match (old, new) {
                    (Some(old), Some(new)) => edits.push((old.to_string(), new.to_string())),
                    _ => return Err(format!("{at}.edits[{i}] needs old and new strings.")),
                }
            }
            Some(edits)
        }
        Some(_) => return Err(format!("{at}.edits must be a list of {{old, new}}.")),
    };
    let tweaks = match entry.get("tweaks") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) => Some(read_tweaks(at, list)?),
        Some(_) => return Err(format!("{at}.tweaks must be a list.")),
    };
    Ok(Fields {
        name: string("name")?.map(|v| clean(&v)),
        group: string("group")?.map(|v| clean(&v)),
        width: pixels("width")?,
        height: pixels("height")?,
        html: string("html")?,
        based_on: string("basedOn")?,
        css: string("css")?,
        notes: string("notes")?.map(|v| clean(&v)),
        edits,
        tweaks,
    })
}

fn read_tweaks(at: &str, list: &[Value]) -> Result<Vec<Tweak>, String> {
    if list.len() > MAX_TWEAKS_PER_ARTBOARD {
        return Err(format!("{at}.tweaks: at most {MAX_TWEAKS_PER_ARTBOARD} tweaks an artboard. Keep the few levers that change the most."));
    }
    let mut tweaks: Vec<Tweak> = Vec::new();
    for (i, raw) in list.iter().enumerate() {
        let at = format!("{at}.tweaks[{i}]");
        let mut tweak: Tweak = serde_json::from_value(raw.clone()).map_err(|_| {
            format!("{at} needs id, label, kind (color, number, toggle or choice) and default, plus var or attr.")
        })?;
        tweak.label = clean(&tweak.label);
        if slugify(&tweak.id) != tweak.id {
            return Err(format!("{at}.id must be lowercase letters, digits and hyphens."));
        }
        if tweak.label.is_empty() || tweak.label.chars().count() > 32 {
            return Err(format!("{at}.label must be 1–32 characters."));
        }
        if tweaks.iter().any(|t| t.id == tweak.id) {
            return Err(format!("{at}.id “{}” is used twice.", tweak.id));
        }
        match (&tweak.var, &tweak.attr) {
            (Some(var), None) if var.starts_with("--") && var.len() > 2 && var[2..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') => {}
            (None, Some(attr)) if !attr.is_empty() && slugify(attr) == *attr => {}
            _ => return Err(format!("{at} needs exactly one of var (a custom property like \"--accent\") or attr (lowercase, set as data-<attr> on <html>).")),
        }
        if tweak.kind == TweakKind::Choice && (tweak.options.len() < 2 || tweak.options.len() > 8) {
            return Err(format!("{at}: a choice needs 2–8 options."));
        }
        if tweak.kind == TweakKind::Number && tweak.min.zip(tweak.max).is_some_and(|(min, max)| min >= max) {
            return Err(format!("{at}: min must be below max."));
        }
        if let Some(unit) = &tweak.unit
            && !["px", "rem", "em", "%", "ms", "s", "deg", ""].contains(&unit.as_str())
        {
            return Err(format!("{at}.unit must be px, rem, em, %, ms, s or deg."));
        }
        tweak.accepts(&tweak.default).map_err(|_| format!("{at}.default is not a value this tweak takes."))?;
        tweaks.push(tweak);
    }
    Ok(tweaks)
}

/// Applies exact, unique find-and-replace edits to `text`, in order.
pub fn apply_edits(at: &str, text: &str, edits: &[(String, String)]) -> Result<String, String> {
    let mut text = text.to_string();
    for (i, (old, new)) in edits.iter().enumerate() {
        if old.is_empty() {
            return Err(format!("{at}.edits[{i}].old must not be empty."));
        }
        match text.matches(old.as_str()).count() {
            0 => return Err(format!("{at}.edits[{i}].old was not found. Copy it exactly from the current markup (canvas_look with html: true shows it).")),
            1 => text = text.replacen(old.as_str(), new, 1),
            n => return Err(format!("{at}.edits[{i}].old is found {n} times. Include more of the surrounding markup so it is unique.")),
        }
    }
    Ok(text)
}

/// Validates one `canvas` call against the chat's canvases and works out the canvas after it.
pub fn plan(canvases: &[Canvas], input: &Value) -> Result<Planned, String> {
    let Some(params) = input.as_object() else {
        return Err("Expected an object with a title or a canvas id, and artboards.".into());
    };
    let title = match params.get("title") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if !clean(text).is_empty() => {
            let title = clean(text);
            if title.chars().count() > MAX_CANVAS_TITLE {
                return Err(format!("title must be at most {MAX_CANVAS_TITLE} characters."));
            }
            Some(title)
        }
        Some(_) => return Err("title must be a short non-empty string.".into()),
    };
    let base = match params.get("canvas") {
        None | Some(Value::Null) => {
            if title.is_none() {
                return Err("Give a title to start a new canvas, or the id of an existing one in canvas.".into());
            }
            None
        }
        Some(Value::String(id)) if !id.trim().is_empty() => match canvases.iter().find(|c| c.id == id.trim()) {
            Some(found) => Some(found),
            None => {
                let known: Vec<&str> = canvases.iter().map(|c| c.id.as_str()).collect();
                let hint = if known.is_empty() { " Omit canvas to start one.".to_string() } else { format!(" Canvases: {}.", known.join(", ")) };
                return Err(format!("There is no canvas “{}” in this chat.{hint}", id.trim()));
            }
        },
        Some(_) => return Err("canvas must be the id of a canvas in this chat.".into()),
    };

    let entries = match params.get("artboards") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) => list.clone(),
        Some(_) => return Err("artboards must be a list.".into()),
    };
    if entries.len() > MAX_ARTBOARDS_PER_CALL {
        return Err(format!("One call can add or change at most {MAX_ARTBOARDS_PER_CALL} artboards. Split the rest into another call."));
    }
    let removals = string_list(params.get("remove"), "remove must be a list of artboard ids.")?;
    let resolve_comments = string_list(params.get("resolve_comments"), "resolve_comments must be a list of comment ids.")?;
    if base.is_none() && !removals.is_empty() {
        return Err("A new canvas has nothing to remove.".into());
    }
    let renamed = base.is_some_and(|base| title.as_ref().is_some_and(|t| *t != base.title));
    if entries.is_empty() && removals.is_empty() && resolve_comments.is_empty() && !renamed {
        return Err(if base.is_some() {
            "Nothing to change: add or revise artboards, remove some, resolve comments, or give a new title.".into()
        } else {
            "A new canvas needs at least one artboard.".into()
        });
    }

    let id = match base {
        Some(base) => base.id.clone(),
        None => unique_id(&slugify(title.as_deref().unwrap_or("canvas")), |c| canvases.iter().any(|canvas| canvas.id == c)),
    };
    let mut canvas = Canvas {
        id,
        title: title.clone().or_else(|| base.map(|b| b.title.clone())).unwrap_or_default(),
        artboards: base.map(|b| b.artboards.clone()).unwrap_or_default(),
        last_changed: Vec::new(),
    };
    let find = |canvas: &Canvas, wanted: &str| -> Option<String> {
        let wanted = wanted.trim();
        canvas
            .artboards
            .iter()
            .find(|a| a.id == wanted)
            .or_else(|| canvas.artboards.iter().find(|a| a.name.eq_ignore_ascii_case(wanted)))
            .map(|a| a.id.clone())
    };

    let mut applied: Vec<Applied> = Vec::new();
    let mut call_bytes = 0usize;
    for (index, raw) in entries.iter().enumerate() {
        let at = format!("artboards[{index}]");
        let Some(entry) = raw.as_object() else {
            return Err(format!("{at} must be an object."));
        };
        let reference = match entry.get("id") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) if !id.trim().is_empty() => Some(id.trim().to_string()),
            Some(_) => return Err(format!("{at}.id must be a string.")),
        };
        let existing = match &reference {
            Some(id) => match canvas.artboards.iter().position(|a| &a.id == id) {
                Some(position) => Some(position),
                None => return Err(format!("There is no artboard “{id}” on “{}”. Omit id to add a new one.", canvas.title)),
            },
            None => None,
        };
        if let Some(position) = existing
            && applied.iter().any(|a| a.id == canvas.artboards[position].id)
        {
            return Err(format!("{at} changes “{}” a second time in one call.", canvas.artboards[position].id));
        }
        let mut fields = read_fields(&at, entry)?;
        if let Some(name) = &fields.name {
            if name.is_empty() {
                return Err(format!("{at}.name must not be empty."));
            }
            if name.chars().count() > MAX_ARTBOARD_NAME {
                return Err(format!("{at}.name must be at most {MAX_ARTBOARD_NAME} characters."));
            }
        }
        if fields.group.as_ref().is_some_and(|g| g.chars().count() > MAX_ARTBOARD_GROUP) {
            return Err(format!("{at}.group must be at most {MAX_ARTBOARD_GROUP} characters."));
        }
        if fields.notes.as_ref().is_some_and(|n| n.chars().count() > MAX_ARTBOARD_NOTES) {
            return Err(format!("{at}.notes must be at most {MAX_ARTBOARD_NOTES} characters."));
        }
        if fields.width.is_some_and(|w| !(MIN_ARTBOARD_SIZE..=MAX_ARTBOARD_WIDTH).contains(&w)) {
            return Err(format!("{at}.width must be between {MIN_ARTBOARD_SIZE} and {MAX_ARTBOARD_WIDTH} pixels."));
        }
        if fields.height.is_some_and(|h| !(MIN_ARTBOARD_SIZE..=MAX_ARTBOARD_HEIGHT).contains(&h)) {
            return Err(format!("{at}.height must be between {MIN_ARTBOARD_SIZE} and {MAX_ARTBOARD_HEIGHT} pixels, or omitted to fit the content."));
        }
        if fields.html.as_ref().is_some_and(|h| h.trim().is_empty()) {
            return Err(format!("{at}.html must not be empty."));
        }
        if fields.html.is_some() && fields.based_on.is_some() {
            return Err(format!("{at} gives both html and basedOn. A variant takes its base's markup; give it css instead."));
        }
        if fields.edits.is_some() && (fields.html.is_some() || fields.css.is_some()) {
            return Err(format!("{at} gives edits with html or css. Send the whole markup, or edits to the current one, not both."));
        }
        if fields.edits.as_ref().is_some_and(|e| e.len() > MAX_EDITS_PER_ARTBOARD) {
            return Err(format!("{at}.edits: at most {MAX_EDITS_PER_ARTBOARD} edits an artboard. Send the whole markup instead."));
        }

        let mut next = match existing {
            Some(position) => canvas.artboards[position].clone(),
            None => {
                let Some(name) = fields.name.clone() else {
                    return Err(format!("{at} needs a name."));
                };
                if fields.width.is_none() && fields.based_on.is_none() {
                    return Err(format!("{at} needs a width, e.g. 1440 for desktop or 390 for a phone."));
                }
                if fields.html.is_none() && fields.based_on.is_none() {
                    return Err(format!("{at} needs html, or basedOn to make it a variant of another artboard."));
                }
                if fields.edits.is_some() {
                    return Err(format!("{at} adds an artboard, which has no markup to edit yet. Give it html."));
                }
                let id = unique_id(&slugify(&name), |c| canvas.artboards.iter().any(|a| a.id == c));
                Artboard { id, name, width: 0, ..Artboard::default() }
            }
        };

        if let Some(wanted) = fields.based_on.take() {
            let Some(target) = find(&canvas, &wanted) else {
                return Err(format!("{at}.basedOn names “{wanted}”, which is not on “{}”.", canvas.title));
            };
            if target == next.id {
                return Err(format!("{at} can't be a variant of itself."));
            }
            let target_board = canvas.artboards.iter().find(|a| a.id == target).cloned().unwrap_or_default();
            if let Some(grand) = &target_board.based_on {
                return Err(format!("{at}.basedOn names “{target}”, which is itself a variant. Base it on “{grand}” instead."));
            }
            if canvas.artboards.iter().any(|a| a.based_on.as_deref() == Some(next.id.as_str())) {
                return Err(format!("“{}” is the base of other variants, so it can't become a variant itself.", next.id));
            }
            // A new variant that gives no width is drawn at its base's whole size.
            if existing.is_none() && fields.width.is_none() {
                fields.width = Some(target_board.width);
                if fields.height.is_none() {
                    fields.height = target_board.height;
                }
            }
            next.based_on = Some(target);
            next.html = None;
        }
        if let Some(html) = fields.html.take() {
            next.html = Some(html);
            next.based_on = None;
        }
        if let Some(edits) = &fields.edits {
            if next.based_on.is_some() {
                let css = next.css.clone().unwrap_or_default();
                next.css = Some(apply_edits(&at, &css, edits)?);
            } else {
                let html = next.html.clone().unwrap_or_default();
                next.html = Some(apply_edits(&at, &html, edits)?);
            }
        }
        if let Some(css) = fields.css.take() {
            next.css = Some(css).filter(|css| !css.trim().is_empty());
        }
        let html_bytes = next.html.as_ref().map_or(0, String::len);
        let css_bytes = next.css.as_ref().map_or(0, String::len);
        if html_bytes > MAX_ARTBOARD_HTML_BYTES {
            return Err(format!(
                "{at}.html is {}, over the {} limit. Draw it with less markup, or move shared styles into a base artboard.",
                size_of(html_bytes),
                size_of(MAX_ARTBOARD_HTML_BYTES)
            ));
        }
        if css_bytes > MAX_ARTBOARD_CSS_BYTES {
            return Err(format!("{at}.css is over the {} limit.", size_of(MAX_ARTBOARD_CSS_BYTES)));
        }
        call_bytes += entry.get("html").and_then(Value::as_str).map_or(0, str::len)
            + entry.get("css").and_then(Value::as_str).map_or(0, str::len);

        if let Some(name) = fields.name {
            next.name = name;
        }
        if let Some(group) = fields.group {
            next.group = Some(group).filter(|g| !g.is_empty());
        }
        if let Some(notes) = fields.notes {
            next.notes = Some(notes).filter(|n| !n.is_empty());
        }
        if let Some(width) = fields.width {
            next.width = width;
        }
        // A new artboard takes its height (or its base's); a revision changes it only when given.
        if existing.is_none() || entry.contains_key("height") {
            next.height = fields.height;
        }
        if let Some(tweaks) = fields.tweaks {
            // A setting the user made survives while its tweak does and still takes it.
            next.tweak_values.retain(|id, value| tweaks.iter().any(|t| &t.id == id && t.accepts(value).is_ok()));
            next.tweaks = tweaks;
        }
        next.rev += 1;
        let op = if existing.is_some() { Op::Update } else { Op::Add };
        applied.push(Applied { index, id: next.id.clone(), op });
        match existing {
            Some(position) => canvas.artboards[position] = next,
            None => canvas.artboards.push(next),
        }
    }
    if call_bytes > MAX_CALL_BYTES {
        return Err(format!(
            "This call carries {} of markup, over the {} limit. Split it into smaller calls.",
            size_of(call_bytes),
            size_of(MAX_CALL_BYTES)
        ));
    }

    let mut removed: Vec<String> = Vec::new();
    for wanted in &removals {
        let Some(target) = find(&canvas, wanted) else {
            return Err(format!("remove names “{wanted}”, which is not on “{}”.", canvas.title));
        };
        if applied.iter().any(|a| a.id == target) {
            return Err(format!("“{target}” is both changed and removed in one call."));
        }
        if !removed.contains(&target) {
            removed.push(target);
        }
    }
    canvas.artboards.retain(|a| !removed.contains(&a.id));
    if let Some(orphan) = canvas.artboards.iter().find(|a| a.based_on.as_ref().is_some_and(|b| removed.contains(b))) {
        return Err(format!(
            "“{}” can't be removed while “{}” is a variant of it. Remove or rebase that variant too.",
            orphan.based_on.as_deref().unwrap_or_default(),
            orphan.id
        ));
    }
    if canvas.artboards.len() > MAX_ARTBOARDS_PER_CANVAS {
        return Err(format!("A canvas holds at most {MAX_ARTBOARDS_PER_CANVAS} artboards. Remove some, or start a new canvas for the next round."));
    }
    // A revised base restyles its variants: they count as changed too, for pictures and looks.
    let mut changed: Vec<String> = applied.iter().map(|a| a.id.clone()).collect();
    for artboard in &mut canvas.artboards {
        if let Some(base) = &artboard.based_on
            && changed.contains(base)
            && !changed.contains(&artboard.id)
        {
            artboard.rev += 1;
            changed.push(artboard.id.clone());
        }
    }
    canvas.last_changed = applied.iter().map(|a| a.id.clone()).collect();
    if canvas.last_changed.is_empty() {
        canvas.last_changed = base.map(|b| b.last_changed.clone()).unwrap_or_default();
    }
    Ok(Planned { canvas, created: base.is_none(), applied, removed, resolve_comments })
}

fn string_list(value: Option<&Value>, error: &str) -> Result<Vec<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(list)) => list.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| error.to_string())).collect(),
        Some(_) => Err(error.to_string()),
    }
}

/// Every artboard whose picture `planned` made stale: the ones it changed and their variants.
pub fn stale(planned: &Planned) -> Vec<String> {
    let changed: Vec<&str> = planned.applied.iter().map(|a| a.id.as_str()).collect();
    planned
        .canvas
        .artboards
        .iter()
        .filter(|a| changed.contains(&a.id.as_str()) || a.based_on.as_deref().is_some_and(|b| changed.contains(&b)))
        .map(|a| a.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn applied(canvases: &[Canvas], input: Value) -> Planned {
        plan(canvases, &input).unwrap_or_else(|e| panic!("{e}"))
    }

    fn refused(canvases: &[Canvas], input: Value) -> String {
        plan(canvases, &input).expect_err("should be refused")
    }

    fn start() -> Canvas {
        applied(&[], json!({
            "title": "Settings directions",
            "artboards": [
                { "name": "Base", "group": "Themes", "width": 1280, "height": 800, "html": "<main>a</main>" },
                { "name": "Ember", "group": "Themes", "basedOn": "base", "css": ":root{--bg:#000}" },
                { "name": "Phone", "width": 390, "basedOn": "Base" }
            ]
        }))
        .canvas
    }

    #[test]
    fn a_new_canvas_takes_its_titles_slug_and_variants_inherit_size() {
        let canvas = start();
        assert_eq!(canvas.id, "settings-directions");
        let ids: Vec<&str> = canvas.artboards.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["base", "ember", "phone"]);
        assert_eq!((canvas.artboards[1].width, canvas.artboards[1].height), (1280, Some(800)));
        assert_eq!((canvas.artboards[2].width, canvas.artboards[2].height), (390, None), "a new width is a device size of its own");
        assert_eq!(canvas.artboards[2].based_on.as_deref(), Some("base"), "named by name, kept by id");
        assert_eq!(resolve_html(&canvas, &canvas.artboards[1]), "<main>a</main>\n<style>:root{--bg:#000}</style>");
        assert!(canvas.artboards.iter().all(|a| a.rev == 1));
    }

    #[test]
    fn revising_a_base_keeps_places_and_marks_its_variants_stale() {
        let canvas = start();
        let planned = applied(&[canvas], json!({ "canvas": "settings-directions", "artboards": [{ "id": "base", "html": "<main>b</main>" }] }));
        assert_eq!(planned.applied, vec![Applied { index: 0, id: "base".into(), op: Op::Update }]);
        assert_eq!(stale(&planned), ["base", "ember", "phone"]);
        assert_eq!(planned.canvas.artboards[0].rev, 2);
        assert_eq!(planned.canvas.artboards[1].rev, 2, "a variant's picture changes with its base");
        assert_eq!(planned.canvas.last_changed, ["base"]);
        assert_eq!(planned.canvas.artboards[0].name, "Base", "fields not given stay");
    }

    #[test]
    fn edits_change_only_what_they_name_and_must_match_once() {
        let canvas = applied(&[], json!({ "title": "T", "artboards": [{ "name": "A", "width": 400, "html": "<h1>Hi</h1><p>x</p><p>x</p>" }] })).canvas;
        let canvases = [canvas];
        let planned = applied(&canvases, json!({ "canvas": "t", "artboards": [{ "id": "a", "edits": [{ "old": "Hi", "new": "Hello" }] }] }));
        assert_eq!(planned.canvas.artboards[0].html.as_deref(), Some("<h1>Hello</h1><p>x</p><p>x</p>"));
        assert!(refused(&canvases, json!({ "canvas": "t", "artboards": [{ "id": "a", "edits": [{ "old": "<p>x</p>", "new": "" }] }] })).contains("found 2 times"));
        assert!(refused(&canvases, json!({ "canvas": "t", "artboards": [{ "id": "a", "edits": [{ "old": "nope", "new": "" }] }] })).contains("not found"));
        assert!(refused(&canvases, json!({ "canvas": "t", "artboards": [{ "id": "a", "html": "<p/>", "edits": [{ "old": "Hi", "new": "" }] }] })).contains("not both"));
    }

    #[test]
    fn a_variants_edits_change_its_css() {
        let canvas = start();
        let planned = applied(&[canvas], json!({ "canvas": "settings-directions", "artboards": [{ "id": "ember", "edits": [{ "old": "#000", "new": "#111" }] }] }));
        assert_eq!(planned.canvas.artboards[1].css.as_deref(), Some(":root{--bg:#111}"));
    }

    #[test]
    fn version_twos_refusals_hold() {
        let canvases = [start()];
        let c = "settings-directions";
        assert!(refused(&[], json!({ "artboards": [] })).contains("Give a title"));
        assert!(refused(&[], json!({ "title": "T" })).contains("at least one artboard"));
        assert!(refused(&canvases, json!({ "canvas": "nope", "artboards": [] })).contains("Canvases: settings-directions."));
        assert!(refused(&canvases, json!({ "canvas": c })).contains("Nothing to change"));
        assert!(refused(&canvases, json!({ "canvas": c, "artboards": [{ "name": "X", "html": "<p/>" }] })).contains("needs a width"));
        assert!(refused(&canvases, json!({ "canvas": c, "artboards": [{ "name": "X", "width": 100, "html": "<p/>" }] })).contains("between 200 and 2560"));
        assert!(refused(&canvases, json!({ "canvas": c, "artboards": [{ "name": "X", "width": 400, "height": 9000, "html": "<p/>" }] })).contains("8000"));
        assert!(refused(&canvases, json!({ "canvas": c, "artboards": [{ "name": "X", "basedOn": "ember" }] })).contains("itself a variant"));
        assert!(refused(&canvases, json!({ "canvas": c, "artboards": [{ "id": "base", "basedOn": "phone" }] })).contains("is itself a variant"));
        assert!(refused(&canvases, json!({ "canvas": c, "remove": ["base"] })).contains("can't be removed while"));
        assert!(refused(&canvases, json!({ "canvas": c, "artboards": [{ "id": "base", "name": "B" }], "remove": ["base"] })).contains("both changed and removed"));
        assert!(refused(&canvases, json!({ "canvas": c, "artboards": [{ "id": "base", "html": "<a/>" }, { "id": "base", "name": "Q" }] })).contains("second time"));
        assert!(refused(&canvases, json!({ "canvas": c, "artboards": [{ "name": "X", "width": 400, "html": "<p/>", "basedOn": "base" }] })).contains("both html and basedOn"));
    }

    #[test]
    fn removing_a_variant_then_its_base_works() {
        let canvases = [start()];
        let planned = applied(&canvases, json!({ "canvas": "settings-directions", "remove": ["Ember", "phone", "base"] }));
        assert!(planned.canvas.artboards.is_empty());
        assert_eq!(planned.removed, ["ember", "phone", "base"]);
    }

    #[test]
    fn ids_are_unique_slugs() {
        assert_eq!(slugify("Café — Dusk!"), "cafe-dusk");
        assert_eq!(slugify("???"), "artboard");
        let planned = applied(&[], json!({ "title": "T", "artboards": [
            { "name": "Dusk", "width": 400, "html": "<p/>" },
            { "name": "dusk", "width": 400, "html": "<p/>" }
        ] }));
        let ids: Vec<&str> = planned.canvas.artboards.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["dusk", "dusk-2"]);
        let again = applied(&[planned.canvas], json!({ "title": "T", "artboards": [{ "name": "A", "width": 400, "html": "<p/>" }] }));
        assert_eq!(again.canvas.id, "t-2", "a second canvas of the same title gets its own id");
    }

    #[test]
    fn tweaks_are_checked_and_keep_the_users_settings() {
        let tweaks = json!([
            { "id": "accent", "label": "Accent", "kind": "color", "default": "#2f6df6", "var": "--accent" },
            { "id": "density", "label": "Density", "kind": "choice", "default": "cosy", "options": ["cosy", "compact"], "attr": "density" },
            { "id": "radius", "label": "Radius", "kind": "number", "default": 8, "min": 0, "max": 24, "unit": "px", "var": "--radius" }
        ]);
        let mut canvas = applied(&[], json!({ "title": "T", "artboards": [{ "name": "A", "width": 400, "html": "<p/>", "tweaks": tweaks }] })).canvas;
        let artboard = &mut canvas.artboards[0];
        assert_eq!(artboard.tweaks.len(), 3);
        assert_eq!(artboard.tweaks[2].css_value(&json!(8)), "8px");
        artboard.tweak_values.insert("accent".into(), json!("#ff0000"));
        artboard.tweak_values.insert("radius".into(), json!(12));
        let canvases = [canvas];
        let planned = applied(&canvases, json!({ "canvas": "t", "artboards": [{ "id": "a", "tweaks": [
            { "id": "accent", "label": "Accent", "kind": "color", "default": "#000", "var": "--accent" }
        ] }] }));
        assert_eq!(planned.canvas.artboards[0].tweak_values, BTreeMap::from([("accent".to_string(), json!("#ff0000"))]));

        for (bad, says) in [
            (json!([{ "id": "x", "label": "X", "kind": "color", "default": "red", "var": "--x" }]), "default"),
            (json!([{ "id": "x", "label": "X", "kind": "color", "default": "#fff", "var": "x" }]), "exactly one of var"),
            (json!([{ "id": "x", "label": "X", "kind": "choice", "default": "a", "options": ["a"], "attr": "x" }]), "2–8 options"),
            (json!([{ "id": "X Y", "label": "X", "kind": "toggle", "default": true, "attr": "x" }]), "lowercase"),
            (json!([{ "id": "x", "label": "X", "kind": "number", "default": 1, "unit": "vh", "var": "--x" }]), "unit"),
        ] {
            let error = refused(&[], json!({ "title": "T", "artboards": [{ "name": "A", "width": 400, "html": "<p/>", "tweaks": bad }] }));
            assert!(error.contains(says), "{error}");
        }
    }

    #[test]
    fn tweak_values_are_checked_and_colours_carry_no_css() {
        let tweak = Tweak { id: "a".into(), label: "Accent".into(), kind: TweakKind::Color, default: json!("#000"), options: vec![], min: None, max: None, step: None, unit: None, var: Some("--a".into()), attr: None };
        assert!(tweak.accepts(&json!("#12abEF")).is_ok());
        assert!(tweak.accepts(&json!("red; background:url(x)")).is_err());
        assert!(tweak.accepts(&json!(3)).is_err());
    }
}
