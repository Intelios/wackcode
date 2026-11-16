//! Settings › Memory: every project the agent keeps notes for, after version 2's page. One
//! folder per repository (worktrees share it), listed as a run loads them.
//!
//! - The head's switch is the master switch; each project card has its own. Both apply from a
//!   chat's next message, since a run reads memory when it starts.
//! - A project with notes gets a card; projects still waiting for a first note share one.
//! - A note opens in an editor in place of the list. Its name follows the title until the user
//!   edits the name, as in version 2.
//! - Deleting a note or a project's folder asks first, inside the page, and moves the files to
//!   the macOS Trash, so it can be undone from Finder.
//!
//! The overview is read from disk when the section opens and after every change, never while
//! drawing. The editor's fields live in [`MemoryEditor`], owned by the app state.

use std::path::PathBuf;

use gpui::{AnyElement, App, AppContext, Entity, SharedString, div, prelude::*, px};
use wack_core::memory::{Draft, Kind, MemoryOverview, MemoryProject, Note, draft_issue, note_name};

use crate::{
    actions::{
        AskDeleteMemory, AskRemoveMemoryFolder, CancelMemoryDelete, ChooseMemoryKind, CloseMemoryEditor, ConfirmMemoryDelete,
        EditMemory, NewMemory, RevealMemory, SaveMemory, SwitchProjectMemory, ToggleMemory,
    },
    button::{Button, Kind as ButtonKind},
    icon::Icon,
    settings::{field, text_input},
    settings_kit::{card, dispatch, head, pill, row, segmented},
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    tools_page::switch,
    tooltip::tip,
    typography::{Type, Typography},
};

/// Each kind's word and what it is for, as the editor offers them.
pub const KINDS: [(Kind, &str, &str); 4] = [
    (Kind::User, "User", "Role, expertise and lasting preferences"),
    (Kind::Feedback, "Feedback", "Corrections and confirmed approaches"),
    (Kind::Project, "Project", "Ongoing work, decisions, deadlines"),
    (Kind::Reference, "Reference", "Where information lives outside the project"),
];

fn kind_word(kind: Kind) -> &'static str {
    KINDS.iter().find(|(candidate, _, _)| *candidate == kind).map(|(_, word, _)| *word).unwrap_or("Project")
}

/// How memory works, for a page with no notes yet.
const STEPS: [(&str, &str); 3] = [
    ("You mention something worth keeping", "A preference, a correction, a decision with a tricky edge."),
    ("The agent saves a small note", "Kept on this Mac with WackCode's data, never inside the project."),
    ("Later chats recall it", "Only when the note looks relevant to what you ask."),
];

/// A delete waiting for the user to confirm it.
#[derive(Clone, Debug, PartialEq)]
pub enum MemoryConfirm {
    Note { dir: PathBuf, name: String, title: String },
    Folder { key: String, dir: PathBuf, name: String, notes: usize },
}

/// The note being written or edited, kept while the user looks elsewhere.
pub struct MemoryEditor {
    pub dir: PathBuf,
    /// The project's name, for the editor's head.
    pub project: SharedString,
    /// The note's name before this edit; `None` for a new note.
    pub previous: Option<String>,
    pub kind: Kind,
    pub title: Entity<TextField>,
    pub description: Entity<TextField>,
    pub name: Entity<TextField>,
    pub body: Entity<TextField>,
    /// The name is derived from the title until the user changes the name themself.
    pub name_follows: bool,
    /// Names already held in the folder, for the taken-name check.
    pub taken: Vec<String>,
    pub error: Option<SharedString>,
}

