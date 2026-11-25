//! Settings › Skills: every folder skills load from and every skill in it, after version 2's
//! page (less its package browser, which version 3 cut).
//!
//! - **Your skills** (`~/.agents/skills`) is always on and the only folder WackCode writes to:
//!   its skills open in the editor, and New skill and Import add to it.
//! - **Other tools' folders** (Claude Code, Codex, OpenCode) are off until switched on; their
//!   skills open read-only, with Copy to Your skills to make one editable.
//! - **Added folders** load after the rest, in the order they were added.
//! - Every skill has its own switch, which takes it out of the prompt and the `/` menu alike. A
//!   skill that lost its name to an earlier one says so ("Not loaded"), and one the agent never
//!   chooses by itself says "/name only".
//!
//! The overview is read from disk when the section opens and after every change, never while
//! drawing. The editor's fields live in [`SkillEditor`], owned by the app state, so leaving the
//! page keeps a half-written skill. Switches apply from a chat's next message.

use std::path::PathBuf;

use gpui::{AnyElement, App, AppContext, Entity, SharedString, div, prelude::*, px};
use wack_core::skills::{FolderKind, Skill, SkillEntry, SkillFolder, file_key, Kind};

use crate::{
    actions::{
        AddSkillFolder, AskDeleteSkill, CancelSkillDelete, ChooseSkillUse, CloseSkill, ConfirmSkillDelete, CopySkill, ImportSkills, NewSkill,
        OpenSkill, RemoveSkillFolder, RevealMemory, SaveSkill, SwitchSkill, SwitchSkillFolder,
    },
    button::{Button, Kind as ButtonKind},
    icon::Icon,
    settings::{field, text_area, text_input},
    settings_kit::{card, dispatch, head, pill, segmented, staged_head},
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    tools_page::switch,
    tooltip::tip,
    typography::{Type, Typography},
};

/// The skill being written or edited, kept while the user looks elsewhere.
pub struct SkillEditor {
    /// Its file; `None` for a new skill.
    pub file: Option<PathBuf>,
    pub name: Entity<TextField>,
    pub description: Entity<TextField>,
    pub hint: Entity<TextField>,
    pub body: Entity<TextField>,
    /// Only when the user types `/name` (`disable-model-invocation`).
    pub manual: bool,
    pub error: Option<SharedString>,
    /// Asking before the skill moves to the Trash.
    pub confirming: bool,
}

impl SkillEditor {
    pub fn new(file: Option<PathBuf>, document: Option<&wack_core::skills::Document>, cx: &mut App) -> Self {
        let name = cx.new(|cx| TextField::new(Type::Mono, "release-notes", cx));
        let description = cx.new(|cx| {
            TextField::new(Type::Ui, "Drafts release notes from the commits since a tag. Use when asked for release notes or a changelog.", cx).multiline(4)
        });
        let hint = cx.new(|cx| TextField::new(Type::Mono, "[since]", cx));
        let body = cx.new(|cx| {
            TextField::new(Type::Mono, "# Release notes\n\nList the commits since the tag with git log, group them by area, and…", cx).multiline(16)
        });
        if let Some(document) = document {
            name.update(cx, |field, cx| field.set_text(&document.name, cx));
            description.update(cx, |field, cx| field.set_text(&document.description, cx));
            hint.update(cx, |field, cx| field.set_text(&document.hint, cx));
            body.update(cx, |field, cx| field.set_text(&document.body, cx));
        }
        Self { file, name, description, hint, body, manual: document.is_some_and(|document| document.manual), error: None, confirming: false }
    }

    pub fn fields(&self) -> [Entity<TextField>; 4] {
        [self.name.clone(), self.description.clone(), self.hint.clone(), self.body.clone()]
    }

    /// The first thing to fill in before the skill can be saved.
    pub fn issue(&self, cx: &App) -> Option<SharedString> {
        let text = |field: &Entity<TextField>| field.read(cx).text().to_string();
        wack_core::skills::validate_name(&text(&self.name))
            .and(wack_core::skills::validate_description(&text(&self.description)))
            .and(wack_core::skills::validate_hint(&text(&self.hint)))
            .err()
            .map(Into::into)
    }

    pub fn view(&self, cx: &App) -> EditorData {
        EditorData {
            new: self.file.is_none(),
            file: self.file.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            hint: self.hint.clone(),
            body: self.body.clone(),
            manual: self.manual,
            issue: self.error.clone().or_else(|| self.issue(cx)),
            error: self.error.is_some(),
            confirming: self.confirming,
        }
    }
}

