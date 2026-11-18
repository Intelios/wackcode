//! `browser_snapshot`'s page outline: the accessibility tree as lines, with a ref on everything
//! the agent can act on.
//!
//! The tree is Chromium's own (`Accessibility.getFullAXTree`, per frame), so it covers shadow
//! DOM and same-site frames and names things as assistive technology does. One
//! `DOMSnapshot.captureSnapshot` adds what the tree lacks: each node's attributes (for a link's
//! address and for redaction) and its box (to mark what is off screen).
//!
//! Invariants:
//! - A ref names a DOM node, so it survives snapshots and resizes and is reset only by a new
//!   page (`AgentView`).
//! - Password, one-time-code and card fields never show their value: `[redacted]`.
//! - The outline is capped (`max_chars`); a cut says how to read the rest.

use std::collections::HashMap;

use serde_json::Value;

use super::{AgentView, Node};

/// Roles the agent acts on: each gets a ref (anything focusable does too).
const ACTIONABLE: [&str; 22] = [
    "button", "link", "textbox", "searchbox", "checkbox", "radio", "combobox", "listbox", "option", "menuitem", "menuitemcheckbox",
    "menuitemradio", "tab", "switch", "slider", "spinbutton", "treeitem", "menulistoption", "popupbutton", "textfield", "togglebutton",
    "disclosuretriangle",
];
/// Roles that only hold others: left out, their children shown in their place.
const HOLDERS: [&str; 8] = ["generic", "none", "presentation", "group", "section", "div", "labeltext", "menulistpopup"];
/// Roles that hold a value worth showing.
const VALUED: [&str; 6] = ["textbox", "searchbox", "combobox", "spinbutton", "slider", "textfield"];
const MAX_NAME: usize = 150;
const MAX_TEXT: usize = 200;
pub const DEFAULT_CHARS: usize = 20_000;
pub const MOST_CHARS: usize = 60_000;

/// One frame's accessibility tree, as DevTools sent it.
pub struct FrameTree {
    /// Empty for the main frame.
    pub frame: String,
    pub url: String,
    pub nodes: Vec<Value>,
}

/// What `DOMSnapshot` told of each node, by backend node id.
#[derive(Default)]
pub struct Dom {
    pub attributes: HashMap<i64, HashMap<String, String>>,
    /// Boxes of the main frame's nodes, in page coordinates.
    pub boxes: HashMap<i64, (f64, f64, f64, f64)>,
    /// The main frame's visible area in page coordinates: x, y, width, height.
    pub visible: Option<(f64, f64, f64, f64)>,
}

impl Dom {
    /// Reads a `DOMSnapshot.captureSnapshot` answer, given the page's size in CSS pixels.
    pub fn from_snapshot(snapshot: &Value, viewport: (f64, f64)) -> Self {
        let strings: Vec<&str> = snapshot["strings"].as_array().map(|all| all.iter().map(|s| s.as_str().unwrap_or("")).collect()).unwrap_or_default();
        let text = |index: &Value| index.as_i64().and_then(|i| strings.get(i as usize).copied()).unwrap_or("");
        let mut dom = Dom::default();
        for (number, document) in snapshot["documents"].as_array().into_iter().flatten().enumerate() {
            let nodes = &document["nodes"];
            let backends: Vec<i64> = nodes["backendNodeId"].as_array().map(|ids| ids.iter().map(|id| id.as_i64().unwrap_or(0)).collect()).unwrap_or_default();
            if let Some(attributes) = nodes["attributes"].as_array() {
                for (index, pairs) in attributes.iter().enumerate() {
                    let Some(backend) = backends.get(index) else { continue };
                    let pairs = pairs.as_array().cloned().unwrap_or_default();
                    let map: HashMap<String, String> =
                        pairs.chunks(2).filter(|pair| pair.len() == 2).map(|pair| (text(&pair[0]).to_ascii_lowercase(), text(&pair[1]).to_string())).collect();
                    if !map.is_empty() {
                        dom.attributes.insert(*backend, map);
                    }
                }
            }
            // The first document is the main frame's: its boxes are in page coordinates.
            if number == 0 {
                let layout = &document["layout"];
                let indices = layout["nodeIndex"].as_array().cloned().unwrap_or_default();
                let bounds = layout["bounds"].as_array().cloned().unwrap_or_default();
                for (index, bound) in indices.iter().zip(bounds.iter()) {
                    let (Some(index), Some(bound)) = (index.as_u64(), bound.as_array()) else { continue };
                    let Some(backend) = backends.get(index as usize) else { continue };
                    let n = |i: usize| bound.get(i).and_then(Value::as_f64).unwrap_or(0.);
                    dom.boxes.insert(*backend, (n(0), n(1), n(2), n(3)));
                }
                let x = document["scrollOffsetX"].as_f64().unwrap_or(0.);
                let y = document["scrollOffsetY"].as_f64().unwrap_or(0.);
                dom.visible = Some((x, y, viewport.0, viewport.1));
            }
        }
        dom
    }