impl MemoryEditor {
    pub fn new(project: &MemoryProject, note: Option<&Note>, cx: &mut App) -> Self {
        let field = |placeholder: &'static str, cx: &mut App| cx.new(|cx| TextField::new(Type::Ui, placeholder, cx));
        let title = field("Run the tests after protocol changes", cx);
        let description = field("Protocol edits need cargo test before review", cx);
        let name = cx.new(|cx| TextField::new(Type::Mono, "project_run-the-tests", cx));
        let body = cx.new(|cx| {
            TextField::new(Type::Ui, "Any change to the protocol needs cargo test --workspace before review.", cx).multiline(12)
        });
        if let Some(note) = note {
            title.update(cx, |field, cx| field.set_text(&note.title, cx));
            description.update(cx, |field, cx| field.set_text(&note.description, cx));
            name.update(cx, |field, cx| field.set_text(&note.name, cx));
            body.update(cx, |field, cx| field.set_text(&note.body, cx));
        }
        Self {
            dir: project.dir.clone(),
            project: project.name.clone().into(),
            previous: note.map(|note| note.name.clone()),
            kind: note.map_or(Kind::Project, |note| note.kind),
            title,
            description,
            name,
            body,
            name_follows: note.is_none(),
            taken: project.notes.iter().map(|note| note.name.clone()).collect(),
            error: None,
        }
    }

    pub fn fields(&self) -> [Entity<TextField>; 4] {
        [self.title.clone(), self.description.clone(), self.name.clone(), self.body.clone()]
    }

    /// The name the title suggests, for a name that still follows it.
    pub fn suggested_name(&self, cx: &App) -> String {
        note_name(self.kind, self.title.read(cx).text())
    }

    pub fn draft(&self, cx: &App) -> Draft {
        let text = |field: &Entity<TextField>| field.read(cx).text().to_string();
        Draft {
            name: text(&self.name).trim().to_string(),
            kind: Some(self.kind),
            title: text(&self.title),
            description: text(&self.description),
            body: text(&self.body),
        }
    }

    /// Why the note cannot be saved yet, or `None` when it can. The store checks again on save.
    pub fn issue(&self, cx: &App) -> Option<SharedString> {
        let draft = self.draft(cx);
        let held = self.previous.as_deref() != Some(draft.name.as_str()) && self.taken.contains(&draft.name);
        if held && !draft.title.trim().is_empty() && !draft.body.trim().is_empty() {
            return Some(format!("A memory named {} already exists in this project.", draft.name).into());
        }
        draft_issue(&draft).map(Into::into)
    }

    /// What the page needs to draw the editor.
    pub fn view(&self, cx: &App) -> EditorData {
        EditorData {
            project: self.project.clone(),
            new: self.previous.is_none(),
            kind: self.kind,
            title: self.title.clone(),
            description: self.description.clone(),
            name: self.name.clone(),
            body: self.body.clone(),
            issue: self.error.clone().or_else(|| self.issue(cx)),
            error: self.error.is_some(),
        }
    }
}

#[derive(Clone)]
pub struct EditorData {
    pub project: SharedString,
    pub new: bool,
    pub kind: Kind,
    pub title: Entity<TextField>,
    pub description: Entity<TextField>,
    pub name: Entity<TextField>,
    pub body: Entity<TextField>,
    /// The first thing to fix, or the last save's failure.
    pub issue: Option<SharedString>,
    /// `issue` is a failed save rather than a field still to fill.
    pub error: bool,
}

#[derive(Clone, Default)]
pub struct MemoryData {
    pub overview: MemoryOverview,
    pub editor: Option<EditorData>,
    pub confirming: Option<MemoryConfirm>,
    pub error: Option<SharedString>,
}

/// A note's age in words, from its ISO 8601 `modified`, against `now` in Unix seconds.
pub fn ago(modified: &str, now: u64) -> Option<String> {
    let number = |range: std::ops::Range<usize>| modified.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13).unwrap_or(0), number(14..16).unwrap_or(0), number(17..19).unwrap_or(0));
    // Days from the civil date (Howard Hinnant's algorithm), so no date library is needed.
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let of_era = shifted - era * 400;
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    let days = era * 146_097 + of_cycle - 719_468;
    let then = days * 86_400 + hour * 3_600 + minute * 60 + second;
    let seconds = (now as i64 - then).max(0);
    Some(match seconds {
        0..60 => "just now".into(),
        60..3_600 => format!("{} min ago", seconds / 60),
        3_600..86_400 => format!("{} h ago", seconds / 3_600),
        86_400..172_800 => "yesterday".into(),
        172_800..2_592_000 => format!("{} days ago", seconds / 86_400),
        _ => modified.get(0..10)?.to_string(),
    })
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_secs()).unwrap_or(0)
}

fn count(notes: usize) -> String {
    format!("{notes} {}", if notes == 1 { "note" } else { "notes" })
}

