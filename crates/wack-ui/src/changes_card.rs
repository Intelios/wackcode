//! Per-turn file changes card: what a finished turn edited or created, under its answer.
//!
//! Ported from version 2's `turn-changes.ts` and `TurnChangesCard.tsx`:
//! - Collects completed, non-error `write` and `edit` calls in the turn, deduplicating repeated
//!   touches to the same normalized file path in first-touched order.
//! - Distinguishes created files (`write` whose result starts with `"Created "`) from edited files
//!   (`edit`, or `write` replacing an existing file). Heading reads `"Created N file(s)"`,
//!   `"Edited N file(s)"`, or `"Changed N files"`, with net `+added −removed` totals and a button
//!   to open the Changes pane (`ShowChanges`).
//! - Surfaces up to [`MAX_SHOWN_FILES`] (3) files with a `"Show N more"` / `"Show less"` toggle.
//!   Colliding basenames show their shortest distinguishing trailing folder suffix; non-colliding
//!   deep paths shorten to `…/parent/`.
//! - Clicking a file row expands its inline diff right inside the card, with a copy button and a
//!   link to jump to that file in the Changes pane. Expansion state lives in the chat's `open` set
//!   via `Handlers::toggle`, never in this view.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use gpui::{ElementId, FontWeight, SharedString, div, prelude::*, px};
use serde_json::Value;

use crate::{
    actions::ShowChanges,
    conversation::{Step, Turn},
    copy::CopyButton,
    icon::Icon,
    motion,
    tags::Tag,
    theme::Theme,
    tooltip::tip,
    transcript::{DiffLine, Handlers, LineKind, diff_line},
    typography::{MONO_FONT, Type, Typography},
    work::{cap, edit_lines, plural, shown_path, write_lines},
};

/// The most files shown before folding the rest behind "Show N more".
pub const MAX_SHOWN_FILES: usize = 3;
/// Tallest expanded inline diff inside the card before scrolling.
const DIFF_MAX_HEIGHT: f32 = 340.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Create,
    Edit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChangesCard {
    /// Stable toggle prefix for this turn's card, e.g. `"turn-1-changes"`.
    pub key: SharedString,
    /// `"Edited 1 file"`, `"Created 2 files"`, `"Changed 4 files"`.
    pub heading: SharedString,
    pub files: Vec<ChangedFile>,
    pub added: usize,
    pub removed: usize,
    /// True when the list is expanded past [`MAX_SHOWN_FILES`].
    pub expanded: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChangedFile {
    /// Toggle key for this file's inline diff, e.g. `"turn-1-changes-file-0"`.
    pub key: SharedString,
    /// Normalized path relative to the project (or absolute if outside).
    pub path: SharedString,
    /// Directory prefix shown muted before `name`, ending in `/` when non-empty.
    pub dir: SharedString,
    /// File basename.
    pub name: SharedString,
    pub kind: FileKind,
    pub added: usize,
    pub removed: usize,
    /// How many `write`/`edit` calls touched this file during the turn.
    pub calls: usize,
    /// True when every call on this file was a `write` (Copy copies plain file content).
    pub plain_write: bool,
    /// Capped diff lines shown when the row is expanded.
    pub lines: Vec<DiffLine>,
    /// Uncapped diff (or written file content) put on the clipboard by Copy.
    pub copy_text: SharedString,
    /// Whether this file's inline diff is currently expanded.
    pub open: bool,
}

impl ChangesCard {
    /// The `open` key that expands/collapses the file list beyond [`MAX_SHOWN_FILES`].
    pub fn more_key(&self) -> String {
        format!("{}-more", self.key)
    }

    /// Whether this card owns `key`, so [`crate::chat::ChatSession`] knows which entry to rebuild.
    pub fn holds(&self, key: &str) -> bool {
        key == self.more_key() || self.files.iter().any(|file| file.key.as_ref() == key)
    }
}

enum CopyChunk {
    Write(String),
    Diff(Vec<DiffLine>),
}

struct FileAcc {
    path: String,
    kind: FileKind,
    created_in_turn: bool,
    added: usize,
    removed: usize,
    calls: usize,
    only_writes: bool,
    shown_lines: Vec<DiffLine>,
    copy_chunks: Vec<CopyChunk>,
}

/// Normalizes a tool's `path` argument relative to `cwd` so repeated touches to the same file
/// (relative, `./`-prefixed, or absolute inside `cwd`) fold into one row.
fn normalize_path(raw: &str, cwd: &Path) -> String {
    let relative = shown_path(raw, cwd);
    let mut trimmed = relative.as_str();
    while let Some(rest) = trimmed.strip_prefix("./") {
        trimmed = rest;
    }
    if trimmed.len() > 1 {
        trimmed = trimmed.trim_end_matches('/');
    }
    if trimmed.is_empty() { relative } else { trimmed.to_string() }
}

/// Shortens a deep path for display, matching version 2's `displayPath`.
fn display_path(path: &str) -> String {
    let normalized = path.trim_start_matches("./").trim_end_matches('/');
    let parts: Vec<&str> = normalized.split('/').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return path.to_string();
    }
    if normalized.starts_with('/') && parts.len() > 2 {
        return format!("…/{}", parts[parts.len() - 2..].join("/"));
    }
    if parts.len() > 3 {
        return format!("…/{}", parts[parts.len() - 2..].join("/"));
    }
    normalized.to_string()
}

