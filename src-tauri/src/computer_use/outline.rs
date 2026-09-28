//! Renders an accessibility tree as the indented outline `computer_snapshot` returns, assigning
//! the element refs `computer_act` takes. Pure: the tree comes from `ax.rs` (or a test), so
//! folding, capping and ref assignment are unit-tested without Accessibility.
//!
//! One line per element: `[e3-12] button "Save" (disabled)`. Unlabelled layout containers are
//! folded away (their children move up), a static text repeating its parent's label is dropped,
//! and secure text fields never carry a value — `ax.rs` never reads one.

pub const MAX_OUTLINE_BYTES: usize = 48 * 1024;
const MAX_LABEL: usize = 120;
const MAX_VALUE: usize = 200;

/// Containers that only arrange other elements. Without a label they are folded away.
const CONTAINERS: &[&str] = &[
    "AXGroup",
    "AXSplitGroup",
    "AXLayoutArea",
    "AXLayoutItem",
    "AXScrollArea",
    "AXUnknown",
    "AXGrowArea",
    "AXMatte",
    "AXRuler",
    "AXRulerMarker",
    "AXSplitter",
];

/// Subroles more telling than their role.
const SUBROLES: &[&str] = &[
    "AXSecureTextField",
    "AXSearchField",
    "AXCloseButton",
    "AXMinimizeButton",
    "AXZoomButton",
    "AXFullScreenButton",
    "AXToggle",
    "AXSwitch",
    "AXTabButton",
    "AXSortButton",
];

#[derive(Debug, Clone, Default)]
pub struct Node {
    /// Index of this element in the walker's element list; what a ref resolves to.
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
    pub children: Vec<Node>,
    /// Children the walker left out (its per-element cap), shown as "… N more".
    pub omitted_children: usize,
}

impl Node {
    pub fn is_secure(&self) -> bool {
        self.subrole.as_deref() == Some("AXSecureTextField")
    }
}

#[derive(Debug, Clone, Default)]
pub struct Outline {
    pub text: String,
    /// `refs[i]` is the handle of ref `{prefix}-{i}`.
    pub refs: Vec<usize>,
    pub truncated: bool,
}

/// `"AXStaticText"` → `"staticText"`.
fn display_role(node: &Node) -> String {
    let raw = node.subrole.as_deref().filter(|subrole| SUBROLES.contains(subrole)).unwrap_or(&node.role);
    let name = raw.strip_prefix("AX").unwrap_or(raw);
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => "element".into(),
    }
}

fn clip(text: &str, max: usize) -> String {
    let single_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= max {
        single_line
    } else {
        format!("{}…", single_line.chars().take(max).collect::<String>())
    }
}

fn nonempty(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|text| !text.is_empty())
}

fn label(node: &Node) -> Option<&str> {
    nonempty(&node.title).or_else(|| nonempty(&node.description))
}

fn is_folded(node: &Node) -> bool {
    CONTAINERS.contains(&node.role.as_str())
        && label(node).is_none()
        && nonempty(&node.value).is_none()
        && !node.focused
        && !node.selected
}

fn is_checkable(node: &Node) -> bool {
    matches!(node.role.as_str(), "AXCheckBox" | "AXRadioButton") || matches!(node.subrole.as_deref(), Some("AXToggle" | "AXSwitch"))
}

fn line(node: &Node, reference: &str) -> String {
    let mut text = format!("[{reference}] {}", display_role(node));
    let is_static = node.role == "AXStaticText";
    match (is_static, label(node), nonempty(&node.value)) {
        // A static text's value is its text.
        (true, None, Some(value)) => text.push_str(&format!(" \"{}\"", clip(value, MAX_VALUE))),
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
    if label(node).is_none() {
        if let Some(placeholder) = nonempty(&node.placeholder) {
            text.push_str(&format!(" placeholder=\"{}\"", clip(placeholder, MAX_LABEL)));
        }
    }
    let flags = [
        (node.enabled == Some(false), "disabled"),
        (node.focused, "focused"),
        (node.selected, "selected"),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .map(|(_, name)| *name)
    .collect::<Vec<_>>();
    if !flags.is_empty() {
        text.push_str(&format!(" ({})", flags.join(", ")));
    }
    text
}

struct Renderer<'a> {
    prefix: &'a str,
    max_bytes: usize,
    outline: Outline,
}

impl Renderer<'_> {
    fn push(&mut self, depth: usize, text: &str) -> bool {
        let needed = depth * 2 + text.len() + 1;
        if self.outline.text.len() + needed > self.max_bytes {
            self.outline.truncated = true;
            return false;
        }
        for _ in 0..depth {
            self.outline.text.push_str("  ");
        }
        self.outline.text.push_str(text);
        self.outline.text.push('\n');
        true
    }

    /// Renders `node` at `depth`; false once the byte cap is reached.
    fn node(&mut self, node: &Node, depth: usize, parent_label: Option<&str>) -> bool {
        if self.outline.truncated {
            return false;
        }
        let redundant_text = node.role == "AXStaticText"
            && node.children.is_empty()
            && parent_label.is_some()
            && nonempty(&node.value).or_else(|| label(node)) == parent_label;
        if redundant_text {
            return true;
        }
        let (child_depth, own_label) = if is_folded(node) {
            (depth, parent_label)
        } else {
            let reference = format!("{}-{}", self.prefix, self.outline.refs.len());
            if !self.push(depth, &line(node, &reference)) {
                return false;
            }
            self.outline.refs.push(node.handle);
            (depth + 1, label(node))
        };
        for child in &node.children {
            if !self.node(child, child_depth, own_label) {
                return false;
            }
        }
        if node.omitted_children > 0 {
            return self.push(child_depth, &format!("… {} more", node.omitted_children));
        }
        true
    }
}