pub fn page(data: MemoryData, theme: &Theme) -> AnyElement {
    if let Some(editor) = &data.editor {
        return editor_page(editor, theme).into_any_element();
    }
    let overview = &data.overview;
    let on = overview.enabled;
    let total = overview.notes();
    let status = if !on { "Off".to_string() } else if total == 0 { "No notes yet".to_string() } else { count(total) };
    let master = div()
        .id("memory-master")
        .cursor_pointer()
        .on_click(dispatch(ToggleMemory))
        .child(switch(on, theme))
        .tag("memory-master");
    let mut page = div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(head(
            "Memory",
            "Notes the agent keeps for itself, per project. Chats carry an index of their titles and read a note only when it looks relevant. Switches apply from a chat’s next message.",
            None,
            theme,
        ))
        .child(
            card(None, None, None, theme).child(
                row("Use memory", Some("Lets the agent save notes and recall them in later chats."))
                    .first()
                    .control(div().flex().items_center().gap(px(12.)).child(badge(status, theme)).child(master))
                    .view(theme),
            ),
        );
    if let Some(error) = data.error {
        page = page.child(div().typography(Type::Ui).text_color(theme.danger).child(error));
    }
    if !on {
        page = page.child(
            div()
                .typography(Type::Ui)
                .text_color(theme.text_dim)
                .child("Memory is switched off. The agent keeps nothing and its memory tools are withdrawn from every chat."),
        );
    }
    let confirming = data.confirming.as_ref();
    let (with_notes, waiting): (Vec<&MemoryProject>, Vec<&MemoryProject>) =
        overview.projects.iter().partition(|project| !project.notes.is_empty());
    if with_notes.is_empty() {
        page = page.child(empty(theme));
    }
    for (index, project) in with_notes.into_iter().enumerate() {
        page = page.child(project_card(index, project, on, confirming, theme));
    }
    if !waiting.is_empty() {
        page = page.child(waiting_card(&waiting, on, confirming, theme));
    }
    page.into_any_element()
}

fn badge(text: impl Into<SharedString>, theme: &Theme) -> impl IntoElement {
    div()
        .flex_none()
        .h(px(20.))
        .px(px(8.))
        .rounded_full()
        .flex()
        .items_center()
        .bg(theme.hover)
        .typography(Type::Small)
        .text_color(theme.text_soft)
        .child(text.into())
}

fn empty(theme: &Theme) -> impl IntoElement {
    let mut steps = card(Some("No memories yet"), None, None, theme);
    for (index, (title, detail)) in STEPS.iter().enumerate() {
        steps = steps.child(
            div()
                .px(px(18.))
                .py(px(12.))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .items_start()
                .gap(px(14.))
                .child(
                    div()
                        .flex_none()
                        .size(px(24.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme.accent_wash)
                        .typography(Type::UiStrong)
                        .text_color(theme.accent_text)
                        .child((index + 1).to_string()),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().typography(Type::UiStrong).text_color(theme.text).child(*title))
                        .child(div().typography(Type::Small).text_color(theme.text_dim).child(*detail)),
                ),
        );
    }
    steps
}

/// A round icon button that dispatches `action`, without the click reaching the row beneath.
fn icon_button(
    id: impl Into<gpui::ElementId>,
    icon: Icon,
    words: &'static str,
    danger: bool,
    action: impl gpui::Action,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let action = action.boxed_clone();
    div()
        .id(id)
        .flex_none()
        .size(px(28.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|button| button.bg(theme.hover))
        .tooltip(tip(words))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(action.boxed_clone(), cx);
        })
        .child(icon.view(px(14.), if danger { theme.text_soft } else { theme.text_dim }))
}

