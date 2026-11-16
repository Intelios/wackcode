//! The Build and Run output panes: live command output over the shared terminal grid.
//!
//! Unlike [`super::TerminalPane`], a [`RunPane`] does not own the process it displays. Sessions live
//! on [`Runner`] keyed by `(canonical_cwd, CommandKind)` so multiple chats in the same project see
//! the same live output and closing a pane never terminates a build or dev server.

use std::{path::PathBuf, time::Duration};

use gpui::{
    Animation, AnimationExt, App, Bounds, ClipboardItem, Context, Entity, FocusHandle, Hsla, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, ScrollWheelEvent, SharedString, Window, canvas, div, prelude::*, px,
};
use wack_core::{
    CommandKind, CommandSession, CommandStatus, Runner,
    terminal::SelectionKind,
};

use crate::{
    actions::{ClearCommandOutput, OpenCommandEditor, StopCommand, TerminalBackTab, TerminalEscape, TriggerProjectCommand},
    button::{Button, Kind as ButtonKind},
    icon::Icon,
    panes::grid::{self, Grid, Prepared, TERMINAL_CONTEXT},
    tags::Tag,
    text_field,
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
};

const PULSE: Duration = Duration::from_millis(1400);

pub struct RunPane {
    pub kind: CommandKind,
    pub folder: PathBuf,
    configured_command: Option<String>,
    session: Option<CommandSession>,
    pub focus: FocusHandle,
    grid: Option<Grid>,
    scroll_remainder: f32,
    selecting: bool,
}

impl RunPane {
    pub fn new(
        kind: CommandKind,
        folder: PathBuf,
        runner: &Runner,
        configured_command: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let session = runner.session(&folder, kind);
        let _ = cx;
        Self {
            kind,
            folder,
            configured_command,
            session,
            focus: cx.focus_handle(),
            grid: None,
            scroll_remainder: 0.,
            selecting: false,
        }
    }