/// Renders the tree under `root` with refs `{prefix}-{i}`, capped at `max_bytes`.
pub fn render(root: &Node, prefix: &str, max_bytes: usize) -> Outline {
    let mut renderer = Renderer { prefix, max_bytes, outline: Outline::default() };
    renderer.node(root, 0, None);
    renderer.outline
}

/// The ref's index when it belongs to `prefix` (e.g. `"e3-12"` with prefix `"e3"` → 12).
pub fn parse_ref(reference: &str, prefix: &str) -> Option<usize> {
    reference.trim().strip_prefix(prefix)?.strip_prefix('-')?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(handle: usize, role: &str) -> Node {
        Node { handle, role: role.into(), ..Node::default() }
    }

    fn sample() -> Node {
        let mut window = node(0, "AXWindow");
        window.title = Some("Untitled".into());
        let mut group = node(1, "AXGroup");
        let mut save = node(2, "AXButton");
        save.title = Some("Save".into());
        let mut save_text = node(3, "AXStaticText");
        save_text.value = Some("Save".into());
        save.children.push(save_text);
        let mut field = node(4, "AXTextField");
        field.value = Some("hello".into());
        field.focused = true;
        let mut secret = node(5, "AXTextField");
        secret.subrole = Some("AXSecureTextField".into());
        let mut check = node(6, "AXCheckBox");
        check.title = Some("Remember".into());
        check.value = Some("1".into());
        check.enabled = Some(false);
        group.children = vec![save, field, secret, check];
        group.omitted_children = 3;
        window.children.push(group);
        window
    }

    #[test]
    fn renders_folded_labelled_outline_with_refs() {
        let outline = render(&sample(), "e1", MAX_OUTLINE_BYTES);
        assert_eq!(
            outline.text,
            "[e1-0] window \"Untitled\"\n  [e1-1] button \"Save\"\n  [e1-2] textField value=\"hello\" (focused)\n  [e1-3] secureTextField (secure)\n  [e1-4] checkBox \"Remember\" (checked) (disabled)\n  … 3 more\n"
        );
        assert_eq!(outline.refs, vec![0, 2, 4, 5, 6]);
        assert!(!outline.truncated);
    }

    #[test]
    fn secure_fields_never_show_a_value_even_if_one_is_present() {
        let mut secret = node(0, "AXTextField");
        secret.subrole = Some("AXSecureTextField".into());
        secret.value = Some("hunter2".into());
        let outline = render(&secret, "e1", MAX_OUTLINE_BYTES);
        assert!(!outline.text.contains("hunter2"));
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
        let outline = render(&root, "e2", 1024);
        assert!(outline.truncated);
        assert!(outline.text.len() <= 1024);
        assert_eq!(outline.refs.len(), outline.text.lines().count());
    }

    #[test]
    fn long_labels_are_clipped_to_one_line() {
        let mut text = node(0, "AXStaticText");
        text.value = Some(format!("first\nsecond {}", "x".repeat(400)));
        let outline = render(&text, "e1", MAX_OUTLINE_BYTES);
        assert_eq!(outline.text.lines().count(), 1);
        assert!(outline.text.contains("first second"));
        assert!(outline.text.trim_end().ends_with("…\""));
    }

    #[test]
    fn refs_parse_only_for_their_own_snapshot() {
        assert_eq!(parse_ref("e3-12", "e3"), Some(12));
        assert_eq!(parse_ref(" e3-0 ", "e3"), Some(0));
        assert_eq!(parse_ref("e2-12", "e3"), None);
        assert_eq!(parse_ref("e31-2", "e3"), None);
        assert_eq!(parse_ref("e3-x", "e3"), None);
    }
}
