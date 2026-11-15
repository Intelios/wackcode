//! What the model reads back from a `todo` call, and the cleaning of task text.
//!
//! Ported from `@juicesharp/rpiv-todo` v2.11.0 (MIT) by way of version 2; the strings are
//! upstream's. Task text is the model's own, so it is cleaned before it is shown again: escape
//! sequences are dropped whole, line breaks become spaces so a field cannot change the layout,
//! and bidi controls are removed so a field cannot reorder what is read beside it.

use super::{Status, Task, TodoList, reducer::{Op, blocks}};

/// Removes terminal control sequences and control characters from model-written text.
pub fn clean(text: &str) -> String {
    const ESC: char = '\u{1b}';
    let characters: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        let next = characters.get(index + 1).copied();
        let (csi, osc) = match (character, next) {
            (ESC, Some('[')) => (Some(index + 2), None),
            (ESC, Some(']')) => (None, Some(index + 2)),
            ('\u{9b}', _) => (Some(index + 1), None),
            ('\u{9d}', _) => (None, Some(index + 1)),
            _ => (None, None),
        };
        if let Some(mut end) = csi {
            while characters.get(end).is_some_and(|c| ('0'..='?').contains(c)) {
                end += 1;
            }
            while characters.get(end).is_some_and(|c| (' '..='/').contains(c)) {
                end += 1;
            }
            if characters.get(end).is_some_and(|c| ('@'..='~').contains(c)) {
                index = end + 1;
                continue;
            }
        }
        if let Some(mut end) = osc {
            // An unterminated sequence swallows the rest, as a terminal would treat it.
            while characters.get(end).is_some_and(|c| !matches!(*c, '\u{07}' | '\u{9c}' | ESC)) {
                end += 1;
            }
            match (characters.get(end), characters.get(end + 1)) {
                (Some('\u{07}' | '\u{9c}'), _) => end += 1,
                (Some(&ESC), Some('\\')) => end += 2,
                _ => {}
            }
            index = end;
            continue;
        }
        if character == ESC && next.is_some() {
            index += 2;
            continue;
        }
        match character {
            '\n' | '\r' | '\t' | '\u{2028}' | '\u{2029}' => out.push(' '),
            '\u{00}'..='\u{1f}' | '\u{7f}'..='\u{9f}' => {}
            '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {}
            other => out.push(other),
        }
        index += 1;
    }
    out
}

fn ids(list: &[u64], separator: &str) -> String {
    list.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(separator)
}

/// One task as `[status] #id subject (activeForm) ⛓ #dep`.
pub fn list_line(task: &Task) -> String {
    let block = if task.blocked_by.is_empty() { String::new() } else { format!(" ⛓ {}", ids(&task.blocked_by, ",")) };
    let form = match &task.active_form {
        Some(form) if task.status == Status::InProgress => format!(" ({})", clean(form)),
        _ => String::new(),
    };
    format!("[{}] #{} {}{form}{block}", task.status.as_str(), task.id, clean(&task.subject))
}

fn get_lines(task: &Task, list: &TodoList) -> String {
    let mut lines = vec![format!("#{} [{}] {}", task.id, task.status.as_str(), clean(&task.subject))];
    if let Some(description) = task.description.as_ref().filter(|text| !text.is_empty()) {
        lines.push(format!("  description: {}", clean(description)));
    }
    if let Some(form) = task.active_form.as_ref().filter(|text| !text.is_empty()) {
        lines.push(format!("  activeForm: {}", clean(form)));
    }
    if !task.blocked_by.is_empty() {
        lines.push(format!("  blockedBy: {}", ids(&task.blocked_by, ", ")));
    }
    if let Some(blocked) = blocks(&list.tasks).get(&task.id) {
        lines.push(format!("  blocks: {}", ids(blocked, ", ")));
    }
    lines.join("\n")
}

/// The result of a call, as the model reads it.
pub fn content(op: &Op, list: &TodoList) -> String {
    match op {
        Op::Create { id } => match list.task(*id) {
            Some(task) => format!("Created #{}: {} (pending)", task.id, clean(&task.subject)),
            None => format!("Created #{id}"),
        },
        Op::Update { id, to, changed: false, .. } => {
            format!("No change: #{id} already matches the requested values (status: {})", to.as_str())
        }
        Op::Update { id, from, to, .. } if from != to => format!("Updated #{id} ({} → {})", from.as_str(), to.as_str()),
        Op::Update { id, .. } => format!("Updated #{id}"),
        Op::Delete { id, subject } => format!("Deleted #{id}: {}", clean(subject)),
        Op::Clear { count } => format!("Cleared {count} tasks"),
        Op::List { status, include_deleted } => {
            let lines: Vec<String> = list
                .tasks
                .iter()
                .filter(|task| *include_deleted || task.status != Status::Deleted)
                .filter(|task| status.is_none_or(|status| task.status == status))
                .map(list_line)
                .collect();
            if lines.is_empty() { "No tasks".to_string() } else { lines.join("\n") }
        }
        Op::Get { task } => get_lines(task, list),
        Op::Error(message) => format!("Error: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::todo::{Input, reducer::{Action, apply}};

    #[test]
    fn results_read_as_upstream_wrote_them() {
        let mut list = TodoList::default();
        let op = apply(&mut list, Action::Create, &Input { subject: Some("Write the parser".into()), ..Input::default() });
        assert_eq!(content(&op, &list), "Created #1: Write the parser (pending)");

        let start = Input { id: Some(1), status: Some(Status::InProgress), active_form: Some("writing the parser".into()), ..Input::default() };
        let op = apply(&mut list, Action::Update, &start);
        assert_eq!(content(&op, &list), "Updated #1 (pending → in_progress)");
        let op = apply(&mut list, Action::Update, &Input { id: Some(1), status: Some(Status::InProgress), ..Input::default() });
        assert_eq!(content(&op, &list), "No change: #1 already matches the requested values (status: in_progress)");

        apply(&mut list, Action::Create, &Input { subject: Some("Test it".into()), blocked_by: Some(vec![1]), ..Input::default() });
        let op = apply(&mut list, Action::List, &Input::default());
        assert_eq!(content(&op, &list), "[in_progress] #1 Write the parser (writing the parser)\n[pending] #2 Test it ⛓ #1");
        let op = apply(&mut list, Action::Get, &Input { id: Some(1), ..Input::default() });
        assert_eq!(content(&op, &list), "#1 [in_progress] Write the parser\n  activeForm: writing the parser\n  blocks: #2");

        let op = apply(&mut list, Action::Get, &Input { id: Some(9), ..Input::default() });
        assert_eq!(content(&op, &list), "Error: #9 not found");
        let op = apply(&mut list, Action::List, &Input { status: Some(Status::Completed), ..Input::default() });
        assert_eq!(content(&op, &list), "No tasks");
        let op = apply(&mut list, Action::Clear, &Input::default());
        assert_eq!(content(&op, &list), "Cleared 2 tasks");
    }

    #[test]
    fn task_text_cannot_carry_control_sequences() {
        assert_eq!(clean("red \u{1b}[31malert\u{1b}[0m"), "red alert");
        assert_eq!(clean("title\u{1b}]0;evil\u{07} after"), "title after");
        assert_eq!(clean("one\ntwo\tthree\u{2028}four"), "one two three four");
        assert_eq!(clean("a\u{202e}b\u{200f}c\u{7f}d"), "abcd");
        assert_eq!(clean("plain — text ✓"), "plain — text ✓");
    }
}