    /// A fixed command pane for gallery scenes.
    pub fn fixture(
        kind: CommandKind,
        folder: impl Into<PathBuf>,
        configured_command: Option<&str>,
        session: Option<CommandSession>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            kind,
            folder: folder.into(),
            configured_command: configured_command.map(Into::into),
            session,
            focus: cx.focus_handle(),
            grid: None,
            scroll_remainder: 0.,
            selecting: false,
        }
    }

    pub fn status(&self) -> CommandStatus {
        self.session
            .as_ref()
            .map(|session| session.info().status)
            .unwrap_or(CommandStatus::Idle)
    }

    /// Syncs the pane with the latest session and configured command for its project folder.
    pub fn sync(&mut self, session: Option<CommandSession>, configured_command: Option<String>, cx: &mut Context<Self>) {
        self.session = session;
        self.configured_command = configured_command;
        if let (Some(session), Some(grid)) = (&self.session, self.grid)
            && grid.columns > 0
            && grid.rows > 0
        {
            session.resize(grid.columns, grid.rows, grid.cell_px());
        }
        cx.notify();
    }

    pub fn set_configured_command(&mut self, configured_command: Option<String>, cx: &mut Context<Self>) {
        self.configured_command = configured_command;
        cx.notify();
    }

    pub fn wake(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    fn fit(&mut self, columns: usize, rows: usize, cell: (u16, u16)) {
        if columns == 0 || rows == 0 {
            return;
        }
        if let Some(session) = &self.session {
            session.resize(columns, rows, cell);
        }
    }

    fn accepts_input(&self) -> bool {
        self.status() == CommandStatus::Running
    }

    fn write(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if !self.accepts_input() {
            return;
        }
        if let Some(session) = &self.session {
            session.clear_selection();
            session.write(&bytes);
            cx.notify();
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.session.as_ref().and_then(CommandSession::selected_text) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if !self.accepts_input() {
            return;
        }
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        if let (Some(session), Some(text)) = (&self.session, text) {
            session.clear_selection();
            session.paste(&text);
            cx.notify();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.platform {
            match keystroke.key.as_str() {
                "c" => self.copy(cx),
                "v" => self.paste(cx),
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        if !self.accepts_input() {
            return;
        }
        let Some(app_cursor) = self.session.as_ref().map(|session| session.modes().app_cursor) else {
            return;
        };
        if let Some(bytes) = grid::encode(keystroke, app_cursor) {
            self.write(bytes, cx);
            cx.stop_propagation();
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (Some(session), Some(grid)) = (&self.session, self.grid) else {
            return;
        };
        let (row, column, right_half) = grid.cell_at(event.position);
        if event.modifiers.shift && event.click_count == 1 {
            session.select_to(row, column, right_half);
        } else {
            let kind = match event.click_count {
                2 => SelectionKind::Words,
                3.. => SelectionKind::Lines,
                _ => SelectionKind::Characters,
            };
            session.select_from(row, column, right_half, kind);
        }
        self.selecting = true;
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            self.selecting = false;
            return;
        }
        if let (Some(session), Some(grid)) = (&self.session, self.grid) {
            let (row, column, right_half) = grid.cell_at(event.position);
            session.select_to(row, column, right_half);
            cx.notify();
        }
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(grid), Some(session)) = (self.grid, &self.session) else {
            return;
        };
        let lines = grid::scroll_lines(grid, event, &mut self.scroll_remainder);
        if lines == 0 {
            return;
        }
        let modes = session.modes();
        if modes.alt_screen && self.accepts_input() {
            let arrow = match (lines > 0, modes.app_cursor) {
                (true, true) => "\x1bOA",
                (true, false) => "\x1b[A",
                (false, true) => "\x1bOB",
                (false, false) => "\x1b[B",
            };
            session.write(arrow.repeat(lines.unsigned_abs() as usize).as_bytes());
        } else {
            session.scroll(lines);
        }
        cx.notify();
    }
}

impl Render for RunPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let focused = self.focus.is_focused(window);
        let pane = cx.entity();
        let kind = self.kind;
        let info = self.session.as_ref().map(CommandSession::info);
        let status = info.as_ref().map(|info| info.status.clone()).unwrap_or(CommandStatus::Idle);
        let active = status.is_active();
        let stopping = status == CommandStatus::Stopping;
        let has_output = self.session.as_ref().is_some_and(CommandSession::has_output);

        let display_command: SharedString = info
            .as_ref()
            .map(|info| info.command.clone())
            .or_else(|| self.configured_command.clone())
            .map(Into::into)
            .unwrap_or_else(|| "No command configured".into());
        let command_ink = if info.is_some() || self.configured_command.is_some() {
            theme.text_soft
        } else {
            theme.text_faint
        };

        let clear_tag = match kind {
            CommandKind::Build => "clear-build-output",
            CommandKind::Run => "clear-run-output",
        };
        let stop_tag = match kind {
            CommandKind::Build => "stop-build-pane",
            CommandKind::Run => "stop-run-pane",
        };

        let meta_bar = div()
            .flex_none()
            .h(px(32.))
            .px(px(12.))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .typography(Type::Mono)
                    .text_color(command_ink)
                    .child(display_command),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(has_output, |right| {
                        right.child(
                            div()
                                .id(("clear-output", kind as usize))
                                .cursor_pointer()
                                .size(px(22.))
                                .rounded(px(6.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(|button| button.bg(theme.hover))
                                .tooltip(tip("Clear output"))
                                .on_click(move |_, window, cx| {
                                    window.dispatch_action(Box::new(ClearCommandOutput { kind }), cx);
                                })
                                .child(Icon::Erase.view(px(13.), theme.text_dim))
                                .tag(clear_tag),
                        )
                    })
                    .when(active, |right| {
                        right.child(
                            div()
                                .id(("stop-command-pane", kind as usize))
                                .h(px(22.))
                                .px(px(8.))
                                .rounded_full()
                                .bg(Hsla { a: 0.14, ..theme.danger })
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .when(!stopping, |button| {
                                    button
                                        .cursor_pointer()
                                        .hover(|button| button.bg(Hsla { a: 0.22, ..theme.danger }))
                                        .on_click(move |_, window, cx| {
                                            window.dispatch_action(Box::new(StopCommand { kind }), cx);
                                        })
                                })
                                .when(stopping, |button| button.opacity(0.5))
                                .child(Icon::Stop.view(px(10.), theme.danger))
                                .child(div().typography(Type::Small).text_color(theme.danger).child("Stop"))
                                .tag(stop_tag),
                        )
                    })
                    .when(status != CommandStatus::Idle, |right| {
                        right.child(status_pill(kind, &status, &theme))
                    }),
            );

        let body = if self.session.is_some() {
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .track_focus(&self.focus)
                .key_context(TERMINAL_CONTEXT)
                .on_key_down(cx.listener(Self::key_down))
                .on_action(cx.listener(|pane, _: &TerminalEscape, _, cx| pane.write(b"\x1b".to_vec(), cx)))
                .on_action(cx.listener(|pane, _: &TerminalBackTab, _, cx| pane.write(b"\x1b[Z".to_vec(), cx)))
                .on_action(cx.listener(|pane, _: &text_field::Copy, _, cx| pane.copy(cx)))
                .on_action(cx.listener(|pane, _: &text_field::Paste, _, cx| pane.paste(cx)))
                .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                .on_mouse_move(cx.listener(Self::mouse_move))
                .on_mouse_up(MouseButton::Left, cx.listener(|pane, _, _, _| pane.selecting = false))
                .on_scroll_wheel(cx.listener(Self::scroll_wheel))
                .child(
                    canvas(
                        move |bounds, window, cx| prepare(&pane, bounds, focused, window, cx),
                        move |_, prepared, window, cx| grid::paint(prepared, window, cx),
                    )
                    .size_full(),
                )
                .into_any_element()
        } else {
            empty_state(kind, self.configured_command.clone(), &theme).into_any_element()
        };

        div().size_full().flex().flex_col().child(meta_bar).child(body)
    }
}

fn status_pill(kind: CommandKind, status: &CommandStatus, theme: &Theme) -> impl IntoElement {
    let (ground, ink, dot) = match status {
        CommandStatus::Running => (theme.accent_wash, theme.accent_text, theme.accent_text),
        CommandStatus::Stopping | CommandStatus::Stopped | CommandStatus::Idle => (theme.hover, theme.text_dim, theme.text_faint),
        CommandStatus::Finished => (Hsla { a: 0.14, ..theme.success }, theme.success, theme.success),
        CommandStatus::Failed { .. } => (Hsla { a: 0.14, ..theme.danger }, theme.danger, theme.danger),
    };
    let dot_el = div().size(px(6.)).rounded_full().bg(dot);
    let dot_el = if *status == CommandStatus::Running {
        dot_el
            .with_animation(("run-status-dot", kind as usize), Animation::new(PULSE).repeat(), |el, t| {
                let alpha = 0.55 + 0.45 * ((t * std::f32::consts::TAU).cos() * 0.5 + 0.5);
                el.opacity(alpha)
            })
            .into_any_element()
    } else {
        dot_el.into_any_element()
    };

    div()
        .flex_none()
        .h(px(22.))
        .px(px(8.))
        .rounded_full()
        .bg(ground)
        .flex()
        .items_center()
        .gap(px(5.))
        .child(dot_el)
        .child(div().typography(Type::Small).text_color(ink).child(status.label(kind)))
}

fn empty_state(kind: CommandKind, configured: Option<String>, theme: &Theme) -> impl IntoElement {
    let (icon, heading) = match kind {
        CommandKind::Build => (Icon::Hammer, "No build started"),
        CommandKind::Run => (Icon::Play, "No run started"),
    };
    let (subtitle, action) = match configured {
        Some(command) => (
            div().typography(Type::Mono).text_color(theme.text_dim).child(command).into_any_element(),
            Button::new(format!("start-empty-command-{}", kind.name()), kind.title(), ButtonKind::Secondary)
                .action(TriggerProjectCommand { kind }),
        ),
        None => (
            div()
                .typography(Type::Small)
                .text_color(theme.text_faint)
                .child(format!("Set a project {} command to run it here.", kind.name()))
                .into_any_element(),
            Button::new(format!("configure-empty-command-{}", kind.name()), "Configure…", ButtonKind::Secondary)
                .action(OpenCommandEditor { kind }),
        ),
    };

    div()
        .flex_1()
        .size_full()
        .px(px(24.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .child(icon.view(px(18.), theme.text_faint))
        .child(div().typography(Type::UiStrong).text_color(theme.text).child(heading))
        .child(subtitle)
        .child(div().pt(px(4.)).child(action))
}

fn prepare(pane: &Entity<RunPane>, bounds: Bounds<Pixels>, focused: bool, window: &mut Window, cx: &mut App) -> Prepared {
    let (grid, font, font_size) = Grid::measure(bounds, window);
    let (snapshot, show_cursor) = pane.update(cx, |pane, _| {
        pane.fit(grid.columns, grid.rows, grid.cell_px());
        pane.grid = Some(grid);
        let active = pane.accepts_input();
        (pane.session.as_ref().map(CommandSession::snapshot), active)
    });
    Prepared {
        snapshot,
        grid,
        font,
        font_size,
        theme: Theme::get(cx).clone(),
        focused,
        show_cursor,
    }
}
