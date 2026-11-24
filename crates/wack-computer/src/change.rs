//! What an action changed, from the app's fingerprint before and after it. Pure, so it is
//! unit-tested.
//!
//! Invariants:
//! - An action's answer says what changed (windows, the title, focus, the value, the app coming
//!   forward) or that nothing visible did, so the model can usually go on without looking again:
//!   version 2 told it to look after every action, which doubled the round trips.
//! - Values are clipped to one line; the tool wraps the report as app data.

use crate::{backend::Fingerprint, outline::clip};

const MAX_VALUE: usize = 200;

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
    fn an_emptied_value_is_said() {
        let before = print(&[], None, Some("draft"), None);
        let after = print(&[], None, Some(""), None);
        assert_eq!(report(&before, &after, 5, "Notes"), vec!["its value is now empty"]);
    }
}