/// The in-page question before anything moves to the Trash.
fn confirm_box(confirm: &MemoryConfirm, theme: &Theme) -> impl IntoElement {
    let (title, body) = match confirm {
        MemoryConfirm::Note { title, .. } => (
            format!("Delete “{title}”?"),
            "Its file moves to the Trash, so chats stop recalling it. You can put it back from the Trash.".to_string(),
        ),
        MemoryConfirm::Folder { name, notes, .. } => (
            format!("Delete “{name}”’s memory?"),
            if *notes > 0 {
                format!("Its {} to the Trash with the folder.", if *notes == 1 { "note moves".to_string() } else { format!("{notes} notes move") })
            } else {
                "The folder is removed. If this project is still added, a fresh empty folder appears the next time a chat starts in it.".to_string()
            },
        ),
    };
    div()
        .mx(px(18.))
        .my(px(12.))
        .p(px(14.))
        .rounded(px(12.))
        .bg(theme.window)
        .border_1()
        .border_color(theme.border_strong)
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child(title))
                .child(div().typography(Type::Ui).text_color(theme.text_dim).child(body)),
        )
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(Button::new("memory-keep", "Keep", ButtonKind::Secondary).action(CancelMemoryDelete).tag("memory-keep"))
                .child(Button::new("memory-trash", "Move to Trash", ButtonKind::Danger).action(ConfirmMemoryDelete).tag("memory-trash")),
        )
}

fn project_card(index: usize, project: &MemoryProject, master: bool, confirming: Option<&MemoryConfirm>, theme: &Theme) -> impl IntoElement {
    let live = master && project.enabled;
    let kinds: Vec<String> = KINDS
        .iter()
        .filter_map(|(kind, word, _)| {
            let notes = project.notes.iter().filter(|note| note.kind == *kind).count();
            (notes > 0).then(|| format!("{notes} {}", word.to_lowercase()))
        })
        .collect();
    let toggle = div()
        .id(("memory-project-switch", index))
        .cursor_pointer()
        .on_click(dispatch(SwitchProjectMemory { key: project.key.clone(), on: !project.enabled }))
        .child(switch(live, theme))
        .tag("memory-project-switch");
    let header = div()
        .px(px(18.))
        .pt(px(16.))
        .pb(px(12.))
        .flex()
        .items_center()
        .gap(px(12.))
        .child(monogram(&project.name, theme))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child(project.name.clone()))
                .child(
                    div()
                        .typography(Type::Small)
                        .text_color(theme.text_faint)
                        .truncate()
                        .child(format!("{} · {} · {}", shown_path(&project.path), count(project.notes.len()), kinds.join(", "))),
                ),
        )
        .child(
            pill(("memory-new", index), "New memory", Some(Icon::Plus), false, theme)
                .on_click(dispatch(NewMemory { dir: project.dir.clone() }))
                .tag("memory-new"),
        )
        .child(
            icon_button(("memory-reveal-folder", index), Icon::Folder, "Show the folder in Finder", false, RevealMemory { path: project.dir.clone() }, theme),
        )
        .child(
            icon_button(("memory-remove-folder", index), Icon::Trash, "Delete this project's memory", true, AskRemoveMemoryFolder { key: project.key.clone() }, theme)
                .tag("memory-remove-folder"),
        )
        .child(toggle);
    let mut body = card(None, None, None, theme).when(!live, |card| card.opacity(0.7)).child(header);
    if let Some(confirm @ MemoryConfirm::Folder { key, .. }) = confirming
        && *key == project.key
    {
        body = body.child(confirm_box(confirm, theme));
    }
    let now = now();
    for note in &project.notes {
        body = body.child(note_row(project, note, now, theme));
        if let Some(confirm @ MemoryConfirm::Note { dir, name, .. }) = confirming
            && *dir == project.dir
            && *name == note.name
        {
            body = body.child(confirm_box(confirm, theme));
        }
    }
    body
}

fn monogram(name: &str, theme: &Theme) -> impl IntoElement {
    let letter: String = name.chars().find(|character| character.is_alphanumeric()).map(|letter| letter.to_uppercase().collect()).unwrap_or_default();
    div()
        .flex_none()
        .size(px(32.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.accent_wash)
        .typography(Type::UiStrong)
        .text_color(theme.accent_text)
        .child(letter)
}

fn shown_path(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_string(),
    }
}