#[derive(Clone)]
pub struct EditorData {
    pub new: bool,
    pub file: Option<PathBuf>,
    pub name: Entity<TextField>,
    pub description: Entity<TextField>,
    pub hint: Entity<TextField>,
    pub body: Entity<TextField>,
    pub manual: bool,
    pub issue: Option<SharedString>,
    pub error: bool,
    pub confirming: bool,
}

/// A skill from another folder, open read-only.
#[derive(Clone, Debug, PartialEq)]
pub struct Viewing {
    pub entry: SkillEntry,
    /// Its folder's name.
    pub folder: String,
    pub body: String,
}

/// Settings › Skills' state, owned by the app so leaving the page keeps it.
#[derive(Default)]
pub struct SkillsState {
    pub folders: Vec<SkillFolder>,
    pub editor: Option<SkillEditor>,
    pub viewing: Option<Viewing>,
    pub error: Option<SharedString>,
    /// What the last import or copy did.
    pub notice: Option<SharedString>,
}

impl SkillsState {
    pub fn data(&self, cx: &App) -> SkillsData {
        SkillsData {
            folders: self.folders.clone(),
            editor: self.editor.as_ref().map(|editor| editor.view(cx)),
            viewing: self.viewing.clone(),
            error: self.error.clone(),
            notice: self.notice.clone(),
        }
    }

    /// The listed skill whose file is `file`, with its folder.
    pub fn find(&self, file: &std::path::Path) -> Option<(&SkillFolder, &SkillEntry)> {
        self.folders.iter().find_map(|folder| folder.skills.iter().find(|entry| entry.skill.file == file).map(|entry| (folder, entry)))
    }
}

#[derive(Clone, Default)]
pub struct SkillsData {
    pub folders: Vec<SkillFolder>,
    pub editor: Option<EditorData>,
    pub viewing: Option<Viewing>,
    pub error: Option<SharedString>,
    pub notice: Option<SharedString>,
}

/// How skills work, for a library with none yet.
const STEPS: [(&str, &str); 3] = [
    ("Write down how you do something", "A release, a review, a kind of fix. One folder with a SKILL.md, kept in ~/.agents/skills."),
    ("Describe when it applies", "Every chat lists each skill's name and description; the agent loads one when a task matches."),
    ("Or run it yourself", "Type /name in the composer to send the skill with what you write after it."),
];

pub fn page(data: SkillsData, theme: &Theme) -> AnyElement {
    if let Some(editor) = &data.editor {
        return editor_page(editor, theme).into_any_element();
    }
    if let Some(viewing) = &data.viewing {
        return view_page(viewing, theme).into_any_element();
    }
    let new = pill("skill-new", "New skill", Some(Icon::Plus), true, theme).on_click(dispatch(NewSkill)).tag("skill-new");
    let mut page = div().flex().flex_col().gap(px(18.)).child(staged_head(crate::settings::Section::Skills, 
        "Skills",
        "Instructions the agent loads when a task matches its description, or that you run as /name. Changes apply from a chat’s next message.",
        Some(new.into_any_element()),
        theme,
    ));
    if let Some(error) = data.error {
        page = page.child(div().typography(Type::Ui).text_color(theme.danger).child(error));
    }
    if let Some(notice) = data.notice {
        page = page.child(div().typography(Type::Ui).text_color(theme.text_soft).child(notice).tag("skills-notice"));
    }
    let library = data.folders.iter().find(|folder| folder.kind == FolderKind::Library);
    if let Some(library) = library {
        page = page.child(library_card(library, theme));
        if library.skills.is_empty() {
            page = page.child(empty(theme));
        }
    }
    let tools: Vec<&SkillFolder> = data.folders.iter().filter(|folder| folder.kind == FolderKind::Tool).collect();
    if !tools.is_empty() {
        let mut card = card(Some("Other tools’ folders"), None, Some("Off until you switch them on"), theme);
        for (index, folder) in tools.into_iter().enumerate() {
            card = card.child(folder_row(("tool-folder", index), folder, theme));
            if folder.enabled {
                card = folder_skills(card, folder, theme);
            }
        }
        page = page.child(card);
    }
    let added: Vec<&SkillFolder> = data.folders.iter().filter(|folder| folder.kind == FolderKind::Added).collect();
    let add = pill("skill-add-folder", "Add folder", Some(Icon::Folder), false, theme).on_click(dispatch(AddSkillFolder)).tag("skill-add-folder");
    let mut added_card = card(Some("Added folders"), None, Some("Load after the others, in this order"), theme);
    for (index, folder) in added.iter().enumerate() {
        added_card = folder_skills(added_card.child(folder_row(("added-folder", index), folder, theme)), folder, theme);
    }
    added_card = added_card.child(div().px(px(18.)).py(px(12.)).border_t_1().border_color(theme.border).flex().child(add));
    page.child(added_card).into_any_element()
}