/// Splits `"src/app.rs"` into `("src/", "app.rs")` and `"Cargo.toml"` into `("", "Cargo.toml")`.
fn split_path(path: &str) -> (String, String) {
    match path.rsplit_once('/') {
        Some((dir, name)) if !name.is_empty() => (format!("{dir}/"), name.to_string()),
        _ => (String::new(), path.to_string()),
    }
}

/// Shortest distinguishing trailing folder suffixes for files that share a basename.
/// Example: `packages/web/src/Button.tsx` + `packages/cli/src/Button.tsx` -> `"web/src/"` vs `"cli/src/"`.
fn disambiguate_folders(entries: &[(String, String)]) -> HashMap<String, String> {
    let mut by_name: HashMap<&str, Vec<&str>> = HashMap::new();
    for (path, name) in entries {
        by_name.entry(name.as_str()).or_default().push(path.as_str());
    }
    let mut hints = HashMap::new();
    for group in by_name.values() {
        if group.len() < 2 {
            continue;
        }
        let segments: Vec<Vec<&str>> = group
            .iter()
            .map(|path| {
                let normalized = path.trim_start_matches("./").trim_end_matches('/');
                let parts: Vec<&str> = normalized.split('/').filter(|part| !part.is_empty()).collect();
                if parts.len() > 1 { parts[..parts.len() - 1].to_vec() } else { Vec::new() }
            })
            .collect();
        let max_depth = segments.iter().map(Vec::len).max().unwrap_or(0);
        for depth in 1..=max_depth {
            let candidates: Vec<String> = segments
                .iter()
                .map(|parts| {
                    let start = parts.len().saturating_sub(depth);
                    parts[start..].join("/")
                })
                .collect();
            let unique: HashSet<&str> = candidates.iter().map(String::as_str).collect();
            if unique.len() == group.len() {
                for (path, candidate) in group.iter().zip(candidates) {
                    if !candidate.is_empty() {
                        hints.insert((*path).to_string(), format!("{candidate}/"));
                    }
                }
                break;
            }
        }
        for (path, parts) in group.iter().zip(&segments) {
            if !hints.contains_key(*path) && !parts.is_empty() {
                hints.insert((*path).to_string(), format!("{}/", parts.join("/")));
            }
        }
    }
    hints
}

fn format_copy_text(only_writes: bool, chunks: &[CopyChunk]) -> String {
    if only_writes {
        return chunks
            .iter()
            .rev()
            .find_map(|chunk| match chunk {
                CopyChunk::Write(content) => Some(content.clone()),
                CopyChunk::Diff(_) => None,
            })
            .unwrap_or_default();
    }
    let mut out = Vec::new();
    for (index, chunk) in chunks.iter().enumerate() {
        if index > 0 {
            out.push("...".to_string());
        }
        match chunk {
            CopyChunk::Write(content) => {
                for line in content.lines() {
                    out.push(format!("+{line}"));
                }
            }
            CopyChunk::Diff(lines) => {
                for line in lines {
                    match line.kind {
                        LineKind::Added => out.push(format!("+{}", line.text)),
                        LineKind::Removed => out.push(format!("-{}", line.text)),
                        LineKind::Context => out.push(format!(" {}", line.text)),
                        LineKind::Gap => out.push("...".to_string()),
                    }
                }
            }
        }
    }
    out.join("\n")
}

