//! The Build and Run split pills and their anchored command configuration popover.
//!
//! Each control splits into an action trigger (`Build` / `Run`, or `Stop` while active) and a
//! chevron opening the command configuration popover directly underneath it. Both live cleanly on
//! `tab_strip` (Chat screens) and inside the `NewChat` hero block.

use gpui::{
    AnchoredPositionMode, Anchor, Entity, Hsla, MouseButton, Pixels, SharedString, Size, Stateful, anchored,
    deferred, div, point, prelude::*, px,
};
use wack_core::{CommandKind, CommandStatus};

use crate::{
    actions::{CloseCommandEditor, OpenCommandEditor, SaveCommandEditor, ShowCommandOutput, TriggerProjectCommand},
    button::{Button, Kind as ButtonKind},
    icon::Icon,
    motion,
    settings::text_input,
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
};

const POPOVER_WIDTH: f32 = 360.;
const POPOVER_GAP: f32 = 6.;

/// View data for a single `Build` or `Run` split pill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandButtonData {
    pub kind: CommandKind,
    pub command: Option<String>,
    pub status: CommandStatus,
    pub editor_open: bool,
}

impl CommandButtonData {
    pub fn idle(kind: CommandKind, command: Option<&str>) -> Self {
        Self {
            kind,
            command: command.map(Into::into),
            status: CommandStatus::Idle,
            editor_open: false,
        }
    }
}

/// View data for the open command configuration popover anchored below its button.
#[derive(Clone)]
pub struct CommandEditorData {
    pub kind: CommandKind,
    pub project_name: SharedString,
    pub project_path: SharedString,
    pub input: Entity<TextField>,
    pub has_session: bool,
    pub active: bool,
    pub can_run: bool,
    pub error: Option<SharedString>,
}

/// Renders one `Build` or `Run` split pill, plus its anchored popover when `editor` targets it.
/// Pass `ground` on hero backdrop screens (`NewChat`) so idle pills keep frosted glass contrast.
pub fn split_button(
    data: &CommandButtonData,
    editor: Option<&CommandEditorData>,
    compact: bool,
    ground: Option<Hsla>,
    viewport: Size<Pixels>,
    theme: &Theme,
) -> impl IntoElement {
    let kind = data.kind;
    let configured = data.command.is_some();
    let active = data.status.is_active();
    let stopping = data.status == CommandStatus::Stopping;

    let (main_tag, config_tag) = match kind {
        CommandKind::Build => ("build-button", "build-config"),
        CommandKind::Run => ("run-button", "run-config"),
    };
    let main_id = SharedString::from(main_tag);
    let config_id = SharedString::from(config_tag);

    let (icon, label, main_ink) = if active {
        let ink = if stopping { theme.text_dim } else { theme.danger };
        let label = if stopping { "Stopping…" } else { "Stop" };
        (Icon::Stop, label, ink)
    } else {
        let icon = match kind {
            CommandKind::Build => Icon::Hammer,
            CommandKind::Run => Icon::Play,
        };
        let ink = if configured { theme.text_soft } else { theme.text_dim };
        (icon, kind.title(), ink)
    };

    let main_tip: SharedString = match (&data.status, &data.command) {
        (CommandStatus::Stopping, _) => format!("Stopping {}", kind.name()).into(),
        (CommandStatus::Running, Some(cmd)) => format!("Stop {} · {cmd}", kind.name()).into(),
        (CommandStatus::Running, None) => format!("Stop {}", kind.name()).into(),
        (_, Some(cmd)) => format!("{} · {cmd}", kind.title()).into(),
        (_, None) => format!("Configure {} command", kind.name()).into(),
    };
    let config_tip: SharedString = format!("Configure {} command", kind.name()).into();

    let (shell_bg, shell_border) = if active {
        (Some(Hsla { a: 0.14, ..theme.danger }), Hsla { a: 0.34, ..theme.danger })
    } else if data.editor_open {
        (Some(theme.hover), theme.border_strong)
    } else {
        (ground, theme.border)
    };

    let main_part = div()
        .id(main_id)
        .h_full()
        .pl(px(8.))
        .pr(px(if compact { 6. } else { 7. }))
        .rounded_l_full()
        .flex()
        .items_center()
        .gap(px(5.))
        .when(!stopping, |button| {
            button
                .cursor_pointer()
                .hover(|button| {
                    button.bg(if active {
                        Hsla { a: 0.22, ..theme.danger }
                    } else {
                        theme.hover
                    })
                })
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(TriggerProjectCommand { kind }), cx);
                })
        })
        .when(stopping, |button| button.opacity(0.6))
        .tooltip(tip(main_tip))
        .child(icon.view(px(11.), main_ink))
        .when(!compact, |button| {
            button.child(
                div()
                    .typography(if active || configured { Type::Ui } else { Type::Small })
                    .text_color(main_ink)
                    .child(label),
            )
        })
        .tag(main_tag);

    let config_part = div()
        .id(config_id)
        .h_full()
        .pl(px(4.))
        .pr(px(7.))
        .rounded_r_full()
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .when(data.editor_open, |chevron| chevron.bg(theme.hover))
        .hover(|chevron| chevron.bg(theme.hover))
        .tooltip(tip(config_tip))
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(OpenCommandEditor { kind }), cx);
        })
        .child(Icon::ChevronDown.view(px(10.), if data.editor_open { theme.text } else { theme.text_dim }))
        .tag(config_tag);

    let pill = div()
        .occlude()
        .flex_none()
        .h(px(22.))
        .rounded_full()
        .border_1()
        .border_color(shell_border)
        .when_some(shell_bg, |pill, bg| pill.bg(bg))
        .flex()
        .items_center()
        .child(main_part)
        .child(div().flex_none().w(px(1.)).h(px(12.)).bg(shell_border))
        .child(config_part);

    let open_editor = editor.filter(|editor| editor.kind == kind);
    div().relative().flex_none().child(pill).when_some(open_editor, |host, editor| {
        let backdrop = anchored()
            .position_mode(AnchoredPositionMode::Window)
            .position(point(px(0.), px(0.)))
            .child(
                div()
                    .id("command-editor-backdrop")
                    .w(viewport.width)
                    .h(viewport.height)
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(CloseCommandEditor), cx);
                    }),
            );
        let popover = div().absolute().bottom_0().right_0().child(
            anchored()
                .anchor(Anchor::TopRight)
                .offset(point(px(0.), px(POPOVER_GAP)))
                .snap_to_window_with_margin(px(8.))
                .child(motion::enter(
                    gpui::ElementId::Name(format!("arrive-{}-editor", kind.name()).into()),
                    editor_card(editor, theme),
                )),
        );
        host.child(deferred(backdrop).with_priority(1))
            .child(deferred(popover).with_priority(2))
    })
}

