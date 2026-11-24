//! Renders an accessibility tree as the outline `computer_snapshot` returns and the element list
//! that comes with a `computer_screenshot`. Pure: the tree comes from `native::ax` (or a test), so
//! folding, filtering, capping and refs are unit-tested without Accessibility. Ported from version
//! 2's `wackdev/src/outline.rs`.
//!
//! Invariants:
//! - One line per element: `[e12] button "Save" (disabled)`. Unlabelled layout containers fold
//!   away (their children move up), and a static text repeating its parent's label is dropped.
//! - Refs come from the caller (`Refs`): the engine gives each element one number for as long as
//!   it exists, so a ref survives later looks. Version 2 renumbered every look, which killed
//!   every ref the model held.
//! - Only refs on printed lines are reported as shown; the tools refuse a ref the model was never
//!   shown.
//! - Secure text fields never carry a value: `native::ax` never reads one, and a value that got
//!   here anyway is not printed.
//! - Text that came from the app is data: titles and values are clipped to one line here, and the
//!   tool wraps the whole outline in a data-only marker.

use crate::geometry::{self, Rect};

pub const MAX_OUTLINE_BYTES: usize = 24 * 1024;
const MAX_LABEL: usize = 120;
const MAX_VALUE: usize = 200;
/// The most elements listed under a screenshot.
pub const MAX_LISTED: usize = 60;

/// Containers that only arrange other elements. Without a label they are folded away.
const CONTAINERS: &[&str] = &[
    "AXGroup", "AXSplitGroup", "AXLayoutArea", "AXLayoutItem", "AXScrollArea", "AXUnknown", "AXGrowArea", "AXMatte", "AXRuler",
    "AXRulerMarker", "AXSplitter",
];

/// Subroles more telling than their role.
const SUBROLES: &[&str] = &[
    "AXSecureTextField", "AXSearchField", "AXCloseButton", "AXMinimizeButton", "AXZoomButton", "AXFullScreenButton", "AXToggle", "AXSwitch",
    "AXTabButton", "AXSortButton",
];

/// Roles a person can act on, which a screenshot's element list names.
const ACTIONABLE: &[&str] = &[
    "AXButton", "AXTextField", "AXTextArea", "AXComboBox", "AXLink", "AXRadioButton", "AXCheckBox", "AXPopUpButton", "AXMenuButton",
    "AXDisclosureTriangle", "AXMenuItem", "AXSlider", "AXIncrementor", "AXTab", "AXSegmentedControl",
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    /// Index of this element in the walk's element list; what a ref is given for.
    pub handle: usize,
    pub role: String,
    pub subrole: Option<String>,
    pub title: Option<String>,
    /// Never set for secure text fields.
    pub value: Option<String>,
    pub description: Option<String>,
    pub placeholder: Option<String>,
    pub enabled: Option<bool>,
    pub focused: bool,
    pub selected: bool,
    /// Global screen points, when the element reports a frame.
    pub frame: Option<Rect>,
    pub children: Vec<Node>,
    /// Children the walk left out (its per-element cap), shown as "… N more".
    pub omitted_children: usize,
}

impl Node {
    pub fn is_secure(&self) -> bool {
        self.subrole.as_deref() == Some("AXSecureTextField")
    }

    /// Every node in the tree, depth first.
    pub fn each(&self, visit: &mut dyn FnMut(&Node)) {
        visit(self);
        for child in &self.children {
            child.each(visit);
        }
    }

    pub fn find(&self, handle: usize) -> Option<&Node> {
        if self.handle == handle {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(handle))
    }
}

/// Gives each element its ref number.
pub type Refs<'a> = &'a mut dyn FnMut(usize) -> u32;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outline {
    pub text: String,
    /// The refs on printed lines.
    pub shown: Vec<u32>,
    pub truncated: bool,
    /// Lines a `query` left out.
    pub filtered: usize,
}

/// `"AXStaticText"` → `"staticText"`.
pub fn display_role(role: &str, subrole: Option<&str>) -> String {
    let raw = subrole.filter(|subrole| SUBROLES.contains(subrole)).unwrap_or(role);
    let name = raw.strip_prefix("AX").unwrap_or(raw);
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => "element".into(),
    }
}

