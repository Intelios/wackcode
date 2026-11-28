//! The Changes pane: what changed in the project since its last commit, file by file, read-only.
//!
//! Git runs off the UI thread (`wack_core::git`); the pane only ever draws the last answer it
//! got. It refreshes when it is shown, when the agent finishes a tool or a run, and on request.
//! A refresh asked for while one is running is not lost: it runs again once the first is done.
//!
//! The diff is drawn through GPUI's virtual list, so a large diff costs only what is on screen.
//! Long lines wrap rather than scroll sideways, so nothing is ever hidden.

use std::path::PathBuf;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, ListAlignment, ListState, SharedString, Task, div, list,
    prelude::*, px, uniform_list,
};
use wack_core::git::{self, ChangedFile, Changes, FileStatus};

use crate::{
    tags::Tag,
    theme::Theme,
    transcript::{self, DiffLine, LineKind},
    typography::{Type, Typography},
};

/// Below this width the file list sits above the diff rather than beside it.
const SIDE_BY_SIDE_WIDTH: f32 = 620.;
const FILE_ROW_HEIGHT: f32 = 26.;
/// Longer lines are cut: a minified file would otherwise make one enormous row.
const MAX_LINE_CHARS: usize = 1_000;

enum State {
    Loading,
    NotARepository,
    Failed(String),
    Ready { root: PathBuf, files: Vec<ChangedFile> },
}

enum Diff {
    None,
    Loading,
    Ready(Vec<DiffLine>),
    Failed(String),
}

pub struct ChangesPane {
    folder: PathBuf,
    state: State,
    /// The selected file's path, kept across refreshes while the file is still changed.
    selected: Option<String>,
    /// A file asked for before the latest refresh landed, applied if Git lists it.
    pending_select: Option<String>,
    diff: Diff,
    /// The path `diff` belongs to, so a refresh of the same file keeps the reader's place.
    diff_path: Option<String>,
    lines: ListState,
    refreshing: Option<Task<()>>,
    again: bool,
    loading_diff: Option<Task<()>>,
    pub focus: FocusHandle,
}

impl ChangesPane {
    pub fn new(folder: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut pane = Self::empty(folder, cx);
        pane.refresh(cx);
        pane
    }

    fn empty(folder: PathBuf, cx: &mut Context<Self>) -> Self {
        Self {
            folder,
            state: State::Loading,
            selected: None,
            pending_select: None,
            diff: Diff::None,
            diff_path: None,
            lines: ListState::new(0, ListAlignment::Top, px(600.)),
            refreshing: None,
            again: false,
            loading_diff: None,
            focus: cx.focus_handle(),
        }
    }

    /// A pane showing fixed files and one diff, for gallery scenes.
    pub fn fixture(files: Vec<ChangedFile>, selected: &str, diff: &str, cx: &mut Context<Self>) -> Self {
        let mut pane = Self::empty(PathBuf::from("/"), cx);
        pane.state = State::Ready { root: PathBuf::from("/"), files };
        pane.selected = Some(selected.to_string());
        pane.set_diff(selected.to_string(), Ok(diff.to_string()));
        pane
    }

    /// How many files changed, once known.
    pub fn count(&self) -> Option<usize> {
        match &self.state {
            State::Ready { files, .. } => Some(files.len()),
            _ => None,
        }
    }