/// Summarises the completed `write` and `edit` calls in `turn` into a Changes card, or `None`
/// if the turn did not successfully modify any files.
pub fn from_turn(turn: &Turn, turn_key: &str, cwd: &Path, open: &HashSet<String>) -> Option<ChangesCard> {
    let mut order: Vec<String> = Vec::new();
    let mut by_path: HashMap<String, FileAcc> = HashMap::new();

    for step in &turn.steps {
        let Step::Tool(tool) = step else { continue };
        if tool.name != "edit" && tool.name != "write" {
            continue;
        }
        let Some(result) = &tool.result else { continue };
        if result.is_error {
            continue;
        }
        let Some(raw_path) = tool
            .input
            .as_ref()
            .and_then(|input| input.get("path"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            continue;
        };
        let path = normalize_path(raw_path, cwd);
        let is_new = tool.name == "write" && result.text.starts_with("Created ");
        if !by_path.contains_key(&path) {
            order.push(path.clone());
            by_path.insert(
                path.clone(),
                FileAcc {
                    path: path.clone(),
                    kind: if is_new { FileKind::Create } else { FileKind::Edit },
                    created_in_turn: is_new,
                    added: 0,
                    removed: 0,
                    calls: 0,
                    only_writes: tool.name == "write",
                    shown_lines: Vec::new(),
                    copy_chunks: Vec::new(),
                },
            );
        }
        let acc = by_path.get_mut(&path).expect("inserted above");
        if tool.name == "write" {
            let content = tool.input.as_ref().and_then(|input| input.get("content")).and_then(Value::as_str).unwrap_or("");
            let lines = write_lines(content);
            match crate::work::replaced_counts(&result.text) {
                // Replacing a file it did not create: what changed, as Git will count it.
                Some((added, removed)) if !acc.created_in_turn => {
                    acc.added += added;
                    acc.removed += removed;
                }
                _ => {
                    acc.added = lines.len();
                    acc.removed = 0;
                }
            }
            if acc.calls > 0 {
                acc.shown_lines.push(DiffLine { kind: LineKind::Gap, number: None, text: "Replaced file".into() });
            }
            acc.shown_lines.extend(cap(lines));
            acc.copy_chunks.push(CopyChunk::Write(content.to_string()));
        } else {
            let lines = edit_lines(tool.input.as_ref());
            acc.added += lines.iter().filter(|line| line.kind == LineKind::Added).count();
            acc.removed += lines.iter().filter(|line| line.kind == LineKind::Removed).count();
            acc.only_writes = false;
            if acc.calls > 0 {
                let label = if acc.calls == 1 && matches!(acc.copy_chunks.first(), Some(CopyChunk::Write(_))) {
                    "Edited".to_string()
                } else {
                    format!("Edit {}", acc.calls + 1)
                };
                acc.shown_lines.push(DiffLine { kind: LineKind::Gap, number: None, text: label.into() });
            }
            acc.shown_lines.extend(cap(lines.clone()));
            acc.copy_chunks.push(CopyChunk::Diff(lines));
        }
        acc.calls += 1;
    }

    if order.is_empty() {
        return None;
    }

    let raw_splits: Vec<(String, String, String)> = order
        .iter()
        .map(|path| {
            let shown = display_path(path);
            let (dir, name) = split_path(&shown);
            (path.clone(), dir, name)
        })
        .collect();
    let pairs: Vec<(String, String)> = raw_splits.iter().map(|(path, _, name)| (path.clone(), name.clone())).collect();
    let hints = disambiguate_folders(&pairs);

    let card_key = format!("{turn_key}-changes");
    let more_key = format!("{card_key}-more");
    let mut files = Vec::with_capacity(order.len());
    let mut total_added = 0usize;
    let mut total_removed = 0usize;

    for (index, (path, default_dir, name)) in raw_splits.into_iter().enumerate() {
        let acc = by_path.remove(&path).expect("present");
        let dir = hints.get(&path).cloned().unwrap_or(default_dir);
        let file_key = format!("{card_key}-file-{index}");
        let copy_text = format_copy_text(acc.only_writes, &acc.copy_chunks);
        total_added += acc.added;
        total_removed += acc.removed;
        files.push(ChangedFile {
            open: open.contains(&file_key),
            key: file_key.into(),
            path: acc.path.into(),
            dir: dir.into(),
            name: name.into(),
            kind: acc.kind,
            added: acc.added,
            removed: acc.removed,
            calls: acc.calls,
            plain_write: acc.only_writes,
            lines: acc.shown_lines,
            copy_text: copy_text.into(),
        });
    }

    let all_create = files.iter().all(|file| file.kind == FileKind::Create);
    let all_edit = files.iter().all(|file| file.kind == FileKind::Edit);
    let count = plural(files.len(), "file", "files");
    let heading = if all_create {
        format!("Created {count}")
    } else if all_edit {
        format!("Edited {count}")
    } else {
        format!("Changed {count}")
    };

    Some(ChangesCard {
        expanded: open.contains(&more_key),
        key: card_key.into(),
        heading: heading.into(),
        files,
        added: total_added,
        removed: total_removed,
    })
}

// -------------------------------------------------------------------------------------------------
// View

fn stats_view(added: usize, removed: usize, theme: &Theme) -> Option<gpui::Div> {
    if added == 0 && removed == 0 {
        return None;
    }
    Some(
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(5.))
            .typography(Type::Small)
            .when(added > 0, |row| row.child(div().text_color(theme.diff_added).child(format!("+{added}"))))
            .when(removed > 0, |row| row.child(div().text_color(theme.diff_removed).child(format!("−{removed}")))),
    )
}

