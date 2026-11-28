//! Settings › Commands: the slash commands, after version 2's page (less package commands,
//! which version 3 cut).
//!
//! - **WackCode's** (`/compact`, `/init`, `/new`, `/name`, `/copy`), each with a switch. A
//!   switched-off one leaves the `/` menu but keeps its name, so nothing else takes it.
//! - **Yours**: Markdown files in WackCode's data folder, the file name being the command. Each
//!   opens in an editor whose preview expands the prompt with sample arguments as you type,
//!   by the same rules sending it uses (`$1`, `$ARGUMENTS`, `${2:-default}`, `${@:2}`).
//! - Skills answer to `/name` too; they are switched on and off in Settings › Skills.
//!
//! The list is read from disk when the section opens and after every change; the editor's fields
//! live in [`CommandEditor`], owned by the app state.

use std::path::PathBuf;

use gpui::{AnyElement, App, AppContext, Entity, SharedString, div, prelude::*, px};
use wack_core::skills::{CommandEntry, Kind, expand_template};

use crate::{
    actions::{
        AskDeleteSlashCommand, CancelSlashCommandDelete, CloseSlashCommand, ConfirmSlashCommandDelete, NewSlashCommand, OpenSettingsSection,
        OpenSlashCommand, RevealMemory, SaveSlashCommand, SwitchSlashCommand,
    },
    button::{Button, Kind as ButtonKind},
    icon::Icon,
    settings::{Section, field, text_area, text_input},
    settings_kit::{card, dispatch, head, pill, staged_head},
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    tools_page::switch,
    tooltip::tip,
    typography::{Type, Typography},
};

/// The command being written or edited, kept while the user looks elsewhere.
pub struct CommandEditor {
    /// Its name before this edit; `None` for a new command.
    pub previous: Option<String>,
    pub name: Entity<TextField>,
    pub description: Entity<TextField>,
    pub hint: Entity<TextField>,
    pub body: Entity<TextField>,
    /// Arguments to try the prompt with, for the preview.
    pub sample: Entity<TextField>,
    pub error: Option<SharedString>,
    pub confirming: bool,
}

impl CommandEditor {
    pub fn new(template: Option<&wack_core::skills::Template>, cx: &mut App) -> Self {
        let name = cx.new(|cx| TextField::new(Type::Mono, "review", cx));
        let description = cx.new(|cx| TextField::new(Type::Ui, "Review a file the way the team does", cx));
        let hint = cx.new(|cx| TextField::new(Type::Mono, "<file>", cx));
        let body = cx.new(|cx| TextField::new(Type::Mono, "Review $1 for correctness first, then naming, then tests. Say what you would change before changing anything.", cx).multiline(12));
        let sample = cx.new(|cx| TextField::new(Type::Mono, "src/main.rs", cx));
        if let Some(template) = template {
            name.update(cx, |field, cx| field.set_text(&template.name, cx));
            description.update(cx, |field, cx| field.set_text(&template.description, cx));
            hint.update(cx, |field, cx| field.set_text(template.hint.as_deref().unwrap_or(""), cx));
            body.update(cx, |field, cx| field.set_text(&template.body, cx));
        }
        Self { previous: template.map(|template| template.name.clone()), name, description, hint, body, sample, error: None, confirming: false }
    }

    pub fn fields(&self) -> [Entity<TextField>; 5] {
        [self.name.clone(), self.description.clone(), self.hint.clone(), self.body.clone(), self.sample.clone()]
    }

    pub fn issue(&self, cx: &App) -> Option<SharedString> {
        let name = self.name.read(cx).text().trim().trim_start_matches('/').to_string();
        if name.is_empty() {
            return Some("Give the command a name.".into());
        }
        if !wack_core::skills::valid_command_name(&name) {
            return Some("Use lowercase letters, numbers and single hyphens in a command's name.".into());
        }
        if self.body.read(cx).text().trim().is_empty() {
            return Some("Write the prompt the command sends.".into());
        }
        None
    }

    pub fn view(&self, cx: &App) -> EditorData {
        let body = self.body.read(cx).text().to_string();
        let sample = self.sample.read(cx).text().to_string();
        EditorData {
            new: self.previous.is_none(),
            name: self.name.clone(),
            description: self.description.clone(),
            hint: self.hint.clone(),
            body: self.body.clone(),
            sample: self.sample.clone(),
            preview: expand_template(&body, &sample).trim().to_string().into(),
            issue: self.error.clone().or_else(|| self.issue(cx)),
            error: self.error.is_some(),
            confirming: self.confirming,
        }
    }
}