    /// Asks Git again. Cheap to call often: overlapping requests fold into one more run.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing.is_some() {
            self.again = true;
            return;
        }
        let folder = self.folder.clone();
        let lookup = cx.background_spawn(async move { git::changes(&folder) });
        self.refreshing = Some(cx.spawn(async move |pane, cx| {
            let result = lookup.await;
            let _ = pane.update(cx, |pane, cx| {
                pane.refreshing = None;
                pane.apply(result, cx);
                if std::mem::take(&mut pane.again) {
                    pane.refresh(cx);
                }
            });
        }));
    }

    fn apply(&mut self, result: Result<Changes, String>, cx: &mut Context<Self>) {
        self.state = match result {
            Ok(Changes::NotARepository) => State::NotARepository,
            Ok(Changes::Repository { root, files }) => State::Ready { root, files },
            Err(message) => State::Failed(message),
        };
        let files = match &self.state {
            State::Ready { files, .. } => files.as_slice(),
            _ => &[],
        };
        if let Some(wanted) = self.pending_select.take()
            && files.iter().any(|file| file.path == wanted)
        {
            self.selected = Some(wanted);
        }
        if !self.selected.as_ref().is_some_and(|selected| files.iter().any(|file| &file.path == selected)) {
            self.selected = files.first().map(|file| file.path.clone());
        }
        match self.selected.clone() {
            Some(path) => self.load_diff(path, cx),
            None => {
                self.diff = Diff::None;
                self.diff_path = None;
                self.lines.reset(0);
            }
        }
        cx.notify();
    }

    pub fn select(&mut self, path: &str, cx: &mut Context<Self>) {
        self.pending_select = None;
        if self.selected.as_deref() != Some(path) {
            self.selected = Some(path.to_string());
            self.load_diff(path.to_string(), cx);
            cx.notify();
        }
    }

    /// Selects `path` right away if it is already in Git's list, or remembers it for the next
    /// refresh without disturbing the current selection if the file is no longer modified.
    pub fn select_if_present(&mut self, path: &str, cx: &mut Context<Self>) {
        if let State::Ready { files, .. } = &self.state
            && files.iter().any(|file| file.path == path)
        {
            self.select(path, cx);
            return;
        }
        self.pending_select = Some(path.to_string());
    }

    fn load_diff(&mut self, path: String, cx: &mut Context<Self>) {
        let State::Ready { root, files } = &self.state else {
            return;
        };
        let Some(file) = files.iter().find(|file| file.path == path).cloned() else {
            return;
        };
        if self.diff_path.as_deref() != Some(path.as_str()) {
            self.diff = Diff::Loading;
        }
        let root = root.clone();
        let read = cx.background_spawn(async move { git::file_diff(&root, &file) });
        self.loading_diff = Some(cx.spawn(async move |pane, cx| {
            let text = read.await;
            let _ = pane.update(cx, |pane, cx| {
                // Only the diff of the file still selected is shown.
                if pane.selected.as_deref() == Some(path.as_str()) {
                    pane.set_diff(path, text);
                    cx.notify();
                }
            });
        }));
    }

    fn set_diff(&mut self, path: String, text: Result<String, String>) {
        let same_file = self.diff_path.as_deref() == Some(path.as_str());
        let old_count = match &self.diff {
            Diff::Ready(lines) => lines.len(),
            _ => 0,
        };
        self.diff = match text {
            Ok(text) => Diff::Ready(parse(&text)),
            Err(message) => Diff::Failed(message),
        };
        let count = match &self.diff {
            Diff::Ready(lines) => lines.len(),
            _ => 0,
        };
        if same_file && count == old_count {
            self.lines.remeasure();
        } else if same_file {
            self.lines.splice(0..old_count, count);
        } else {
            self.lines.reset(count);
        }
        self.diff_path = Some(path);
    }
}

/// A unified diff (`git diff` output for one file) as lines to draw. Lines are numbered in the
/// new file, removed lines in the old; each hunk after the first, or one that starts past the
/// top of the file, is marked with a gap carrying Git's section heading.
pub fn parse(text: &str) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    let (mut old, mut new) = (0u32, 0u32);
    let mut in_hunk = false;
    let line = |kind, number, text: &str| {
        let mut text = text.trim_end_matches('\r').replace('\t', "    ");
        if text.chars().count() > MAX_LINE_CHARS {
            text = text.chars().take(MAX_LINE_CHARS).collect::<String>() + "…";
        }
        DiffLine { kind, number, text: text.into() }
    };
    for raw in text.lines() {
        if let Some(rest) = raw.strip_prefix("@@ ") {
            let (ranges, section) = rest.split_once("@@").unwrap_or((rest, ""));
            for part in ranges.split_whitespace() {
                let start = |range: &str| range.split(',').next().and_then(|start| start.parse().ok()).unwrap_or(0);
                if let Some(range) = part.strip_prefix('-') {
                    old = start(range);
                } else if let Some(range) = part.strip_prefix('+') {
                    new = start(range);
                }
            }
            if in_hunk || new > 1 {
                lines.push(line(LineKind::Gap, None, section.trim()));
            }
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            if raw.starts_with("Binary files") {
                lines.push(line(LineKind::Gap, None, "A binary file: its contents are not shown."));
            }
            continue;
        }
        match raw.as_bytes().first() {
            Some(b'+') => {
                lines.push(line(LineKind::Added, Some(new), &raw[1..]));
                new += 1;
            }
            Some(b'-') => {
                lines.push(line(LineKind::Removed, Some(old), &raw[1..]));
                old += 1;
            }
            Some(b'\\') => lines.push(line(LineKind::Gap, None, raw.trim_start_matches('\\').trim())),
            Some(b' ') | None => {
                lines.push(line(LineKind::Context, Some(new), raw.get(1..).unwrap_or("")));
                old += 1;
                new += 1;
            }
            _ => {}
        }
    }
    lines
}

