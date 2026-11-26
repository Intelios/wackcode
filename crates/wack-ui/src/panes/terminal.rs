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
//!   too (bound in the "Terminal" key context), unless a context menu or notice is open to dismiss,
//!   so the way out by keyboard is ⌃`.
//! - Selecting output raises a floating "Add to draft" pill above the selection's end row (never
//!   while dragging, so it never steals mouse-up), and right-clicking opens a context menu with
//!   Copy, Add selection to draft, Paste, Select all, Clear screen, and Restart shell.
//! - Not yet: IME composition, mouse reporting to programs, and links.

use std::path::PathBuf;

use futures::{StreamExt, channel::mpsc};
use gpui::{
    AnchoredPositionMode, App, Bounds, ClipboardItem, Context, Entity, FocusHandle, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollWheelEvent, SharedString, Task, Window, anchored, canvas,
    deferred, div, point, prelude::*, px,
};
use wack_core::terminal::{SelectionKind, Terminal, TerminalEvent};

pub use crate::panes::grid::TERMINAL_CONTEXT;
use crate::{
    actions::{AttachTerminalSelection, TerminalBackTab, TerminalEscape},
    icon::Icon,
    motion,
    panes::{
        PaneId,
        grid::{self, Grid, PADDING_X, PADDING_Y, Prepared},
    },
    tags::Tag,
    text_field,
    theme::Theme,
    typography::{Type, Typography},
};

type MenuAction = Box<dyn Fn(&mut TerminalPane, &mut Window, &mut Context<TerminalPane>)>;

const PILL_HEIGHT: f32 = 28.;
const PILL_MIN_WIDTH: f32 = 128.;
const PILL_CLEARANCE: f32 = 36.;
const PILL_OFFSET_ABOVE: f32 = 32.;
const PILL_GAP_BELOW: f32 = 6.;
const MENU_WIDTH: f32 = 220.;

pub struct TerminalPane {
    id: Option<PaneId>,
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
    bounds: Option<Bounds<Pixels>>,
    scroll_remainder: f32,
    selecting: bool,
    /// Open right-click context menu, anchored in window coordinates.
    menu: Option<Point<Pixels>>,
    /// Why the composer refused a selection (caps exceeded); cleared on the next interaction.
    notice: Option<SharedString>,
    /// Selection to re-apply after a fixture terminal resizes on its first layout pass.
    fixture_selection: Option<((usize, usize), (usize, usize))>,
}

impl TerminalPane {
    pub fn new(id: Option<PaneId>, folder: PathBuf, cx: &mut Context<Self>) -> Self {
        Self {
            id,
            folder,
            terminal: None,
            fixture: false,
            failed: None,
            title: None,
            exited: None,
            focus: cx.focus_handle(),
            _events: None,
            grid: None,
            bounds: None,
            scroll_remainder: 0.,
            selecting: false,
            menu: None,
            notice: None,
            fixture_selection: None,
        }
    }

    /// A pane showing what `bytes` draw, with no shell behind it.
    pub fn fixture(bytes: &[u8], columns: usize, rows: usize, title: Option<&str>, cx: &mut Context<Self>) -> Self {
        let mut pane = Self::new(None, PathBuf::from("/"), cx);
        pane.terminal = Some(Terminal::replay(columns, rows, bytes));
        pane.fixture = true;
        pane.title = title.map(Into::into);
        pane
    }

    /// Selects a range on a fixture pane, for gallery scenes showing the floating pill or menu.
    pub fn with_selection(mut self, start: (usize, usize), end: (usize, usize)) -> Self {
        self.fixture_selection = Some((start, end));
        if let Some(terminal) = &self.terminal {
            terminal.select_from(start.0, start.1, false, SelectionKind::Characters);
            terminal.select_to(end.0, end.1, true);
        }
        self
    }

    /// Opens the right-click menu at `position` (window coordinates), for gallery scenes.
    pub fn with_menu(mut self, position: Point<Pixels>) -> Self {
        self.menu = Some(position);
        self
    }

    /// Shows a refusal notice in the top-right corner, for gallery scenes.
    pub fn with_notice(mut self, notice: impl Into<SharedString>) -> Self {
        self.notice = Some(notice.into());
        self
    }