/// The card: on the cards' own ground, so it reads over any picture, a hairline round it. Its line
/// says how much, its one action in the accent; each file is its name, its folder quieter, its
/// lines in the diff colours.
pub fn view(card: &ChangesCard, theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    let head = div()
        .min_h(px(38.))
        .pl(px(14.))
        .pr(px(8.))
        .py(px(6.))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().flex_none().min_w_0().truncate().typography(Type::UiStrong).text_color(theme.text).child(card.heading.clone()))
        .children(stats_view(card.added, card.removed, theme).map(|stats| stats.tag("turn-changes-totals")))
        .child(div().flex_1())
        .child(
            div()
                .id(ElementId::Name(format!("{}-open", card.key).into()))
                .flex_none()
                .h(px(24.))
                .px(px(10.))
                .rounded_full()
                .hover(|button| button.bg(theme.accent_wash))
                .cursor_pointer()
                .flex()
                .items_center()
                .gap(px(4.))
                .typography(Type::Small)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.accent_text)
                .tooltip(tip("Open Changes pane (⌘⇧G)"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowChanges { path: None }), cx))
                .child("Review")
                .child(Icon::ChevronRight.view(px(10.), theme.accent_text))
                .tag("turn-changes-open"),
        );

    let shown = if card.expanded { card.files.len() } else { card.files.len().min(MAX_SHOWN_FILES) };
    let hidden = card.files.len().saturating_sub(shown);

    let mut rows = div().flex().flex_col();
    for file in &card.files[..shown] {
        rows = rows.child(file_row(file, theme, handlers));
    }

    let more = (card.files.len() > MAX_SHOWN_FILES).then(|| {
        let toggle = handlers.toggle.clone();
        let more_key: SharedString = card.more_key().into();
        let label = if card.expanded { "Show less".to_string() } else { format!("Show {hidden} more") };
        div()
            .id(ElementId::Name(format!("{}-more-btn", card.key).into()))
            .h(px(30.))
            .px(px(14.))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(5.))
            .cursor_pointer()
            .typography(Type::Small)
            .text_color(theme.text_dim)
            .hover(|row| row.bg(theme.hover).text_color(theme.text_soft))
            .on_click(move |_, window, cx| toggle(more_key.clone(), window, cx))
            .child(label)
            .child(if card.expanded { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(10.), theme.text_faint))
            .tag("turn-changes-more")
    });

    motion::enter(
        card.key.clone(),
        div()
            .rounded(px(12.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(head)
            .child(rows)
            .children(more),
    )
    .tag("turn-changes-card")
}

fn file_row(file: &ChangedFile, theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    let toggle = handlers.toggle.clone();
    let key = file.key.clone();
    let hover = theme.hover;
    let icon = match file.kind {
        FileKind::Create => Icon::FilePlus,
        FileKind::Edit => Icon::File,
    }
    .view(px(13.), theme.text_dim);

    let head = div()
        .id(file.key.clone())
        .min_h(px(30.))
        .px(px(14.))
        .py(px(4.))
        .flex()
        .items_center()
        .gap(px(8.))
        .cursor_pointer()
        .hover(move |row| row.bg(hover))
        .tooltip(tip(file.path.clone()))
        .on_click(move |_, window, cx| toggle(key.clone(), window, cx))
        .child(icon)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.))
                .overflow_hidden()
                .child(div().flex_none().typography(Type::Ui).text_color(theme.text_body).child(file.name.clone()))
                .when(!file.dir.is_empty(), |path| {
                    path.child(div().min_w_0().truncate().typography(Type::Ui).text_color(theme.text_dim).child(file.dir.clone().trim_end_matches('/').to_string()))
                }),
        )
        .children(stats_view(file.added, file.removed, theme))
        .child(crate::transcript::chevron(&file.key, file.open, theme.text_faint, handlers))
        .tag("turn-changes-file");

    // Drawn while open, and while it folds shut.
    let diff_body = (file.open || handlers.folding(&file.key).is_some()).then(|| {
        let open_path = file.path.to_string();
        let copy_label = if file.plain_write { "Copy" } else { "Copy diff" };
        let meta = if file.calls > 1 {
            format!("{}  ·  {}", file.path, plural(file.calls, "edit", "edits"))
        } else {
            file.path.to_string()
        };
        let sub_bar = div()
            .flex_none()
            .h(px(30.))
            .px(px(12.))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .typography(Type::Small)
                    .font_family(MONO_FONT)
                    .text_color(theme.text_dim)
                    .child(meta),
            )
            .when(!file.copy_text.is_empty(), |bar| {
                bar.child(CopyButton::new(format!("{}-copy", file.key), file.copy_text.clone(), copy_label, "copy-turn-diff"))
            })
            .child(
                div()
                    .id(ElementId::Name(format!("{}-in-changes", file.key).into()))
                    .flex_none()
                    .h(px(22.))
                    .px(px(8.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .typography(Type::Small)
                    .text_color(theme.accent_text)
                    .cursor_pointer()
                    .hover(|btn| btn.bg(theme.accent_wash))
                    .tooltip(tip("Inspect in Changes pane"))
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(ShowChanges { path: Some(open_path.clone()) }), cx)
                    })
                    .child("In Changes")
                    .tag("turn-changes-file-open"),
            );

        let content = if file.lines.is_empty() {
            div().px(px(14.)).py(px(10.)).typography(Type::Small).text_color(theme.text_faint).child("Empty file.").into_any_element()
        } else {
            div()
                .id(ElementId::Name(format!("{}-diff", file.key).into()))
                .max_h(px(DIFF_MAX_HEIGHT))
                .overflow_y_scroll()
                .py(px(6.))
                .flex()
                .flex_col()
                .children(file.lines.iter().map(|line| diff_line(line, theme)))
                .into_any_element()
        };

        div()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.code_ground)
            .flex()
            .flex_col()
            .child(sub_bar)
            .child(content)
            .tag("turn-changes-diff")
            .into_any_element()
    });
    let height = 31. + (file.lines.len() as f32 * 19. + 12.).min(DIFF_MAX_HEIGHT);
    let diff_body = diff_body.and_then(|body| crate::transcript::unfolding(body, &file.key, file.open, height, handlers));

    div().border_t_1().border_color(theme.border).flex().flex_col().child(head).children(diff_body)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::conversation::{ToolResult, ToolStep};

    fn turn_with(steps: Vec<Step>) -> Turn {
        let mut turn = Turn::default();
        turn.steps = steps;
        turn
    }

    fn tool_step(id: &str, name: &str, input: Value, result: Option<(&str, bool)>) -> Step {
        Step::Tool(ToolStep {
            id: id.into(),
            name: name.into(),
            input: Some(input),
            result: result.map(|(text, is_error)| ToolResult { text: text.into(), is_error, pictures: Vec::new() }),
            writing: None,
            started: None,
        })
    }

    #[test]
    fn accumulates_edits_distinguishes_kinds_skips_errors_and_disambiguates_folders() {
        let turn = turn_with(vec![
            tool_step("w1", "write", json!({ "path": "packages/web/src/Button.tsx", "content": "a\nb\nc\n" }), Some(("Created packages/web/src/Button.tsx (3 lines).", false))),
            tool_step("e1", "edit", json!({ "path": "packages/cli/src/Button.tsx", "edits": [{ "old_text": "x", "new_text": "y\nz" }] }), Some(("Edited packages/cli/src/Button.tsx (1 change).", false))),
            tool_step("e2", "edit", json!({ "path": "/repo/packages/cli/src/Button.tsx", "edits": [{ "old_text": "z", "new_text": "" }] }), Some(("Edited packages/cli/src/Button.tsx (1 change).", false))),
            tool_step("bad", "edit", json!({ "path": "src/broken.ts", "edits": [{ "old_text": "a", "new_text": "b" }] }), Some(("old_text was not found", true))),
            tool_step("pending", "write", json!({ "path": "src/pending.ts", "content": "hi" }), None),
        ]);

        let card = from_turn(&turn, "turn-1", Path::new("/repo"), &HashSet::new()).expect("card");
        assert_eq!(card.heading.as_ref(), "Changed 2 files");
        assert_eq!((card.added, card.removed), (5, 2));
        assert_eq!(card.files.len(), 2);

        let web = &card.files[0];
        assert_eq!(web.path.as_ref(), "packages/web/src/Button.tsx");
        assert_eq!(web.dir.as_ref(), "web/src/");
        assert_eq!(web.name.as_ref(), "Button.tsx");
        assert_eq!(web.kind, FileKind::Create);
        assert_eq!((web.added, web.removed, web.calls), (3, 0, 1));
        assert!(web.plain_write);
        assert_eq!(web.copy_text.as_ref(), "a\nb\nc\n");

        let cli = &card.files[1];
        assert_eq!(cli.path.as_ref(), "packages/cli/src/Button.tsx");
        assert_eq!(cli.dir.as_ref(), "cli/src/");
        assert_eq!(cli.name.as_ref(), "Button.tsx");
        assert_eq!(cli.kind, FileKind::Edit);
        assert_eq!((cli.added, cli.removed, cli.calls), (2, 2, 2));
        assert!(!cli.plain_write);
        assert!(cli.lines.iter().any(|line| line.kind == LineKind::Gap && line.text.as_ref() == "Edit 2"));
    }

    #[test]
    fn a_replaced_file_counts_what_changed_as_git_does() {
        let turn = turn_with(vec![tool_step(
            "w1",
            "write",
            json!({ "path": "src/compare.js", "content": "a\nb\nc\n" }),
            Some(("Replaced src/compare.js (3 lines, +10 −13).", false)),
        )]);
        let card = from_turn(&turn, "turn-1", Path::new("/repo"), &HashSet::new()).unwrap();
        assert_eq!(card.heading.as_ref(), "Edited 1 file");
        assert_eq!((card.added, card.removed), (10, 13));
        // A result from before the counts were reported still reads as the whole file.
        let old = turn_with(vec![tool_step("w1", "write", json!({ "path": "a.rs", "content": "1\n2" }), Some(("Replaced a.rs (2 lines).", false)))]);
        assert_eq!(from_turn(&old, "turn-1", Path::new("/repo"), &HashSet::new()).map(|card| (card.added, card.removed)), Some((2, 0)));
    }

    #[test]
    fn headings_match_single_and_uniform_kinds() {
        let one_edit = turn_with(vec![tool_step(
            "e1",
            "edit",
            json!({ "path": "./crates/wack-llm/src/sse.rs", "edits": [{ "old_text": "a", "new_text": "b\nc" }] }),
            Some(("Edited crates/wack-llm/src/sse.rs (1 change).", false)),
        )]);
        let card = from_turn(&one_edit, "turn-1", Path::new("/repo"), &HashSet::new()).unwrap();
        assert_eq!(card.heading.as_ref(), "Edited 1 file");
        assert_eq!(card.files[0].path.as_ref(), "crates/wack-llm/src/sse.rs");
        assert_eq!(card.files[0].dir.as_ref(), "…/src/");
        assert_eq!(card.files[0].name.as_ref(), "sse.rs");
        assert_eq!((card.added, card.removed), (2, 1));

        let two_creates = turn_with(vec![
            tool_step("w1", "write", json!({ "path": "a.rs", "content": "1\n2" }), Some(("Created a.rs (2 lines).", false))),
            tool_step("w2", "write", json!({ "path": "src/b.rs", "content": "3" }), Some(("Created src/b.rs (1 line).", false))),
        ]);
        let card = from_turn(&two_creates, "turn-2", Path::new("/repo"), &HashSet::new()).unwrap();
        assert_eq!(card.heading.as_ref(), "Created 2 files");
        assert_eq!((card.added, card.removed), (3, 0));
        assert_eq!(card.files[0].dir.as_ref(), "");
        assert_eq!(card.files[1].dir.as_ref(), "src/");
    }

    #[test]
    fn rewriting_a_created_file_replaces_its_line_count_without_double_counting() {
        let turn = turn_with(vec![
            tool_step("w1", "write", json!({ "path": "notes.md", "content": "one\ntwo\nthree\n" }), Some(("Created notes.md (3 lines).", false))),
            tool_step("w2", "write", json!({ "path": "notes.md", "content": "one\ntwo\nthree\nfour\nfive\n" }), Some(("Replaced notes.md (5 lines).", false))),
        ]);
        let card = from_turn(&turn, "turn-1", Path::new("/repo"), &HashSet::new()).unwrap();
        assert_eq!(card.heading.as_ref(), "Created 1 file");
        assert_eq!((card.added, card.removed), (5, 0));
        assert_eq!(card.files[0].calls, 2);
        assert!(card.files[0].plain_write);
        assert_eq!(card.files[0].copy_text.as_ref(), "one\ntwo\nthree\nfour\nfive\n");
    }

    #[test]
    fn replacing_an_existing_file_with_write_counts_as_edit() {
        let turn = turn_with(vec![tool_step(
            "w1",
            "write",
            json!({ "path": "README.md", "content": "new content\n" }),
            Some(("Replaced README.md (1 line).", false)),
        )]);
        let card = from_turn(&turn, "turn-1", Path::new("/repo"), &HashSet::new()).unwrap();
        assert_eq!(card.heading.as_ref(), "Edited 1 file");
        assert_eq!(card.files[0].kind, FileKind::Edit);
    }

    #[test]
    fn returns_none_when_no_file_changes_succeeded() {
        let turn = turn_with(vec![
            tool_step("r1", "read", json!({ "path": "a.rs" }), Some(("content", false))),
            tool_step("e1", "edit", json!({ "path": "a.rs", "edits": [] }), Some(("failed", true))),
        ]);
        assert!(from_turn(&turn, "turn-1", Path::new("/repo"), &HashSet::new()).is_none());
    }

    #[test]
    fn open_set_controls_file_diff_and_list_expansion_and_holds_reports_keys() {
        let turn = turn_with(vec![
            tool_step("e1", "edit", json!({ "path": "a.rs", "edits": [{ "old_text": "1", "new_text": "2" }] }), Some(("Edited a.rs (1 change).", false))),
            tool_step("e2", "edit", json!({ "path": "b.rs", "edits": [{ "old_text": "1", "new_text": "2" }] }), Some(("Edited b.rs (1 change).", false))),
            tool_step("e3", "edit", json!({ "path": "c.rs", "edits": [{ "old_text": "1", "new_text": "2" }] }), Some(("Edited c.rs (1 change).", false))),
            tool_step("e4", "edit", json!({ "path": "d.rs", "edits": [{ "old_text": "1", "new_text": "2" }] }), Some(("Edited d.rs (1 change).", false))),
        ]);
        let mut open = HashSet::new();
        open.insert("turn-1-changes-more".into());
        open.insert("turn-1-changes-file-1".into());
        let card = from_turn(&turn, "turn-1", Path::new("/repo"), &open).unwrap();
        assert!(card.expanded);
        assert!(!card.files[0].open);
        assert!(card.files[1].open);
        assert!(card.holds("turn-1-changes-more"));
        assert!(card.holds("turn-1-changes-file-3"));
        assert!(!card.holds("turn-2-changes-more"));
    }

    #[test]
    fn disambiguates_root_file_against_nested_file_with_same_name() {
        let turn = turn_with(vec![
            tool_step("e1", "edit", json!({ "path": "lib.rs", "edits": [{ "old_text": "a", "new_text": "b" }] }), Some(("Edited.", false))),
            tool_step("e2", "edit", json!({ "path": "src/lib.rs", "edits": [{ "old_text": "a", "new_text": "b" }] }), Some(("Edited.", false))),
        ]);
        let card = from_turn(&turn, "turn-1", Path::new("/repo"), &HashSet::new()).unwrap();
        assert_eq!(card.files[0].dir.as_ref(), "");
        assert_eq!(card.files[1].dir.as_ref(), "src/");
    }
}