#[derive(Clone)]
pub struct EditorData {
    pub new: bool,
    pub name: Entity<TextField>,
    pub description: Entity<TextField>,
    pub hint: Entity<TextField>,
    pub body: Entity<TextField>,
    pub sample: Entity<TextField>,
    /// The prompt as sending it with `sample` would expand it.
    pub preview: SharedString,
    pub issue: Option<SharedString>,
    pub error: bool,
    pub confirming: bool,
}

/// Settings › Commands' state, owned by the app so leaving the page keeps it.
#[derive(Default)]
pub struct CommandsState {
    pub entries: Vec<CommandEntry>,
    /// Where the user's commands live.
    pub folder: PathBuf,
    pub editor: Option<CommandEditor>,
    pub error: Option<SharedString>,
}

impl CommandsState {
    pub fn data(&self, cx: &App) -> CommandsData {
        CommandsData {
            entries: self.entries.clone(),
            folder: self.folder.clone(),
            editor: self.editor.as_ref().map(|editor| editor.view(cx)),
            error: self.error.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub struct CommandsData {
    pub entries: Vec<CommandEntry>,
    pub folder: PathBuf,
    pub editor: Option<EditorData>,
    pub error: Option<SharedString>,
}

pub fn page(data: CommandsData, theme: &Theme) -> AnyElement {
    if let Some(editor) = &data.editor {
        return editor_page(editor, theme).into_any_element();
    }
    let new = pill("command-new", "New command", Some(Icon::Plus), true, theme).cursor_pointer().on_click(dispatch(NewSlashCommand)).tag("command-new");
    let mut page = div().flex().flex_col().gap(px(18.)).child(staged_head(crate::settings::Section::Commands, 
        "Commands",
        "What you can type after / in the composer. Your own commands send a prompt you wrote, with what you type after the name filled in.",
        Some(new.into_any_element()),
        theme,
    ));
    if let Some(error) = data.error {
        page = page.child(div().typography(Type::Ui).text_color(theme.danger).child(error));
    }
    let (builtins, yours): (Vec<&CommandEntry>, Vec<&CommandEntry>) = data.entries.iter().partition(|entry| entry.command.kind == Kind::Builtin);
    let mut own = card(Some("Your commands"), Some(yours.len().to_string().into()), None, theme);
    if yours.is_empty() {
        own = own.child(
            div()
                .px(px(18.))
                .py(px(14.))
                .border_t_1()
                .border_color(theme.border)
                .typography(Type::Ui)
                .text_color(theme.text_dim)
                .child("None yet. A command is a prompt you send often: write it once, then type /name and what changes each time."),
        );
    }
    for entry in &yours {
        own = own.child(command_row(entry, true, theme));
    }
    own = own.child(
        div()
            .px(px(18.))
            .py(px(10.))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().truncate().typography(Type::Small).text_color(theme.text_faint).child(shown(&data.folder)))
            .child(
                div()
                    .id("commands-reveal")
                    .flex_none()
                    .size(px(28.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|button| button.bg(theme.hover))
                    .tooltip(tip("Show the folder in Finder"))
                    .on_click(dispatch(RevealMemory { path: data.folder.clone() }))
                    .child(Icon::Folder.view(px(14.), theme.text_dim)),
            ),
    );
    let mut wackcode = card(Some("WackCode’s"), None, Some("Off takes one out of the / menu"), theme);
    for entry in &builtins {
        wackcode = wackcode.child(command_row(entry, false, theme));
    }
    let skills = div()
        .id("commands-skills-link")
        .flex()
        .items_center()
        .gap(px(6.))
        .cursor_pointer()
        .typography(Type::Ui)
        .text_color(theme.text_dim)
        .hover(|link| link.text_color(theme.text))
        .on_click(dispatch(OpenSettingsSection { section: Section::Skills }))
        .child(Icon::Book.view(px(13.), theme.text_faint))
        .child("Skills answer to /name too. Switch them on and off in Settings › Skills.")
        .tag("commands-skills-link");
    page.child(own).child(wackcode).child(skills).into_any_element()
}

fn shown(path: &std::path::Path) -> String {
    let path = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path,
    }
}

fn command_row(entry: &CommandEntry, opens: bool, theme: &Theme) -> impl IntoElement {
    let command = &entry.command;
    let id = SharedString::from(format!("command-{}", command.key));
    let key = command.key.clone();
    let enabled = entry.enabled;
    let name = command.name.clone();
    div()
        .id(id.clone())
        .px(px(18.))
        .py(px(11.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(14.))
        .when(opens, |row| row.cursor_pointer().hover(|row| row.bg(theme.hover)).on_click(dispatch(OpenSlashCommand { name: name.clone() })))
        .child(crate::slash_menu::icon(command.kind).view(px(14.), theme.text_dim))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .when(!enabled, |text| text.opacity(0.6))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().min_w_0().truncate().typography(Type::UiStrong).text_color(theme.text).child(format!("/{}", command.name)))
                        .when_some(command.hint.clone(), |line, hint| {
                            line.child(div().flex_none().typography(Type::Small).italic().text_color(theme.text_faint).child(hint))
                        }),
                )
                .child(div().truncate().typography(Type::Small).text_color(theme.text_dim).child(command.description.clone())),
        )
        .child(
            div()
                .id(SharedString::from(format!("{id}-switch")))
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(Box::new(SwitchSlashCommand { key: key.clone(), on: !enabled }), cx);
                })
                .child(switch(enabled, theme))
                .tag("command-switch"),
        )
        .tag("command-row")
}