    fn off_screen(&self, backend: i64) -> bool {
        let (Some((x, y, w, h)), Some(visible)) = (self.boxes.get(&backend), self.visible) else { return false };
        if *w <= 0. && *h <= 0. {
            return false;
        }
        let (vx, vy, vw, vh) = visible;
        x + w <= vx || *x >= vx + vw || y + h <= vy || *y >= vy + vh
    }
}

/// What the agent asked to see.
#[derive(Default)]
pub struct Options {
    /// Only the part under this node (a ref's).
    pub under: Option<i64>,
    /// Only lines containing this, any case.
    pub query: Option<String>,
    pub max_chars: usize,
}

/// A field whose value is a secret: a password, a one-time code, a card number.
fn secret(attributes: Option<&HashMap<String, String>>) -> bool {
    let Some(attributes) = attributes else { return false };
    let get = |name: &str| attributes.get(name).map(|value| value.to_ascii_lowercase()).unwrap_or_default();
    if get("type") == "password" {
        return true;
    }
    let autocomplete = get("autocomplete");
    if ["one-time-code", "current-password", "new-password"].iter().any(|word| autocomplete.contains(word)) || autocomplete.contains("cc-") {
        return true;
    }
    let names = format!("{} {}", get("name"), get("id"));
    ["otp", "cvc", "cvv", "cardnumber", "card-number", "cc-number", "ccnumber", "security-code", "securitycode"].iter().any(|word| names.contains(word))
}

fn clip(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= limit {
        return text;
    }
    format!("{}…", text.chars().take(limit).collect::<String>())
}

fn quote(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn prop<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    node["properties"].as_array()?.iter().find(|p| p["name"] == name).map(|p| &p["value"]["value"])
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s == "true",
        _ => false,
    }
}

struct Render<'a> {
    /// The addresses of the frames this outline covers, to tell which frames it can't.
    frames: Vec<String>,
    dom: &'a Dom,
    view: &'a mut AgentView,
    lines: Vec<String>,
}