    /// The shell's own title, if it set one.
    pub fn title(&self) -> Option<SharedString> {
        self.title.clone().filter(|title| !title.trim().is_empty()).map(Into::into)
    }

    /// Shows or clears why the composer refused the last selection.
    pub fn set_notice(&mut self, notice: Option<String>, cx: &mut Context<Self>) {
        self.notice = notice.map(Into::into);
        cx.notify();
    }

    /// Starts the shell at this size, or fits the running one to it.
    fn fit(&mut self, columns: usize, rows: usize, cell: (u16, u16), cx: &mut Context<Self>) {
        if self.failed.is_some() || columns == 0 || rows == 0 {
            return;
        }
        if let Some(terminal) = &mut self.terminal {
            terminal.resize(columns, rows, cell);
            if let Some((start, end)) = self.fixture_selection {
                terminal.select_from(start.0, start.1, false, SelectionKind::Characters);
                terminal.select_to(end.0, end.1, true);
            }
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
        if self.fixture {
            return;
        }
        self.terminal = None;
        self._events = None;
        self.exited = None;
        self.title = None;
        self.menu = None;
        self.notice = None;
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        if let Some(terminal) = &self.terminal {
            terminal.clear();
            self.notice = None;
            cx.notify();
        }
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        if let Some(terminal) = &self.terminal {
            terminal.select_all();
            self.notice = None;
            cx.notify();
        }
    }

    fn write(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if let Some(terminal) = &self.terminal {
            terminal.clear_selection();
            terminal.write(bytes);
            self.menu = None;
            self.notice = None;
            cx.notify();
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.terminal.as_ref().and_then(Terminal::selected_text) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if self.exited.is_some() || self.failed.is_some() {
            return;
        }
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        if let (Some(terminal), Some(text)) = (&self.terminal, text) {
            terminal.clear_selection();
            terminal.paste(&text);
            self.notice = None;
            cx.notify();
        }
    }

    fn add_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.notice = None;
        let Some(text) = self.terminal.as_ref().and_then(Terminal::selected_text) else {
            cx.notify();
            return;
        };
        window.dispatch_action(Box::new(AttachTerminalSelection { pane: self.id, text }), cx);
        cx.notify();
    }

    fn escape(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() || self.notice.take().is_some() {
            cx.notify();
            return;
        }
        self.write(b"\x1b".to_vec(), cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.platform {
            match keystroke.key.as_str() {
                "c" => self.copy(cx),
                "v" => self.paste(cx),
                "a" => self.select_all(cx),
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
        self.menu = None;
        self.notice = None;
        let (Some(terminal), Some(grid)) = (&self.terminal, self.grid) else {
            cx.notify();
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

    fn right_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Keep the terminal's focus and current selection alive for Copy / Add selection to draft.
        window.focus(&self.focus, cx);
        self.selecting = false;
        self.notice = None;
        self.menu = Some(event.position);
        cx.stop_propagation();
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

    fn mouse_up(&mut self, _: &gpui::MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.selecting = false;
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

    fn context_menu(&self, position: Point<Pixels>, window: &Window, cx: &mut Context<Self>, theme: &Theme) -> impl IntoElement {
        let viewport = window.viewport_size();
        let selected = self.terminal.as_ref().and_then(Terminal::selected_text);
        let has_copyable = selected.is_some();
        let has_selection = selected.is_some_and(|text| !text.trim().is_empty());
        let has_clipboard = cx.read_from_clipboard().and_then(|item| item.text()).is_some_and(|text| !text.is_empty());
        let has_terminal = self.terminal.is_some();
        let live = has_terminal && self.exited.is_none() && self.failed.is_none();

        let backdrop = anchored()
            .position_mode(AnchoredPositionMode::Window)
            .position(point(px(0.), px(0.)))
            .child(
                div()
                    .id("terminal-menu-backdrop")
                    .w(viewport.width)
                    .h(viewport.height)
                    .on_mouse_down(MouseButton::Left, cx.listener(|pane, _, _, cx| {
                        pane.menu = None;
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .on_mouse_down(MouseButton::Right, cx.listener(|pane, _, _, cx| {
                        pane.menu = None;
                        cx.stop_propagation();
                        cx.notify();
                    })),
            );

        let row = |id: &'static str,
                   label: &'static str,
                   icon: Option<Icon>,
                   hint: Option<&'static str>,
                   enabled: bool,
                   tag: Option<&'static str>,
                   on_select: MenuAction| {
            let item = div()
                .id(id)
                .h(px(30.))
                .px(px(10.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .gap(px(9.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .when(enabled, |row| {
                    row.cursor_pointer()
                        .hover(|row| row.bg(theme.hover))
                        .on_click(cx.listener(move |pane, _, window, cx| {
                            pane.menu = None;
                            on_select(pane, window, cx);
                            cx.notify();
                        }))
                })
                .child(
                    div()
                        .w(px(14.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when_some(icon, |slot, icon| {
                            slot.child(icon.view(
                                px(13.),
                                if enabled {
                                    if icon == Icon::FilePlus { theme.accent_text } else { theme.text_dim }
                                } else {
                                    theme.text_faint
                                },
                            ))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .typography(Type::Ui)
                        .text_color(if enabled { theme.text } else { theme.text_faint })
                        .child(label),
                )
                .when_some(hint, |row, hint| row.child(div().typography(Type::Small).text_color(theme.text_faint).child(hint)));
            match tag {
                Some(tag) => item.tag(tag).into_any_element(),
                None => item.into_any_element(),
            }
        };

        let card = div()
            .id("terminal-context-menu")
            .occlude()
            .w(px(MENU_WIDTH))
            .p(px(5.))
            .rounded(px(12.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_lg()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(row(
                "term-menu-copy",
                "Copy",
                Some(Icon::Copy),
                Some("⌘C"),
                has_copyable,
                None,
                Box::new(|pane, _, cx| pane.copy(cx)),
            ))
            .child(row(
                "term-menu-add",
                "Add selection to draft",
                Some(Icon::FilePlus),
                None,
                has_selection,
                Some("terminal-add-menu-item"),
                Box::new(|pane, window, cx| pane.add_selection(window, cx)),
            ))
            .child(row(
                "term-menu-paste",
                "Paste",
                None,
                Some("⌘V"),
                live && has_clipboard,
                None,
                Box::new(|pane, _, cx| pane.paste(cx)),
            ))
            .child(row(
                "term-menu-select-all",
                "Select all",
                None,
                Some("⌘A"),
                has_terminal,
                None,
                Box::new(|pane, _, cx| pane.select_all(cx)),
            ))
            .child(div().my(px(4.)).mx(px(6.)).h(px(1.)).bg(theme.border))
            .child(row(
                "term-menu-clear",
                "Clear screen",
                Some(Icon::Erase),
                None,
                live,
                None,
                Box::new(|pane, _, cx| pane.clear(cx)),
            ))
            .child(row(
                "term-menu-restart",
                "Restart shell",
                Some(Icon::Reload),
                None,
                !self.fixture,
                None,
                Box::new(|pane, _, cx| pane.restart(cx)),
            ));

        div()
            .child(deferred(backdrop).with_priority(1))
            .child(
                deferred(
                    anchored()
                        .position_mode(AnchoredPositionMode::Window)
                        .position(position)
                        .snap_to_window_with_margin(px(8.))
                        .child(motion::enter("terminal-menu-arrive", card).tag("terminal-context-menu")),
                )
                .with_priority(2),
            )
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
        let pill = (!self.selecting && self.menu.is_none())
            .then(|| {
                let (row, column) = self.terminal.as_ref()?.selection_end()?;
                let grid = self.grid?;
                let bounds = self.bounds?;
                Some(pill_position(grid, bounds, row, column))
            })
            .flatten();

        div()
            .size_full()
            .relative()
            .track_focus(&self.focus)
            .key_context(TERMINAL_CONTEXT)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|pane, _: &TerminalEscape, _, cx| pane.escape(cx)))
            .on_action(cx.listener(|pane, _: &TerminalBackTab, _, cx| pane.write(b"\x1b[Z".to_vec(), cx)))
            .on_action(cx.listener(|pane, _: &text_field::Copy, _, cx| pane.copy(cx)))
            .on_action(cx.listener(|pane, _: &text_field::Paste, _, cx| pane.paste(cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::right_mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .child(
                canvas(
                    move |bounds, window, cx| prepare(&pane, bounds, focused, window, cx),
                    move |_, prepared, window, cx| grid::paint(prepared, window, cx),
                )
                .size_full(),
            )
            .when_some(pill, |pane, (top, left)| {
                let button = div()
                    .id("terminal-add-pill")
                    .h(px(PILL_HEIGHT))
                    .px(px(12.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .bg(theme.raised)
                    .border_1()
                    .border_color(theme.border_strong)
                    .shadow_md()
                    .cursor_pointer()
                    .hover(|pill| pill.bg(theme.hover).border_color(theme.accent_line))
                    // Keep the terminal's focus and selection alive until click runs.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|pane, _, window, cx| pane.add_selection(window, cx)))
                    .child(Icon::FilePlus.view(px(12.), theme.accent_text))
                    .child(div().typography(Type::Small).font_weight(gpui::FontWeight::SEMIBOLD).text_color(theme.text).child("Add to draft"));
                pane.child(div().absolute().top(px(top)).left(px(left)).child(motion::enter("terminal-pill-arrive", button).tag("terminal-add-pill")))
            })
            .when_some(self.notice.clone(), |pane, notice| {
                pane.child(
                    div()
                        .id("terminal-add-notice")
                        .absolute()
                        .top(px(10.))
                        .right(px(12.))
                        .max_w(px(280.))
                        .px(px(11.))
                        .py(px(7.))
                        .rounded(px(10.))
                        .bg(theme.raised)
                        .border_1()
                        .border_color(theme.border_strong)
                        .shadow_md()
                        .typography(Type::Small)
                        .text_color(theme.warning)
                        .child(notice)
                        .tag("terminal-add-notice"),
                )
            })
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
            .when_some(self.menu, |pane, position| pane.child(self.context_menu(position, window, cx, &theme)))
    }
}

/// Where the floating "Add to draft" pill sits over the pane: directly below the selection's last
/// row so none of the selected lines are covered (flipping above it when near the bottom edge),
/// clamped horizontally inside the pane.
fn pill_position(grid: Grid, bounds: Bounds<Pixels>, row: usize, column: usize) -> (f32, f32) {
    let x = PADDING_X + column as f32 * grid.cell_width;
    let y = PADDING_Y + row as f32 * grid.line_height;
    let pane_width = f32::from(bounds.size.width);
    let pane_height = f32::from(bounds.size.height);
    let below = y + grid.line_height + PILL_GAP_BELOW;
    let top = if below + PILL_HEIGHT <= pane_height - PADDING_Y {
        below
    } else if y >= PILL_CLEARANCE {
        y - PILL_OFFSET_ABOVE
    } else {
        (pane_height - PILL_HEIGHT - PADDING_Y).max(PADDING_Y)
    };
    let max_left = (pane_width - PILL_MIN_WIDTH - PADDING_X).max(PADDING_X);
    let left = x.clamp(PADDING_X, max_left);
    (top, left)
}

/// Measures the cell, fits the shell to the space, and takes a snapshot to paint.
fn prepare(pane: &Entity<TerminalPane>, bounds: Bounds<Pixels>, focused: bool, window: &mut Window, cx: &mut App) -> Prepared {
    let (grid, font, font_size) = Grid::measure(bounds, window);
    let snapshot = pane.update(cx, |pane, cx| {
        pane.fit(grid.columns, grid.rows, grid.cell_px(), cx);
        pane.grid = Some(grid);
        pane.bounds = Some(bounds);
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

#[cfg(test)]
mod tests {
    use gpui::{point, size};

    use super::*;

    #[test]
    fn pill_sits_below_the_selection_end_or_flips_above_near_bottom_and_clamps_width() {
        let grid = Grid { origin: point(px(12.), px(8.)), cell_width: 8., line_height: 18., columns: 40, rows: 12 };
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(344.), px(232.)));

        // Row 4, col 10 -> sits cleanly below row 4 (80 + 18 + 6 = 104).
        let (top, left) = pill_position(grid, bounds, 4, 10);
        assert_eq!((top, left), (104., 92.));

        // Bottom row (row 11) -> no room below, so flips above row 11, and left clamps inside width.
        let (top, left) = pill_position(grid, bounds, 11, 38);
        assert_eq!(top, PADDING_Y + 11. * 18. - PILL_OFFSET_ABOVE);
        assert_eq!(left, 344. - PILL_MIN_WIDTH - PADDING_X);
    }
}