fn editor_page(editor: &EditorData, theme: &Theme) -> impl IntoElement {
    let section = |content: AnyElement| div().px(px(18.)).py(px(13.)).border_t_1().border_color(theme.border).child(content);
    let back = div()
        .id("back-commands")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .pb(px(18.))
        .cursor_pointer()
        .typography(Type::Ui)
        .text_color(theme.text_dim)
        .hover(|link| link.text_color(theme.text))
        .on_click(dispatch(CloseSlashCommand))
        .child(Icon::ChevronRight.view(px(13.), theme.text_faint).with_transformation(gpui::Transformation::rotate(gpui::radians(std::f32::consts::PI))))
        .child("All commands")
        .tag("back-commands");
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
        bar = bar.child(Button::new("command-delete", "Delete", ButtonKind::Secondary).action(AskDeleteSlashCommand).tag("command-delete"));
    }
    bar = bar.child(Button::new("command-cancel", "Cancel", ButtonKind::Secondary).action(CloseSlashCommand).tag("command-cancel")).child(
        Button::new("command-save", if editor.new { "Create command" } else { "Save" }, ButtonKind::Primary)
            .action(SaveSlashCommand)
            .enabled(editor.issue.is_none() || editor.error)
            .tag("command-save"),
    );
    let mut form = card(Some("Your commands"), None, None, theme)
        .child(section(field("Name", Some("What you type after /. Lowercase words joined by hyphens.".into()), text_input(editor.name.clone(), "command-name", theme), theme).into_any_element()))
        .child(section(
            field("Description", Some("Optional. Shown in the / menu; without one, the prompt's first line is.".into()), text_input(editor.description.clone(), "command-description", theme), theme)
                .into_any_element(),
        ))
        .child(section(
            field("Argument hint", Some("Optional. What the command takes after its name: <needed> or [optional].".into()), text_input(editor.hint.clone(), "command-hint", theme), theme)
                .into_any_element(),
        ))
        .child(section(
            field(
                "Prompt",
                Some("$1, $2… are the words typed after the name, $ARGUMENTS all of them; ${1:-default} fills an empty one; quotes keep words together.".into()),
                text_area(editor.body.clone(), 120., theme).tag("command-body"),
                theme,
            )
            .into_any_element(),
        ))
        .child(section(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(field("Try it with", None, text_input(editor.sample.clone(), "command-sample", theme), theme))
                .child(
                    div()
                        .w_full()
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(9.))
                        .bg(theme.hover)
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(div().typography(Type::Label).text_color(theme.text_dim).child("Sends"))
                        .child(div().typography(Type::Mono).text_color(theme.text_soft).child(editor.preview.clone()))
                        .tag("command-preview"),
                )
                .into_any_element(),
        ));
    if editor.confirming {
        form = form.child(
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
                .child(div().typography(Type::UiStrong).text_color(theme.text).child("Delete this command?"))
                .child(div().typography(Type::Ui).text_color(theme.text_dim).child("Its file moves to the Trash. You can put it back from the Trash."))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(Button::new("command-keep", "Keep", ButtonKind::Secondary).action(CancelSlashCommandDelete))
                        .child(Button::new("command-trash", "Move to Trash", ButtonKind::Danger).action(ConfirmSlashCommandDelete).tag("command-trash")),
                ),
        );
    }
    div()
        .flex()
        .flex_col()
        .child(back)
        .child(head(if editor.new { "New command" } else { "Edit command" }, "A prompt you send often, run by typing /name and what changes each time.", None, theme))
        .child(form.child(bar))
}
