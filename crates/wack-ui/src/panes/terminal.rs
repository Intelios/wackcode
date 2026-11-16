//! The Terminal pane: a shell in the project folder, drawn on a grid of our own.
//!
//! The engine runs the shell and keeps the screen (`wack_core::terminal`); this view paints a
//! snapshot of it each frame and turns keys, clicks and scrolling into what a terminal sends.
//!
//! Invariants:
//! - The shell starts the first time the pane is drawn, at the size it is drawn at, and lives as
//!   long as the pane: hiding the dock keeps it, closing the pane ends it.
//! - Colours come from the theme: the ANSI sixteen are theme tokens, and a program's exact
//!   colours are shown as asked.
//! - ⌘ shortcuts stay the app's; everything else typed goes to the shell. Esc goes to the shell
//!   too (bound in the "Terminal" key context), so the way out by keyboard is ⌃`.
//! - Not yet: IME composition, mouse reporting to programs, and links.

use std::path::PathBuf;

use futures::{StreamExt, channel::mpsc};
use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    Pixels, ScrollWheelEvent, SharedString, Task, Window, canvas, div, prelude::*, px,
};
use wack_core::terminal::{SelectionKind, Terminal, TerminalEvent};

pub use crate::panes::grid::TERMINAL_CONTEXT;
use crate::{
    actions::{TerminalBackTab, TerminalEscape},
    panes::grid::{self, Grid, PADDING_X, PADDING_Y, Prepared},
    text_field,
    theme::Theme,
    typography::{Type, Typography},
};

pub struct TerminalPane {
    folder: PathBuf,
    terminal: Option<Terminal>,
    /// A fixed screen for gallery scenes: never starts a shell.
    fixture: bool,
    failed: Option<String>,
    /// What the shell called itself, if it did.
    title: Option<String>,
    /// The shell ended, with its exit code when it had one.
    exited: Option<Option<i32>>,
    pub focus: FocusHandle,
    _events: Option<Task<()>>,
    grid: Option<Grid>,
    scroll_remainder: f32,
    selecting: bool,
}

impl TerminalPane {
    pub fn new(folder: PathBuf, cx: &mut Context<Self>) -> Self {
        Self {
            folder,
            terminal: None,
            fixture: false,
            failed: None,
            title: None,
            exited: None,
            focus: cx.focus_handle(),
            _events: None,
            grid: None,
            scroll_remainder: 0.,
            selecting: false,
        }
    }

    /// A pane showing what `bytes` draw, with no shell behind it.
    pub fn fixture(bytes: &[u8], columns: usize, rows: usize, title: Option<&str>, cx: &mut Context<Self>) -> Self {
        let mut pane = Self::new(PathBuf::from("/"), cx);
        pane.terminal = Some(Terminal::replay(columns, rows, bytes));
        pane.fixture = true;
        pane.title = title.map(Into::into);
        pane
    }

    /// The shell's own title, if it set one.
    pub fn title(&self) -> Option<SharedString> {
        self.title.clone().filter(|title| !title.trim().is_empty()).map(Into::into)
    }

    /// Starts the shell at this size, or fits the running one to it.
    fn fit(&mut self, columns: usize, rows: usize, cell: (u16, u16), cx: &mut Context<Self>) {
        if self.failed.is_some() || columns == 0 || rows == 0 {
            return;
        }
        if let Some(terminal) = &mut self.terminal {
            terminal.resize(columns, rows, cell);
            return;
        }
        if self.fixture || self.exited.is_some() {
            return;
        }
        let (sender, mut receiver) = mpsc::unbounded::<TerminalEvent>();
        match Terminal::spawn(&self.folder, columns, rows, cell, move |event| {
            let _ = sender.unbounded_send(event);
        }) {
            Ok(terminal) => {
                self.terminal = Some(terminal);
                self._events = Some(cx.spawn(async move |pane, cx| {
                    while let Some(first) = receiver.next().await {
                        let mut batch = vec![first];
                        while let Ok(event) = receiver.try_recv() {
                            batch.push(event);
                        }
                        if pane.update(cx, |pane, cx| pane.apply(batch, cx)).is_err() {
                            return;
                        }
                    }
                }));
            }
            Err(message) => self.failed = Some(message),
        }
    }

    fn apply(&mut self, batch: Vec<TerminalEvent>, cx: &mut Context<Self>) {
        for event in batch {
            match event {
                TerminalEvent::Wakeup | TerminalEvent::Bell => {}
                TerminalEvent::Title(title) => self.title = Some(title),
                TerminalEvent::ResetTitle => self.title = None,
                TerminalEvent::Exited(code) => self.exited = Some(code),
            }
        }
        cx.notify();
    }