fn note_row(project: &MemoryProject, note: &Note, now: u64, theme: &Theme) -> impl IntoElement {
    let id = SharedString::from(format!("memory-note-{}-{}", project.key, note.name));
    let description: AnyElement = if note.description.is_empty() {
        div()
            .typography(Type::Small)
            .italic()
            .text_color(theme.text_faint)
            .child("No description — the index shows the title alone")
            .into_any_element()
    } else {
        div().typography(Type::Small).text_color(theme.text_dim).child(note.description.clone()).into_any_element()
    };
    let when = note.modified.as_deref().and_then(|modified| ago(modified, now));
    let file = project.dir.join(format!("{}.md", note.name));
    div()
        .id(id.clone())
        .px(px(18.))
        .py(px(11.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(14.))
        .cursor_pointer()
        .hover(|row| row.bg(theme.hover))
        .on_click(dispatch(EditMemory { dir: project.dir.clone(), name: note.name.clone() }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().min_w_0().truncate().typography(Type::UiStrong).text_color(theme.text).child(note.title.clone()))
                        .child(badge(kind_word(note.kind), theme))
                        .children(when.map(|when| div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(when))),
                )
                .child(description)
                .child(div().typography(Type::Mono).text_color(theme.text_faint).child(note.name.clone())),
        )
        .child(
            icon_button(SharedString::from(format!("{id}-find")), Icon::Search, "Show this note's file in Finder", false, RevealMemory { path: file }, theme),
        )
        .child(
            icon_button(
                SharedString::from(format!("{id}-delete")),
                Icon::Trash,
                "Delete this note",
                true,
                AskDeleteMemory { dir: project.dir.clone(), name: note.name.clone() },
                theme,
            )
            .tag("memory-delete"),
        )
        .tag("memory-note")
}

fn waiting_card(projects: &[&MemoryProject], master: bool, confirming: Option<&MemoryConfirm>, theme: &Theme) -> impl IntoElement {
    let mut waiting = card(Some("Waiting for a first note"), None, None, theme);
    for (index, project) in projects.iter().enumerate() {
        let live = master && project.enabled;
        waiting = waiting.child(
            div()
                .px(px(18.))
                .py(px(11.))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .gap(px(12.))
                .when(!live, |row| row.opacity(0.7))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().typography(Type::UiStrong).text_color(theme.text).child(project.name.clone()))
                        .child(div().truncate().typography(Type::Small).text_color(theme.text_faint).child(shown_path(&project.path))),
                )
                .child(
                    pill(("memory-write-one", index), "Write one", None, false, theme)
                        .on_click(dispatch(NewMemory { dir: project.dir.clone() }))
                        .tag("memory-write-one"),
                )
                .child(
                    icon_button(("memory-waiting-remove", index), Icon::Trash, "Delete this project's memory folder", true, AskRemoveMemoryFolder { key: project.key.clone() }, theme),
                )
                .child(
                    div()
                        .id(("memory-waiting-switch", index))
                        .cursor_pointer()
                        .on_click(dispatch(SwitchProjectMemory { key: project.key.clone(), on: !project.enabled }))
                        .child(switch(live, theme)),
                ),
        );
        if let Some(confirm @ MemoryConfirm::Folder { key, .. }) = confirming
            && *key == project.key
        {
            waiting = waiting.child(confirm_box(confirm, theme));
        }
    }
    waiting
}