pub fn clip(text: &str, max: usize) -> String {
    let single_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= max { single_line } else { format!("{}…", single_line.chars().take(max).collect::<String>()) }
}

fn nonempty(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|text| !text.is_empty())
}

pub fn label(node: &Node) -> Option<&str> {
    nonempty(&node.title).or_else(|| nonempty(&node.description))
}

fn is_folded(node: &Node) -> bool {
    CONTAINERS.contains(&node.role.as_str()) && label(node).is_none() && nonempty(&node.value).is_none() && !node.focused && !node.selected
}

fn is_checkable(node: &Node) -> bool {
    matches!(node.role.as_str(), "AXCheckBox" | "AXRadioButton") || matches!(node.subrole.as_deref(), Some("AXToggle" | "AXSwitch"))
}

pub fn is_actionable(node: &Node) -> bool {
    ACTIONABLE.contains(&node.role.as_str())
}

/// What an element is, without its ref: `button "Save" (disabled)`.
pub fn describe(node: &Node) -> String {
    let mut text = display_role(&node.role, node.subrole.as_deref());
    let is_static = node.role == "AXStaticText";
    match (is_static, label(node), nonempty(&node.value)) {
        // A static text's value is its text.
        (true, None, Some(value)) if !node.is_secure() => text.push_str(&format!(" \"{}\"", clip(value, MAX_VALUE))),
        (_, Some(label), _) => text.push_str(&format!(" \"{}\"", clip(label, MAX_LABEL))),
        _ => {}
    }
    if node.is_secure() {
        text.push_str(" (secure)");
    } else if is_checkable(node) {
        match nonempty(&node.value) {
            Some("1") => text.push_str(" (checked)"),
            Some("0") => text.push_str(" (unchecked)"),
            Some("2") => text.push_str(" (mixed)"),
            _ => {}
        }
    } else if let Some(value) = nonempty(&node.value).filter(|_| !is_static || label(node).is_some()) {
        text.push_str(&format!(" value=\"{}\"", clip(value, MAX_VALUE)));
    }
    if label(node).is_none()
        && let Some(placeholder) = nonempty(&node.placeholder)
    {
        text.push_str(&format!(" placeholder=\"{}\"", clip(placeholder, MAX_LABEL)));
    }
    let flags = [(node.enabled == Some(false), "disabled"), (node.focused, "focused"), (node.selected, "selected")]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>();
    if !flags.is_empty() {
        text.push_str(&format!(" ({})", flags.join(", ")));
    }
    text
}

struct Line {
    depth: usize,
    text: String,
    reference: Option<u32>,
}

fn collect(node: &Node, depth: usize, parent_label: Option<&str>, refs: Refs<'_>, lines: &mut Vec<Line>) {
    let redundant_text =
        node.role == "AXStaticText" && node.children.is_empty() && parent_label.is_some() && nonempty(&node.value).or_else(|| label(node)) == parent_label;
    if redundant_text {
        return;
    }
    let (child_depth, own_label) = if is_folded(node) {
        (depth, parent_label)
    } else {
        let reference = refs(node.handle);
        lines.push(Line { depth, text: format!("[e{reference}] {}", describe(node)), reference: Some(reference) });
        (depth + 1, label(node))
    };
    for child in &node.children {
        collect(child, child_depth, own_label, refs, lines);
    }
    if node.omitted_children > 0 {
        lines.push(Line { depth: child_depth, text: format!("… {} more", node.omitted_children), reference: None });
    }
}

/// Renders the tree under `root`, keeping only lines containing `query` (any case) when given,
/// capped at `max_bytes`.
pub fn render(root: &Node, refs: Refs<'_>, query: Option<&str>, max_bytes: usize) -> Outline {
    let mut lines = Vec::new();
    collect(root, 0, None, refs, &mut lines);
    let query = query.map(str::trim).filter(|query| !query.is_empty()).map(str::to_lowercase);
    let mut outline = Outline::default();
    for line in lines {
        if let Some(query) = &query
            && (line.reference.is_none() || !line.text.to_lowercase().contains(query))
        {
            outline.filtered += usize::from(line.reference.is_some());
            continue;
        }
        // A filtered outline is flat: its lines' parents may be gone.
        let depth = if query.is_some() { 0 } else { line.depth };
        let needed = depth * 2 + line.text.len() + 1;
        if outline.text.len() + needed > max_bytes {
            outline.truncated = true;
            break;
        }
        outline.text.push_str(&"  ".repeat(depth));
        outline.text.push_str(&line.text);
        outline.text.push('\n');
        outline.shown.extend(line.reference);
    }
    outline
}