fn library_card(folder: &SkillFolder, theme: &Theme) -> impl IntoElement {
    let count = folder.skills.len();
    let header = div()
        .px(px(18.))
        .pt(px(14.))
        .pb(px(12.))
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child("Your skills"))
                .child(div().truncate().typography(Type::Small).text_color(theme.text_faint).child(format!("{} · {}", folder.display, skills_count(count)))),
        )
        .child(pill("skill-import", "Import", Some(Icon::Download), false, theme).on_click(dispatch(ImportSkills)).tag("skill-import"))
        .child(icon_button("skills-reveal", Icon::Folder, "Show the folder in Finder", RevealMemory { path: folder.path.clone() }, theme));
    folder_skills(card(None, None, None, theme).child(header), folder, theme)
}

fn skills_count(count: usize) -> String {
    format!("{count} {}", if count == 1 { "skill" } else { "skills" })
}

/// Another tool's folder or an added one: its name, path, and its switch or remove button.
fn folder_row(id: (&'static str, usize), folder: &SkillFolder, theme: &Theme) -> impl IntoElement {
    let detail = if folder.enabled || folder.kind == FolderKind::Added {
        format!("{} · {}", folder.display, skills_count(folder.skills.len()))
    } else {
        folder.display.clone()
    };
    let control: AnyElement = match folder.kind {
        FolderKind::Added => icon_button(id, Icon::Trash, "Stop loading skills from this folder", RemoveSkillFolder { id: folder.id.clone() }, theme)
            .tag("skill-remove-folder")
            .into_any_element(),
        _ => div()
            .id(id)
            .cursor_pointer()
            .on_click(dispatch(SwitchSkillFolder { id: folder.id.clone(), on: !folder.enabled }))
            .child(switch(folder.enabled, theme))
            .tag("skill-folder-switch")
            .into_any_element(),
    };
    div()
        .px(px(18.))
        .py(px(12.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(12.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child(folder.label.clone()))
                .child(div().truncate().typography(Type::Small).text_color(theme.text_faint).child(detail)),
        )
        .when(folder.path.is_dir(), |row| {
            row.child(icon_button(SharedString::from(format!("{}-reveal", folder.id)), Icon::Search, "Show the folder in Finder", RevealMemory { path: folder.path.clone() }, theme))
        })
        .child(control)
}

/// A folder's skills, then the files in it that could not load. Under another folder's row they
/// sit indented, so they read as its own.
fn folder_skills(mut card: gpui::Div, folder: &SkillFolder, theme: &Theme) -> gpui::Div {
    let inset = if folder.kind == FolderKind::Library { 18. } else { 36. };
    for entry in &folder.skills {
        card = card.child(skill_row(entry, inset, theme));
    }
    for problem in &folder.problems {
        card = card.child(
            div()
                .pl(px(inset))
                .pr(px(18.))
                .py(px(10.))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().truncate().typography(Type::Mono).text_color(theme.text_dim).child(problem.file.display().to_string()))
                .child(div().typography(Type::Small).text_color(theme.warning).child(format!("Not loaded: {}", problem.message))),
        );
    }
    card
}

fn badge(text: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
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

fn skill_row(entry: &SkillEntry, inset: f32, theme: &Theme) -> impl IntoElement {
    let skill = &entry.skill;
    let id = SharedString::from(format!("skill-{}", skill.file.display()));
    let key = file_key(Kind::Skill, &skill.file);
    let enabled = entry.enabled;
    let dimmed = !enabled || entry.shadowed_by.is_some();
    div()
        .id(id.clone())
        .pl(px(inset))
        .pr(px(18.))
        .py(px(11.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(14.))
        .cursor_pointer()
        .hover(|row| row.bg(theme.hover))
        .on_click(dispatch(OpenSkill { file: skill.file.clone() }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .when(dimmed, |text| text.opacity(0.6))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().min_w_0().truncate().typography(Type::UiStrong).text_color(theme.text).child(format!("/{}", skill.name)))
                        .when_some(skill.hint.clone(), |line, hint| {
                            line.child(div().flex_none().typography(Type::Small).italic().text_color(theme.text_faint).child(hint))
                        })
                        .when(skill.manual, |line| line.child(badge("/name only", theme)))
                        .when_some(entry.shadowed_by.clone(), |line, by| {
                            line.child(
                                badge("Not loaded", theme)
                                    .id(SharedString::from(format!("{id}-shadowed")))
                                    .tooltip(tip(format!("{by} has a skill with this name, and it loads first."))),
                            )
                        }),
                )
                .child(div().typography(Type::Small).text_color(theme.text_dim).text_ellipsis().line_clamp(2).child(skill.description.clone())),
        )
        .child(
            div()
                .id(SharedString::from(format!("{id}-switch")))
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(Box::new(SwitchSkill { key: key.clone(), on: !enabled }), cx);
                })
                .child(switch(enabled, theme))
                .tag("skill-switch"),
        )
        .tag("skill-row")
}