impl Render<'_> {
    fn frame(&mut self, tree: &FrameTree, under: Option<i64>, depth: usize) -> bool {
        let by_id: HashMap<&str, &Value> = tree.nodes.iter().filter_map(|node| Some((node["nodeId"].as_str()?, node))).collect();
        let start = match under {
            Some(backend) => tree.nodes.iter().find(|node| node["backendDOMNodeId"].as_i64() == Some(backend)),
            None => tree.nodes.iter().find(|node| node.get("parentId").is_none()).or_else(|| tree.nodes.first()),
        };
        let Some(start) = start else { return false };
        self.node(tree, &by_id, start, depth, "");
        true
    }

    fn node(&mut self, tree: &FrameTree, by_id: &HashMap<&str, &Value>, node: &Value, depth: usize, parent_name: &str) {
        let children = |node: &Value| -> Vec<&Value> {
            node["childIds"].as_array().into_iter().flatten().filter_map(|id| by_id.get(id.as_str()?).copied()).collect()
        };
        let role = node["role"]["value"].as_str().unwrap_or("").to_string();
        let lower = role.to_ascii_lowercase();
        let name = clip(node["name"]["value"].as_str().unwrap_or(""), MAX_NAME);
        let ignored = node["ignored"].as_bool().unwrap_or(false);
        if ignored || lower == "inlinetextbox" || lower == "linebreak" || (HOLDERS.contains(&lower.as_str()) && name.is_empty()) {
            for child in children(node) {
                self.node(tree, by_id, child, depth, parent_name);
            }
            return;
        }
        let indent = "  ".repeat(depth);
        if lower == "statictext" {
            let text = clip(&name, MAX_TEXT);
            if !text.is_empty() && !parent_name.contains(&text) {
                self.lines.push(format!("{indent}text \"{}\"", quote(&text)));
            }
            return;
        }
        let backend = node["backendDOMNodeId"].as_i64();
        let attributes = backend.and_then(|id| self.dom.attributes.get(&id));
        let focusable = truthy(prop(node, "focusable"));
        let actionable = ACTIONABLE.contains(&lower.as_str()) || (focusable && lower != "rootwebarea" && lower != "webarea");
        let mut line = if lower == "rootwebarea" || lower == "webarea" { format!("{indent}document") } else { format!("{indent}{role}") };
        if !name.is_empty() {
            line.push_str(&format!(" \"{}\"", quote(&name)));
        }
        if actionable && let Some(backend) = backend {
            let number = self.view.ref_for(Node { frame: tree.frame.clone(), backend });
            line.push_str(&format!(" [ref_{number}]"));
        }
        if VALUED.contains(&lower.as_str()) {
            let value = node["value"]["value"].as_str().map(str::to_string).unwrap_or_default();
            if secret(attributes) {
                if !value.is_empty() {
                    line.push_str(" value=[redacted]");
                }
            } else if !value.is_empty() {
                line.push_str(&format!(" value=\"{}\"", quote(&clip(&value, MAX_TEXT))));
            }
            if let Some(kind) = attributes.and_then(|a| a.get("type")).filter(|kind| !kind.is_empty() && kind.as_str() != "text") {
                line.push_str(&format!(" type={kind}"));
            }
        }
        if let Some(checked) = prop(node, "checked").and_then(Value::as_str) {
            line.push_str(match checked {
                "true" => " checked",
                "mixed" => " mixed",
                _ => " unchecked",
            });
        }
        for (property, word) in [("selected", " selected"), ("disabled", " disabled"), ("required", " required"), ("focused", " focused")] {
            if truthy(prop(node, property)) {
                line.push_str(word);
            }
        }
        if let Some(expanded) = prop(node, "expanded") {
            line.push_str(if truthy(Some(expanded)) { " expanded" } else { " collapsed" });
        }
        if lower == "heading"
            && let Some(level) = prop(node, "level").and_then(Value::as_i64)
        {
            line.push_str(&format!(" level={level}"));
        }
        if lower == "link"
            && let Some(href) = attributes.and_then(|a| a.get("href")).filter(|href| !href.is_empty() && !href.starts_with("javascript:"))
        {
            line.push_str(&format!(" href={}", clip(href, 120)));
        }
        // A frame from another site lives in another process, out of this tree's reach.
        if lower == "iframe" {
            let src = attributes.and_then(|a| a.get("src")).cloned().unwrap_or_default();
            if !src.is_empty() {
                line.push_str(&format!(" src={}", clip(&src, 120)));
                let read = self.frames.iter().any(|url| url == &src || url.ends_with(&src));
                if !read {
                    line.push_str(" (another site's frame: not readable here; browser_screenshot shows it)");
                }
            }
        }
        if actionable && backend.is_some_and(|backend| self.dom.off_screen(backend)) {
            line.push_str(" (off-screen)");
        }
        self.lines.push(line);
        for child in children(node) {
            self.node(tree, by_id, child, depth + 1, &name);
        }
    }
}

/// The outline of `frames` (the main frame first), giving refs through `view`.
pub fn render(frames: &[FrameTree], unreadable: &[String], dom: &Dom, view: &mut AgentView, options: &Options) -> String {
    let mut render = Render { frames: frames.iter().skip(1).map(|tree| tree.url.clone()).collect(), dom, view, lines: Vec::new() };
    let mut found = options.under.is_none();
    for (index, tree) in frames.iter().enumerate() {
        if index > 0 && options.under.is_none() {
            render.lines.push(format!("frame {}:", clip(&tree.url, 120)));
        }
        let depth = usize::from(index > 0 && options.under.is_none());
        if render.frame(tree, options.under, depth) && options.under.is_some() {
            found = true;
            break;
        }
    }
    if !found {
        return "That ref is no longer on the page: take a new snapshot.".into();
    }
    for url in unreadable {
        render.lines.push(format!("frame {} (another site's frame: not readable here; browser_screenshot shows it)", clip(url, 120)));
    }
    let mut lines = render.lines;
    let mut header = Vec::new();
    if let Some(query) = options.query.as_deref().map(str::to_lowercase).filter(|query| !query.is_empty()) {
        lines.retain(|line| line.to_lowercase().contains(&query));
        header.push(if lines.is_empty() { format!("Nothing on the page matches \"{query}\".") } else { format!("{} lines match \"{query}\":", lines.len()) });
    }
    let limit = if options.max_chars == 0 { DEFAULT_CHARS } else { options.max_chars.min(MOST_CHARS) };
    let mut out = header.join("\n");
    let mut cut = false;
    for line in lines {
        if out.len() + line.len() + 1 > limit {
            cut = true;
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
    }
    if cut {
        out.push_str(&format!("\n… cut at {limit} characters. Pass a ref to outline only that part, or query to find something."));
    }
    out
}