/// The actionable elements visible in a window picture, one per line with its centre in the
/// picture's frame: `[e12] button "Save" at (420, 36)`. `window` is the window's frame in screen
/// points and `frame` the picture's frame. At most `max` lines; the rest are counted.
pub fn element_list(root: &Node, refs: Refs<'_>, window: Rect, frame: (u32, u32), max: usize) -> Outline {
    let mut found: Vec<(usize, String, (i64, i64))> = Vec::new();
    let mut extra = 0;
    root.each(&mut |node| {
        if !is_actionable(node) {
            return;
        }
        let Some(rect) = node.frame.filter(Rect::has_area) else { return };
        let (cx, cy) = rect.center();
        if !window.contains(cx, cy) {
            return;
        }
        if found.len() >= max {
            extra += 1;
            return;
        }
        let point = geometry::to_frame((window.width, window.height), frame, cx - window.x, cy - window.y);
        found.push((node.handle, describe(node), point));
    });
    let mut outline = Outline::default();
    for (handle, text, (x, y)) in found {
        let reference = refs(handle);
        outline.text.push_str(&format!("[e{reference}] {text} at ({x}, {y})\n"));
        outline.shown.push(reference);
    }
    if extra > 0 {
        outline.truncated = true;
        outline.text.push_str(&format!("… {extra} more (computer_snapshot lists them all)\n"));
    }
    outline
}