    /// A fresh shell after the last one ended.
    fn restart(&mut self, cx: &mut Context<Self>) {
        self.terminal = None;
        self._events = None;
        self.exited = None;
        self.title = None;
        cx.notify();
    }

    fn write(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if let Some(terminal) = &self.terminal {
            terminal.clear_selection();
            terminal.write(bytes);
            cx.notify();
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.terminal.as_ref().and_then(Terminal::selected_text) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        if let (Some(terminal), Some(text)) = (&self.terminal, text) {
            terminal.clear_selection();
            terminal.paste(&text);
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
        if self.exited.is_some() {
            if keystroke.key == "enter" {
                self.restart(cx);
                cx.stop_propagation();
            }
            return;
        }
        let Some(app_cursor) = self.terminal.as_ref().map(|terminal| terminal.modes().app_cursor) else {
            return;
        };
        if let Some(bytes) = grid::encode(keystroke, app_cursor) {
            self.write(bytes, cx);
            cx.stop_propagation();
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (Some(terminal), Some(grid)) = (&self.terminal, self.grid) else {
            return;
        };
        let (row, column, right_half) = grid.cell_at(event.position);
        if event.modifiers.shift && event.click_count == 1 {
            terminal.select_to(row, column, right_half);
        } else {
            let kind = match event.click_count {
                2 => SelectionKind::Words,
                3.. => SelectionKind::Lines,
                _ => SelectionKind::Characters,
            };
            terminal.select_from(row, column, right_half, kind);
        }
        self.selecting = true;
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            self.selecting = false;
            return;
        }
        if let (Some(terminal), Some(grid)) = (&self.terminal, self.grid) {
            let (row, column, right_half) = grid.cell_at(event.position);
            terminal.select_to(row, column, right_half);
            cx.notify();
        }
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(grid), Some(terminal)) = (self.grid, &self.terminal) else {
            return;
        };
        let lines = grid::scroll_lines(grid, event, &mut self.scroll_remainder);
        if lines == 0 {
            return;
        }
        let modes = terminal.modes();
        if modes.alt_screen {
            let arrow = match (lines > 0, modes.app_cursor) {
                (true, true) => "\x1bOA",
                (true, false) => "\x1b[A",
                (false, true) => "\x1bOB",
                (false, false) => "\x1b[B",
            };
            terminal.write(arrow.repeat(lines.unsigned_abs() as usize).into_bytes());
        } else {
            terminal.scroll(lines);
        }
        cx.notify();
    }
}

impl Render for TerminalPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let focused = self.focus.is_focused(window);
        let pane = cx.entity();
        let ended = self.exited.map(|code| match code {
            Some(0) | None => "The shell ended. Press ↩ for a new one.".to_string(),
            Some(code) => format!("The shell ended with code {code}. Press ↩ for a new one."),
        });
        div()
            .size_full()
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
            .when_some(self.failed.clone(), |pane, message| {
                pane.child(
                    div()
                        .absolute()
                        .top(px(PADDING_Y))
                        .left(px(PADDING_X))
                        .right(px(PADDING_X))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(div().typography(Type::UiStrong).text_color(theme.danger).child("The shell could not start."))
                        .child(div().typography(Type::Small).text_color(theme.text_dim).child(message)),
                )
            })
            .when_some(ended, |pane, ended| {
                pane.child(
                    div()
                        .absolute()
                        .bottom(px(PADDING_Y))
                        .left(px(PADDING_X))
                        .px(px(8.))
                        .py(px(3.))
                        .rounded(px(6.))
                        .bg(theme.raised)
                        .border_1()
                        .border_color(theme.border)
                        .typography(Type::Small)
                        .text_color(theme.text_soft)
                        .child(ended),
                )
            })
    }
}

/// Measures the cell, fits the shell to the space, and takes a snapshot to paint.
fn prepare(pane: &Entity<TerminalPane>, bounds: Bounds<Pixels>, focused: bool, window: &mut Window, cx: &mut App) -> Prepared {
    let (grid, font, font_size) = Grid::measure(bounds, window);
    let snapshot = pane.update(cx, |pane, cx| {
        pane.fit(grid.columns, grid.rows, grid.cell_px(), cx);
        pane.grid = Some(grid);
        pane.terminal.as_ref().map(Terminal::snapshot)
    });
    Prepared {
        snapshot,
        grid,
        font,
        font_size,
        theme: Theme::get(cx).clone(),
        focused,
        show_cursor: true,
    }
}