fn editor_page(editor: &EditorData, theme: &Theme) -> impl IntoElement {
    let back = div()
        .id("back-memories")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .pb(px(18.))
        .cursor_pointer()
        .typography(Type::Ui)
        .text_color(theme.text_dim)
        .hover(|link| link.text_color(theme.text))
        .on_click(dispatch(CloseMemoryEditor))
        .child(
            Icon::ChevronRight
                .view(px(13.), theme.text_faint)
                .with_transformation(gpui::Transformation::rotate(gpui::radians(std::f32::consts::PI))),
        )
        .child("All memories")
        .tag("back-memories");
    let choices: Vec<(Kind, &'static str)> = KINDS.iter().map(|(kind, word, _)| (*kind, *word)).collect();
    let hint = KINDS.iter().find(|(kind, _, _)| *kind == editor.kind).map(|(_, _, hint)| *hint).unwrap_or("");
    let section = |content: AnyElement| div().px(px(18.)).py(px(13.)).border_t_1().border_color(theme.border).child(content);
    let body_field = div()
        .w_full()
        .min_h(px(120.))
        .px(px(11.))
        .py(px(9.))
        .rounded(px(9.))
        .bg(theme.window)
        .border_1()
        .border_color(theme.border)
        .child(editor.body.clone())
        .tag("memory-body");
    let status: AnyElement = match &editor.issue {
        Some(issue) => div()
            .flex_1()
            .typography(Type::Small)
            .text_color(if editor.error { theme.danger } else { theme.text_dim })
            .child(issue.clone())
            .into_any_element(),
        None => div().flex_1().into_any_element(),
    };
    div()
        .flex()
        .flex_col()
        .child(back)
        .child(head(
            if editor.new { "New memory" } else { "Edit memory" },
            "A note the agent can recall in later chats in this project. Keep it as small as it can be and still be useful.",
            None,
            theme,
        ))
        .child(
            card(Some("Project"), Some(editor.project.clone()), None, theme)
                .child(
                    div().px(px(18.)).py(px(13.)).border_t_1().border_color(theme.border).child(field(
                        "Type",
                        Some(format!("{hint}. The index groups notes by type.").into()),
                        segmented("memory-kind", &choices, editor.kind, |kind| Box::new(ChooseMemoryKind { kind }), theme),
                        theme,
                    )),
                )
                .child(section(
                    field("Title", Some("Shown in the index every chat carries.".into()), text_input(editor.title.clone(), "memory-title", theme), theme)
                        .into_any_element(),
                ))
                .child(section(
                    field(
                        "Description",
                        Some("One line saying when the note matters.".into()),
                        text_input(editor.description.clone(), "memory-description", theme),
                        theme,
                    )
                    .into_any_element(),
                ))
                .child(section(
                    field(
                        "Name",
                        Some("The id the agent recalls the note by. Usually derived from the title.".into()),
                        text_input(editor.name.clone(), "memory-name", theme),
                        theme,
                    )
                    .into_any_element(),
                ))
                .child(section(
                    field("Note", Some("What to remember. Keep it as small as it can be and still be useful.".into()), body_field, theme)
                        .into_any_element(),
                ))
                .child(
                    div()
                        .px(px(18.))
                        .py(px(12.))
                        .border_t_1()
                        .border_color(theme.border)
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(status)
                        .child(Button::new("memory-cancel", "Cancel", ButtonKind::Secondary).action(CloseMemoryEditor).tag("memory-cancel"))
                        .child(
                            Button::new("memory-save", if editor.new { "Create memory" } else { "Save" }, ButtonKind::Primary)
                                .action(SaveMemory)
                                .enabled(editor.issue.is_none() || editor.error)
                                .tag("memory-save"),
                        ),
                ),
        )
}

/// Settings › Memory's state, owned by the app so leaving the page keeps it.
#[derive(Default)]
pub struct MemoryState {
    pub overview: MemoryOverview,
    pub editor: Option<MemoryEditor>,
    pub confirming: Option<MemoryConfirm>,
    pub error: Option<SharedString>,
}

impl MemoryState {
    pub fn data(&self, cx: &App) -> MemoryData {
        MemoryData {
            overview: self.overview.clone(),
            editor: self.editor.as_ref().map(|editor| editor.view(cx)),
            confirming: self.confirming.clone(),
            error: self.error.clone(),
        }
    }
}

/// Moves a file or folder to the macOS Trash, where the user can put it back.
pub fn move_to_trash(path: &std::path::Path) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|error| format!("Could not move it to the Trash: {}", error.localizedDescription()))
}

/// Shows a file or folder selected in Finder.
pub fn reveal(path: &std::path::Path) {
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_as_words() {
        // 2026-11-16T12:00:00Z
        let now = 1_794_830_400;
        assert_eq!(ago("2026-11-16T12:00:00Z", now).unwrap(), "just now");
        assert_eq!(ago("2026-11-16T11:55:00Z", now).unwrap(), "5 min ago");
        assert_eq!(ago("2026-11-16T09:00:00Z", now).unwrap(), "3 h ago");
        assert_eq!(ago("2026-11-15T09:00:00Z", now).unwrap(), "yesterday");
        assert_eq!(ago("2026-11-10T12:00:00Z", now).unwrap(), "6 days ago");
        assert_eq!(ago("2026-01-02T12:00:00Z", now).unwrap(), "2026-01-02");
        assert_eq!(ago("2026-11-16T12:00:00.000Z", now).unwrap(), "just now", "version 2's milliseconds");
        assert!(ago("soon", now).is_none());
    }
}