/// The ref number in `"e12"` (or `"[e12]"`).
pub fn parse_ref(reference: &str) -> Option<u32> {
    let trimmed = reference.trim().trim_start_matches('[').trim_end_matches(']');
    trimmed.strip_prefix('e').or_else(|| trimmed.strip_prefix('E'))?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(handle: usize, role: &str) -> Node {
        Node { handle, role: role.into(), ..Node::default() }
    }

    /// Refs that keep their numbers across calls, as the engine's do.
    fn stable() -> impl FnMut(usize) -> u32 {
        let mut given = std::collections::HashMap::new();
        move |handle| {
            let next = given.len() as u32 + 1;
            *given.entry(handle).or_insert(next)
        }
    }

    fn sample() -> Node {
        let mut window = node(0, "AXWindow");
        window.title = Some("Untitled".into());
        let mut group = node(1, "AXGroup");
        let mut save = node(2, "AXButton");
        save.title = Some("Save".into());
        save.frame = Some(Rect::new(110.0, 120.0, 60.0, 20.0));
        let mut save_text = node(3, "AXStaticText");
        save_text.value = Some("Save".into());
        save.children.push(save_text);
        let mut field = node(4, "AXTextField");
        field.value = Some("hello".into());
        field.focused = true;
        field.frame = Some(Rect::new(100.0, 200.0, 200.0, 24.0));
        let mut secret = node(5, "AXTextField");
        secret.subrole = Some("AXSecureTextField".into());
        let mut check = node(6, "AXCheckBox");
        check.title = Some("Remember".into());
        check.value = Some("1".into());
        check.enabled = Some(false);
        check.frame = Some(Rect::new(5000.0, 5000.0, 10.0, 10.0));
        group.children = vec![save, field, secret, check];
        group.omitted_children = 3;
        window.children.push(group);
        window
    }

    #[test]
    fn renders_a_folded_labelled_outline_with_refs() {
        let mut refs = stable();
        let outline = render(&sample(), &mut refs, None, MAX_OUTLINE_BYTES);
        assert_eq!(
            outline.text,
            "[e1] window \"Untitled\"\n  [e2] button \"Save\"\n  [e3] textField value=\"hello\" (focused)\n  [e4] secureTextField (secure)\n  [e5] checkBox \"Remember\" (checked) (disabled)\n  … 3 more\n"
        );
        assert_eq!(outline.shown, vec![1, 2, 3, 4, 5]);
        assert!(!outline.truncated);
    }

    #[test]
    fn refs_survive_a_second_look() {
        let mut refs = stable();
        let first = render(&sample(), &mut refs, None, MAX_OUTLINE_BYTES);
        let mut changed = sample();
        // A new element appears before the others; theirs keep their numbers.
        let mut new = node(9, "AXButton");
        new.title = Some("New".into());
        changed.children[0].children.insert(0, new);
        let second = render(&changed, &mut refs, None, MAX_OUTLINE_BYTES);
        assert!(first.text.contains("[e2] button \"Save\""));
        assert!(second.text.contains("[e2] button \"Save\""));
        assert!(second.text.contains("[e6] button \"New\""));
    }

    #[test]
    fn a_query_keeps_matching_lines_flat_and_counts_the_rest() {
        let mut refs = stable();
        let outline = render(&sample(), &mut refs, Some("SAVE"), MAX_OUTLINE_BYTES);
        assert_eq!(outline.text, "[e2] button \"Save\"\n");
        assert_eq!(outline.shown, vec![2]);
        assert_eq!(outline.filtered, 4);
    }

    #[test]
    fn secure_fields_never_show_a_value_even_if_one_is_present() {
        let mut secret = node(0, "AXTextField");
        secret.subrole = Some("AXSecureTextField".into());
        secret.value = Some("hunter2".into());
        let mut refs = stable();
        assert!(!render(&secret, &mut refs, None, MAX_OUTLINE_BYTES).text.contains("hunter2"));
        let mut as_text = secret.clone();
        as_text.role = "AXStaticText".into();
        assert!(!render(&as_text, &mut refs, None, MAX_OUTLINE_BYTES).text.contains("hunter2"));
    }

    #[test]
    fn caps_output_and_marks_it_truncated() {
        let mut root = node(0, "AXList");
        root.title = Some("Items".into());
        for index in 1..500 {
            let mut item = node(index, "AXStaticText");
            item.value = Some(format!("Row number {index}"));
            root.children.push(item);
        }
        let mut refs = stable();
        let outline = render(&root, &mut refs, None, 1024);
        assert!(outline.truncated);
        assert!(outline.text.len() <= 1024);
        assert_eq!(outline.shown.len(), outline.text.lines().count());
    }

    #[test]
    fn long_labels_are_clipped_to_one_line() {
        let mut text = node(0, "AXStaticText");
        text.value = Some(format!("first\nsecond {}", "x".repeat(400)));
        let mut refs = stable();
        let outline = render(&text, &mut refs, None, MAX_OUTLINE_BYTES);
        assert_eq!(outline.text.lines().count(), 1);
        assert!(outline.text.contains("first second"));
        assert!(outline.text.trim_end().ends_with("…\""));
    }

    #[test]
    fn the_element_list_names_visible_controls_in_frame_coordinates() {
        let mut refs = stable();
        // A window at (100, 100), 400×300 points, pictured at half size.
        let window = Rect::new(100.0, 100.0, 400.0, 300.0);
        let list = element_list(&sample(), &mut refs, window, (200, 150), MAX_LISTED);
        assert_eq!(list.text, "[e1] button \"Save\" at (20, 15)\n[e2] textField value=\"hello\" (focused) at (50, 56)\n");
        assert_eq!(list.shown, vec![1, 2]);
        let capped = element_list(&sample(), &mut stable(), window, (200, 150), 1);
        assert!(capped.truncated && capped.text.ends_with("… 1 more (computer_snapshot lists them all)\n"));
    }

    #[test]
    fn refs_parse_in_either_spelling() {
        assert_eq!(parse_ref("e12"), Some(12));
        assert_eq!(parse_ref(" [e3] "), Some(3));
        assert_eq!(parse_ref("E7"), Some(7));
        assert_eq!(parse_ref("ref_12"), None);
        assert_eq!(parse_ref("e-1"), None);
    }
}