fn empty(theme: &Theme) -> impl IntoElement {
    let mut steps = card(Some("No skills of your own yet"), None, None, theme);
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
fn icon_button(id: impl Into<gpui::ElementId>, icon: Icon, words: &'static str, action: impl gpui::Action, theme: &Theme) -> gpui::Stateful<gpui::Div> {
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
        .child(icon.view(px(14.), theme.text_dim))
}

fn back_link(theme: &Theme) -> impl IntoElement {
    div()
        .id("back-skills")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .pb(px(18.))
        .cursor_pointer()
        .typography(Type::Ui)
        .text_color(theme.text_dim)
        .hover(|link| link.text_color(theme.text))
        .on_click(dispatch(CloseSkill))
        .child(Icon::ChevronRight.view(px(13.), theme.text_faint).with_transformation(gpui::Transformation::rotate(gpui::radians(std::f32::consts::PI))))
        .child("All skills")
        .tag("back-skills")
}

/// A box for long text: the editor's instructions, a viewed skill's body.
fn text_box(content: impl IntoElement, theme: &Theme) -> gpui::Div {
    div().w_full().min_h(px(160.)).px(px(11.)).py(px(9.)).rounded(px(9.)).bg(theme.window).border_1().border_color(theme.border).child(content)
}

fn editor_page(editor: &EditorData, theme: &Theme) -> impl IntoElement {
    let section = |content: AnyElement| div().px(px(18.)).py(px(13.)).border_t_1().border_color(theme.border).child(content);
    let uses = [(false, "The agent decides"), (true, "Only when I type /name")];
    let status: AnyElement = match &editor.issue {
        Some(issue) => div()
            .flex_1()
            .typography(Type::Small)
            .text_color(if editor.error { theme.danger } else { theme.text_dim })
            .child(issue.clone())
            .into_any_element(),
        None => div().flex_1().into_any_element(),
    };
    let mut bar = div().px(px(18.)).py(px(12.)).border_t_1().border_color(theme.border).flex().items_center().gap(px(8.)).child(status);
    if !editor.new {
        bar = bar.child(Button::new("skill-delete", "Delete", ButtonKind::Secondary).action(AskDeleteSkill).tag("skill-delete"));
    }
    bar = bar.child(Button::new("skill-cancel", "Cancel", ButtonKind::Secondary).action(CloseSkill).tag("skill-cancel")).child(
        Button::new("skill-save", if editor.new { "Create skill" } else { "Save" }, ButtonKind::Primary)
            .action(SaveSkill)
            .enabled(editor.issue.is_none() || editor.error)
            .tag("skill-save"),
    );
    let shown = editor.file.as_ref().map(|file| {
        let path = file.display().to_string();
        let home = std::env::var("HOME").unwrap_or_default();
        SharedString::from(match path.strip_prefix(&home) {
            Some(rest) if !home.is_empty() => format!("~{rest}"),
            _ => path,
        })
    });
    let mut form = card(Some("Your skills"), shown, None, theme)
        .child(section(
            field("Name", Some("What you type after /. Lowercase words joined by hyphens; it names the skill's folder.".into()), text_input(editor.name.clone(), "skill-name", theme), theme)
                .into_any_element(),
        ))
        .child(section(
            field(
                "Description",
                Some("What it does and when to use it. Every chat lists this, and the agent decides from it alone.".into()),
                text_area(editor.description.clone(), 64., theme).tag("skill-description"),
                theme,
            )
            .into_any_element(),
        ))
        .child(section(
            field(
                "When it’s used",
                Some(if editor.manual { "Left out of the agent's list; it runs only as /name." } else { "Listed for the agent, and you can still run it as /name." }.into()),
                segmented("skill-use", &uses, editor.manual, |manual| Box::new(ChooseSkillUse { manual }), theme),
                theme,
            )
            .into_any_element(),
        ))
        .child(section(
            field("Argument hint", Some("Optional. What /name takes after it: <needed> or [optional].".into()), text_input(editor.hint.clone(), "skill-hint", theme), theme)
                .into_any_element(),
        ))
        .child(section(
            field("Instructions", Some("Markdown. Relative paths in it are read from the skill's folder.".into()), text_area(editor.body.clone(), 160., theme).tag("skill-body"), theme)
                .into_any_element(),
        ));
    if editor.confirming {
        form = form.child(confirm_box(theme));
    }
    div()
        .flex()
        .flex_col()
        .child(back_link(theme))
        .child(head(
            if editor.new { "New skill" } else { "Edit skill" },
            "A folder with a SKILL.md in ~/.agents/skills, which Claude Code, Codex and other tools can read too.",
            None,
            theme,
        ))
        .child(form.child(bar))
}

fn confirm_box(theme: &Theme) -> impl IntoElement {
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
                .child(div().typography(Type::UiStrong).text_color(theme.text).child("Delete this skill?"))
                .child(div().typography(Type::Ui).text_color(theme.text_dim).child("Its folder moves to the Trash, so chats stop offering it. You can put it back from the Trash.")),
        )
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(Button::new("skill-keep", "Keep", ButtonKind::Secondary).action(CancelSkillDelete).tag("skill-keep"))
                .child(Button::new("skill-trash", "Move to Trash", ButtonKind::Danger).action(ConfirmSkillDelete).tag("skill-trash")),
        )
}

