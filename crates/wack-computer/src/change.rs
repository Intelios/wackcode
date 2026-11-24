//! What an action changed, from the app's fingerprint before and after it. Pure, so it is
//! unit-tested.
//!
//! Invariants:
//! - An action's answer says what changed (windows, the title, focus, the value, the app coming
//!   forward) or that nothing visible did, so the model can usually go on without looking again:
//!   version 2 told it to look after every action, which doubled the round trips.
//! - Values are clipped to one line; the tool wraps the report as app data.

use std::collections::HashMap;

use crate::{
    backend::Fingerprint,
    outline::{self, Node, clip},
};

const MAX_VALUE: usize = 200;
/// Roles that, appearing, are worth saying: something opened over the window.
const OPENED: [&str; 4] = ["AXSheet", "AXPopover", "AXMenu", "AXDialog"];

/// What changed inside a window, from its tree before and after an action: elements whose value
/// changed (the display of a calculator, a status line) and sheets, popovers and menus that
/// opened. `skip` is the acted-on element, whose value the fingerprint already reports. At most
/// `max` lines, then a count; the refs mentioned are returned to be marked as shown.
pub fn tree_report(before: &Node, after: &Node, skip: Option<u32>, max: usize) -> (Vec<String>, Vec<u32>) {
    let mut old: HashMap<usize, Option<String>> = HashMap::new();
    before.each(&mut |node| {
        old.insert(node.handle, node.value.clone());
    });
    let mut lines = Vec::new();
    let mut refs = Vec::new();
    let mut extra = 0;
    after.each(&mut |node| {
        if skip == Some(node.handle as u32) || node.is_secure() {
            return;
        }
        let line = match old.get(&node.handle) {
            Some(value) if *value != node.value && node.value.as_deref().is_some_and(|value| !value.trim().is_empty()) => {
                let was = value.as_deref().filter(|value| !value.trim().is_empty()).map(|value| format!(" (was \"{}\")", clip(value, 60))).unwrap_or_default();
                format!("[e{}] {}{was}", node.handle, outline::describe(node))
            }
            None if OPENED.contains(&node.role.as_str()) || node.subrole.as_deref() == Some("AXDialog") => format!("opened: [e{}] {}", node.handle, outline::describe(node)),
            _ => return,
        };
        if lines.len() < max {
            lines.push(line);
            refs.push(node.handle as u32);
        } else {
            extra += 1;
        }
    });
    if extra > 0 {
        lines.push(format!("… {extra} more changes (computer_snapshot shows them)"));
    }
    (lines, refs)
}

pub fn report(before: &Fingerprint, after: &Fingerprint, pid: i32, app: &str) -> Vec<String> {
    let mut changes = Vec::new();
    for (id, title) in &after.windows {
        match before.windows.iter().find(|(old, _)| old == id) {
            None => changes.push(format!("a window opened: \"{}\" (window {id})", clip(title, 80))),
            Some((_, old)) if old != title => changes.push(format!("window {id}'s title is now \"{}\"", clip(title, 80))),
            _ => {}
        }
    }
    for (id, title) in &before.windows {
        if !after.windows.iter().any(|(new, _)| new == id) {
            changes.push(format!("a window closed: \"{}\" (window {id})", clip(title, 80)));
        }
    }
    if after.value != before.value {
        match &after.value {
            Some(value) if !value.is_empty() => changes.push(format!("its value is now \"{}\"", clip(value, MAX_VALUE))),
            _ => changes.push("its value is now empty".into()),
        }
    }
    if after.focused != before.focused
        && let Some(focused) = &after.focused
    {
        changes.push(format!("focus moved to {focused}"));
    }
    if after.frontmost == Some(pid) && before.frontmost != Some(pid) {
        changes.push(format!("{app} came to the front"));
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn print(windows: &[(u32, &str)], focused: Option<&str>, value: Option<&str>, front: Option<i32>) -> Fingerprint {
        Fingerprint {
            focused: focused.map(str::to_string),
            windows: windows.iter().map(|(id, title)| (*id, title.to_string())).collect(),
            frontmost: front,
            value: value.map(str::to_string),
            target: None,
        }
    }

    #[test]
    fn nothing_changed_says_nothing() {
        let same = print(&[(1, "Untitled")], Some("textArea"), Some("hi"), Some(9));
        assert!(report(&same, &same, 5, "TextEdit").is_empty());
    }

    #[test]
    fn windows_focus_values_and_the_front_are_reported() {
        let before = print(&[(1, "Untitled"), (2, "Old")], Some("textArea"), Some("hi"), Some(9));
        let after = print(&[(1, "Untitled — Edited"), (3, "Save As")], Some("textField \"Save As:\""), Some("hi there"), Some(5));
        assert_eq!(
            report(&before, &after, 5, "TextEdit"),
            vec![
                "window 1's title is now \"Untitled — Edited\"",
                "a window opened: \"Save As\" (window 3)",
                "a window closed: \"Old\" (window 2)",
                "its value is now \"hi there\"",
                "focus moved to textField \"Save As:\"",
                "TextEdit came to the front",
            ]
        );
    }

    #[test]
    fn a_tree_report_names_changed_values_and_what_opened() {
        let node = |handle: usize, role: &str, value: Option<&str>| Node { handle, role: role.into(), description: Some(format!("n{handle}")), value: value.map(str::to_string), ..Node::default() };
        let before = Node { handle: 1, role: "AXWindow".into(), children: vec![node(2, "AXStaticText", Some("7")), node(3, "AXButton", None), node(4, "AXTextField", Some("draft"))], ..Node::default() };
        let after = Node {
            handle: 1,
            role: "AXWindow".into(),
            children: vec![node(2, "AXStaticText", Some("15")), node(3, "AXButton", None), node(4, "AXTextField", Some("draft!")), node(9, "AXSheet", None)],
            ..Node::default()
        };
        let (lines, refs) = tree_report(&before, &after, Some(4), 5);
        assert_eq!(lines, vec!["[e2] staticText \"n2\" value=\"15\" (was \"7\")", "opened: [e9] sheet \"n9\""]);
        assert_eq!(refs, vec![2, 9]);
        let (lines, _) = tree_report(&before, &after, None, 1);
        assert_eq!(lines.len(), 2, "one line, then the count");
        assert!(lines[1].starts_with("… 2 more"));
    }

    #[test]
    fn an_emptied_value_is_said() {
        let before = print(&[], None, Some("draft"), None);
        let after = print(&[], None, Some(""), None);
        assert_eq!(report(&before, &after, 5, "Notes"), vec!["its value is now empty"]);
    }
}