/// The letter and colour a file's status is drawn with.
fn status_mark(status: FileStatus, theme: &Theme) -> (&'static str, gpui::Hsla) {
    match status {
        FileStatus::Added => ("A", theme.diff_added),
        FileStatus::Untracked => ("U", theme.diff_added),
        FileStatus::Modified => ("M", theme.text_soft),
        FileStatus::Deleted => ("D", theme.diff_removed),
        FileStatus::Renamed => ("R", theme.text_soft),
        FileStatus::Conflicted => ("!", theme.danger),
    }
}

/// The pane at `width` points wide. Drawn as a function rather than a view because the layout
/// depends on the width the dock gives it.
pub fn view(pane: &Entity<ChangesPane>, width: f32, theme: &Theme, cx: &App) -> AnyElement {
    let state = pane.read(cx);
    let quiet = |title: &'static str, detail: SharedString| {
        div()
            .size_full()
            .flex()
            .flex_col()
            .px(px(20.))
            .pt(px(28.))
            .gap(px(6.))
            .child(div().typography(Type::Title).text_color(theme.text).child(title))
            .child(div().max_w(px(360.)).typography(Type::Ui).text_color(theme.text_dim).child(detail))
            .into_any_element()
    };
    let files = match &state.state {
        State::Loading => return quiet("Looking…", "Asking Git what changed.".into()),
        State::NotARepository => {
            return quiet("Not under Git.", "This folder is not a Git repository, so there are no changes to show.".into());
        }
        State::Failed(message) => return quiet("Git could not answer.", message.clone().into()),
        State::Ready { files, .. } if files.is_empty() => {
            return quiet("Nothing changed.", "The project matches its last commit.".into());
        }
        State::Ready { files, .. } => files,
    };

    let (added, removed) = files.iter().filter_map(|file| file.lines).fold((0, 0), |sum, (a, r)| (sum.0 + a, sum.1 + r));
    let summary = div()
        .flex_none()
        .h(px(30.))
        .px(px(14.))
        .flex()
        .items_center()
        .gap(px(10.))
        .typography(Type::Small)
        .child(div().text_color(theme.text_soft).child(if files.len() == 1 {
            "1 file".to_string()
        } else {
            format!("{} files", files.len())
        }))
        .child(div().text_color(theme.diff_added).child(format!("+{added}")))
        .child(div().text_color(theme.diff_removed).child(format!("−{removed}")))
        .child(div().flex_1())
        .child(
            div()
                .id("changes-refresh")
                .px(px(6.))
                .py(px(2.))
                .rounded(px(5.))
                .text_color(theme.text_dim)
                .hover(|word| word.text_color(theme.text).bg(theme.hover))
                .cursor_pointer().on_click({
                    let pane = pane.clone();
                    move |_, _, cx| pane.update(cx, |pane, cx| pane.refresh(cx))
                })
                .child("Refresh")
                .tag("changes-refresh"),
        );

    let rows = files.len();
    let file_list = uniform_list("changes-files", rows, {
        let pane = pane.clone();
        move |range, _, cx| {
            let theme = Theme::get(cx).clone();
            let state = pane.read(cx);
            let State::Ready { files, .. } = &state.state else {
                return Vec::new();
            };
            range
                .filter_map(|index| files.get(index).map(|file| (index, file)))
                .map(|(index, file)| {
                    let selected = state.selected.as_deref() == Some(file.path.as_str());
                    let (mark, mark_colour) = status_mark(file.status, &theme);
                    let (folder, name) = match file.path.rsplit_once('/') {
                        Some((folder, name)) => (Some(folder.to_string()), name.to_string()),
                        None => (None, file.path.clone()),
                    };
                    let path = file.path.clone();
                    let pane = pane.clone();
                    div()
                        .id(("changed-file", index))
                        .w_full()
                        .h(px(FILE_ROW_HEIGHT))
                        .px(px(14.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .when(selected, |row| row.bg(theme.accent_wash))
                        .when(!selected, |row| row.hover(|row| row.bg(theme.hover)))
                        .cursor_pointer().on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.select(&path, cx)))
                        .child(div().flex_none().w(px(10.)).typography(Type::Mono).text_color(mark_colour).child(mark))
                        .child(
                            div()
                                .flex_none()
                                .max_w(relative_name_width())
                                .truncate()
                                .typography(if selected { Type::UiStrong } else { Type::Ui })
                                .text_color(if selected { theme.text } else { theme.text_body })
                                .child(name),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .typography(Type::Small)
                                .text_color(theme.text_faint)
                                .child(folder.unwrap_or_default()),
                        )
                        .when_some(file.lines, |row, (added, removed)| {
                            row.child(
                                div()
                                    .flex_none()
                                    .flex()
                                    .gap(px(6.))
                                    .typography(Type::Small)
                                    .when(added > 0, |counts| counts.child(div().text_color(theme.diff_added).child(format!("+{added}"))))
                                    .when(removed > 0, |counts| {
                                        counts.child(div().text_color(theme.diff_removed).child(format!("−{removed}")))
                                    }),
                            )
                        })
                })
                .collect()
        }
    })
    .size_full();

    let diff = diff_view(pane, theme, cx);
    let side_by_side = width >= SIDE_BY_SIDE_WIDTH;
    let body = if side_by_side {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(div().flex_none().w(px(260.)).h_full().border_r_1().border_color(theme.border).child(file_list))
            .child(div().flex_1().min_w_0().h_full().child(diff))
    } else {
        // The list takes what its files need, up to a third of the pane; the diff the rest.
        let list_height = (rows as f32 * FILE_ROW_HEIGHT).min(8.5 * FILE_ROW_HEIGHT);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(div().flex_none().h(px(list_height)).max_h(gpui::relative(0.34)).child(file_list))
            .child(div().flex_1().min_h_0().border_t_1().border_color(theme.border).child(diff))
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .track_focus(&state.focus)
        .child(summary)
        .child(div().flex_none().h(px(1.)).bg(theme.border))
        .child(body)
        .into_any_element()
}