fn view_page(viewing: &Viewing, theme: &Theme) -> impl IntoElement {
    let skill: &Skill = &viewing.entry.skill;
    let row = |label: &'static str, value: AnyElement| {
        div()
            .px(px(18.))
            .py(px(11.))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .items_start()
            .gap(px(14.))
            .child(div().w(px(120.)).flex_none().typography(Type::Ui).text_color(theme.text_soft).child(label))
            .child(div().flex_1().min_w_0().child(value))
    };
    let words = |text: String| div().typography(Type::Ui).text_color(theme.text).child(text).into_any_element();
    let mut card = card(Some("Skill"), Some(viewing.folder.clone().into()), None, theme)
        .child(row("Name", words(format!("/{}", skill.name))))
        .child(row("Description", words(skill.description.clone())))
        .child(row("When it’s used", words(if skill.manual { "Only when you type /name".into() } else { "The agent decides".into() })));
    if let Some(hint) = &skill.hint {
        card = card.child(row("Argument hint", words(hint.clone())));
    }
    if let Some(by) = &viewing.entry.shadowed_by {
        card = card.child(row("Not loaded", words(format!("{by} has a skill named {}, and it loads first.", skill.name))));
    }
    card = card.child(row("File", div().typography(Type::Mono).text_color(theme.text_dim).child(skill.file.display().to_string()).into_any_element()));
    let actions = div()
        .px(px(18.))
        .py(px(12.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().flex_1())
        .child(Button::new("skill-reveal", "Show in Finder", ButtonKind::Secondary).action(RevealMemory { path: skill.file.clone() }))
        .child(Button::new("skill-copy", "Copy to Your skills", ButtonKind::Primary).action(CopySkill { file: skill.file.clone() }).tag("skill-copy"));
    div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(div().flex().flex_col().child(back_link(theme)).child(head(
            "Skill",
            "From another folder, so WackCode only reads it. Copy it to Your skills to change it there.",
            None,
            theme,
        )))
        .child(card.child(actions))
        .child(
            crate::settings_kit::card(Some("Instructions"), None, None, theme).child(
                div()
                    .px(px(18.))
                    .py(px(13.))
                    .border_t_1()
                    .border_color(theme.border)
                    .child(text_box(div().typography(Type::Mono).text_color(theme.text_soft).child(viewing.body.clone()), theme)),
            ),
        )
}