fn editor_card(editor: &CommandEditorData, theme: &Theme) -> Stateful<gpui::Div> {
    let kind = editor.kind;
    let title = match kind {
        CommandKind::Build => "Build command",
        CommandKind::Run => "Run command",
    };
    let run_label = match kind {
        CommandKind::Build => "Save & Build",
        CommandKind::Run => "Save & Run",
    };
    let helper = if editor.active {
        format!(
            "Runs inside this folder via your login shell. Changes apply on the next {}.",
            kind.name()
        )
    } else {
        "Runs inside this folder via your login shell. Leave blank to clear.".to_string()
    };

    div()
        .id("command-editor-card")
        .occlude()
        .w(px(POPOVER_WIDTH))
        .p(px(14.))
        .rounded(px(12.))
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border_strong)
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .flex()
                .items_baseline()
                .justify_between()
                .gap(px(8.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child(title))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .typography(Type::Small)
                        .text_color(theme.text_faint)
                        .child(editor.project_name.clone()),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(div().typography(Type::Small).text_color(theme.text_soft).child("Command"))
                .child(text_input(editor.input.clone(), "command-input", theme)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(theme.text_dim)
                .child(Icon::Folder.view(px(12.), theme.text_faint))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .typography(Type::Mono)
                        .text_color(theme.text_dim)
                        .child(editor.project_path.clone()),
                ),
        )
        .child(div().typography(Type::Small).text_color(theme.text_faint).child(helper))
        .when_some(editor.error.clone(), |card, error| {
            card.child(div().typography(Type::Small).text_color(theme.danger).child(error))
        })
        .child(
            div()
                .pt(px(2.))
                .flex()
                .items_center()
                .gap(px(8.))
                .when(editor.has_session, |row| {
                    row.child(
                        div()
                            .id("command-show-output")
                            .cursor_pointer()
                            .h(px(28.))
                            .px(px(8.))
                            .rounded(px(8.))
                            .flex()
                            .items_center()
                            .typography(Type::Small)
                            .text_color(theme.accent_text)
                            .hover(|button| button.bg(theme.hover))
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(Box::new(ShowCommandOutput { kind }), cx);
                            })
                            .child("Show output")
                            .tag("command-show-output"),
                    )
                })
                .child(div().flex_1())
                .child(
                    Button::new("command-save", "Save", ButtonKind::Secondary)
                        .action(SaveCommandEditor { run_after: false })
                        .tag("command-save"),
                )
                .when(!editor.active && editor.can_run, |row| {
                    row.child(
                        Button::new("command-save-run", run_label, ButtonKind::Primary)
                            .action(SaveCommandEditor { run_after: true })
                            .tag("command-save-run"),
                    )
                }),
        )
}