/// How wide a file's name may grow before its folder gets the rest.
fn relative_name_width() -> gpui::DefiniteLength {
    gpui::relative(0.7)
}

fn diff_view(pane: &Entity<ChangesPane>, theme: &Theme, cx: &App) -> AnyElement {
    let state = pane.read(cx);
    let Some(path) = state.selected.clone() else {
        return div().into_any_element();
    };
    let note = |text: SharedString, colour| {
        div().px(px(14.)).pt(px(14.)).typography(Type::Ui).text_color(colour).child(text).into_any_element()
    };
    let content = match &state.diff {
        Diff::None | Diff::Loading => note("Reading the diff…".into(), theme.text_dim),
        Diff::Failed(message) => note(message.clone().into(), theme.danger),
        Diff::Ready(lines) if lines.is_empty() => note("Only the file’s mode or name changed.".into(), theme.text_dim),
        Diff::Ready(_) => {
            let pane = pane.clone();
            list(state.lines.clone(), move |index, _, cx| {
                let theme = Theme::get(cx);
                match &pane.read(cx).diff {
                    // A full-width column, so a row's ground spans the pane and long lines wrap.
                    Diff::Ready(lines) => match lines.get(index) {
                        Some(line) => div().w_full().flex().flex_col().child(transcript::diff_line(line, theme)).into_any_element(),
                        None => div().into_any_element(),
                    },
                    _ => div().into_any_element(),
                }
            })
            .size_full()
            .pb(px(12.))
            .into_any_element()
        }
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_none()
                .h(px(30.))
                .px(px(14.))
                .flex()
                .items_center()
                .typography(Type::Mono)
                .text_color(theme.text_soft)
                .child(div().min_w_0().truncate().child(path)),
        )
        .child(div().flex_1().min_h_0().child(content))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lines: &[DiffLine]) -> Vec<(LineKind, Option<u32>, String)> {
        lines.iter().map(|line| (line.kind, line.number, line.text.to_string())).collect()
    }

    #[test]
    fn parses_hunks_with_numbers_and_gaps() {
        let text = "diff --git a/x.py b/x.py\nindex 1..2 100644\n--- a/x.py\n+++ b/x.py\n\
                    @@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n@@ -10,2 +10,3 @@ def f():\n x\n+\ty\n z\n\\ No newline at end of file\n";
        assert_eq!(
            kinds(&parse(text)),
            vec![
                (LineKind::Context, Some(1), "a".into()),
                (LineKind::Removed, Some(2), "b".into()),
                (LineKind::Added, Some(2), "B".into()),
                (LineKind::Context, Some(3), "c".into()),
                (LineKind::Gap, None, "def f():".into()),
                (LineKind::Context, Some(10), "x".into()),
                (LineKind::Added, Some(11), "    y".into()),
                (LineKind::Context, Some(12), "z".into()),
                (LineKind::Gap, None, "No newline at end of file".into()),
            ]
        );
    }

    #[test]
    fn a_first_hunk_past_the_top_starts_with_a_gap() {
        let lines = parse("@@ -40 +40,2 @@\n x\n+y\n");
        assert_eq!(lines[0].kind, LineKind::Gap);
        assert_eq!(lines[1].number, Some(40));
    }

    #[test]
    fn binary_files_and_new_files() {
        assert_eq!(kinds(&parse("Binary files a/x.png and b/x.png differ\n"))[0].0, LineKind::Gap);
        let new = parse("--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n");
        assert_eq!(kinds(&new), vec![(LineKind::Added, Some(1), "one".into()), (LineKind::Added, Some(2), "two".into())]);
    }
}
