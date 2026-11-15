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
    App, Bounds, ClipboardItem, Context, Entity, FocusHandle, Font, FontStyle, FontWeight, Hsla, KeyDownEvent, Keystroke,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollWheelEvent, SharedString, StrikethroughStyle, Task,
    TextAlign, TextRun, UnderlineStyle, Window, canvas, div, fill, font, outline, point, prelude::*, px, size,
};
use wack_core::terminal::{Cell, Colour, CursorKind, SelectionKind, Snapshot, Terminal, TerminalEvent};

use crate::{
    actions::TerminalEscape,
    text_field,
    theme::Theme,
    typography::{Type, Typography},
};

pub const TERMINAL_CONTEXT: &str = "Terminal";
/// Space between the pane's edge and the grid.
const PADDING_X: f32 = 12.;
const PADDING_Y: f32 = 8.;

/// Where the grid was last drawn, to turn mouse positions into cells.
#[derive(Clone, Copy)]
struct Grid {
    origin: Point<Pixels>,
    cell_width: f32,
    line_height: f32,
    columns: usize,
    rows: usize,
}

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
        if let Some(bytes) = encode(keystroke, app_cursor) {
            self.write(bytes, cx);
            cx.stop_propagation();
        }
    }

    /// The cell under `position`, and whether its right half was hit.
    fn cell_at(&self, position: Point<Pixels>) -> Option<(usize, usize, bool)> {
        let grid = self.grid?;
        let x = f32::from(position.x - grid.origin.x).max(0.);
        let y = f32::from(position.y - grid.origin.y).max(0.);
        let column = ((x / grid.cell_width) as usize).min(grid.columns.saturating_sub(1));
        let row = ((y / grid.line_height) as usize).min(grid.rows.saturating_sub(1));
        let right_half = x - column as f32 * grid.cell_width > grid.cell_width / 2.;
        Some((row, column, right_half))
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (Some(terminal), Some((row, column, right_half))) = (&self.terminal, self.cell_at(event.position)) else {
            return;
        };
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
        if let (Some(terminal), Some((row, column, right_half))) = (&self.terminal, self.cell_at(event.position)) {
            terminal.select_to(row, column, right_half);
            cx.notify();
        }
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(grid), Some(terminal)) = (self.grid, &self.terminal) else {
            return;
        };
        self.scroll_remainder += f32::from(event.delta.pixel_delta(px(grid.line_height)).y);
        let lines = (self.scroll_remainder / grid.line_height).trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_remainder -= lines as f32 * grid.line_height;
        let modes = terminal.modes();
        if modes.alt_screen {
            // A full-screen program has no scrollback: scrolling moves through it instead.
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
            .on_action(cx.listener(|pane, _: &text_field::Copy, _, cx| pane.copy(cx)))
            .on_action(cx.listener(|pane, _: &text_field::Paste, _, cx| pane.paste(cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(|pane, _, _, _| pane.selecting = false))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .child(
                canvas(
                    move |bounds, window, cx| prepare(&pane, bounds, focused, window, cx),
                    move |_, prepared, window, cx| paint(prepared, window, cx),
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

struct Prepared {
    snapshot: Option<Snapshot>,
    grid: Grid,
    font: Font,
    font_size: Pixels,
    theme: Theme,
    focused: bool,
}

/// Measures the cell, fits the shell to the space, and takes a snapshot to paint.
fn prepare(pane: &Entity<TerminalPane>, bounds: Bounds<Pixels>, focused: bool, window: &mut Window, cx: &mut App) -> Prepared {
    let style = Type::Terminal.style();
    let font = font(style.family);
    let font_size = px(style.size);
    let font_id = window.text_system().resolve_font(&font);
    let cell_width = window.text_system().advance(font_id, font_size, 'm').map(|size| f32::from(size.width)).unwrap_or(7.2);
    let line_height = style.line_height;
    let width = f32::from(bounds.size.width) - 2. * PADDING_X;
    let height = f32::from(bounds.size.height) - 2. * PADDING_Y;
    let columns = (width / cell_width).floor().max(0.) as usize;
    let rows = (height / line_height).floor().max(0.) as usize;
    let grid = Grid {
        origin: bounds.origin + point(px(PADDING_X), px(PADDING_Y)),
        cell_width,
        line_height,
        columns,
        rows,
    };
    let snapshot = pane.update(cx, |pane, cx| {
        pane.fit(columns, rows, (cell_width.round() as u16, line_height.round() as u16), cx);
        pane.grid = Some(grid);
        pane.terminal.as_ref().map(Terminal::snapshot)
    });
    Prepared { snapshot, grid, font, font_size, theme: Theme::get(cx).clone(), focused }
}

fn ink(colour: Colour, theme: &Theme) -> Hsla {
    match colour {
        Colour::Foreground => theme.terminal_text,
        Colour::Background => theme.dock,
        Colour::Indexed(index) => theme.indexed(index),
        Colour::Rgb(r, g, b) => Theme::exact(r, g, b),
    }
}

/// A cell's own ground, when it is not the dock's.
fn ground(cell: &Cell, theme: &Theme) -> Option<Hsla> {
    if cell.selected {
        return Some(theme.selection);
    }
    match cell.background {
        Colour::Background => None,
        other => Some(ink(other, theme)),
    }
}

fn foreground(cell: &Cell, theme: &Theme) -> Hsla {
    let colour = ink(cell.foreground, theme);
    if cell.style.dim { colour.opacity(0.62) } else { colour }
}

fn paint(prepared: Prepared, window: &mut Window, cx: &mut App) {
    let Prepared { snapshot, grid, font, font_size, theme, focused } = prepared;
    let Some(snapshot) = snapshot else {
        return;
    };
    let line_height = px(grid.line_height);
    let x = |column: usize| grid.origin.x + px(column as f32 * grid.cell_width);
    let cell_bounds = |row: usize, column: usize, columns: usize| {
        Bounds::new(
            point(x(column), grid.origin.y + px(row as f32 * grid.line_height)),
            size(px(columns as f32 * grid.cell_width), line_height),
        )
    };

    for (row, line) in snapshot.lines.iter().enumerate().take(grid.rows.max(1)) {
        // Grounds, one quad per run of the same colour.
        let mut column = 0;
        while column < line.len() {
            let Some(colour) = ground(&line[column], &theme) else {
                column += 1;
                continue;
            };
            let start = column;
            while column < line.len() && ground(&line[column], &theme) == Some(colour) {
                column += 1;
            }
            window.paint_quad(fill(cell_bounds(row, start, column - start), colour));
        }

        // Text, in runs of plain ASCII with one style, each placed at its first cell so the
        // grid stays aligned whatever a fallback font's glyphs measure.
        let mut column = 0;
        while column < line.len() {
            let cell = &line[column];
            if cell.spacer || cell.style.hidden || (cell.character == ' ' && !cell.style.underline && !cell.style.strikethrough) {
                column += 1;
                continue;
            }
            let start = column;
            let mut text = String::new();
            loop {
                let cell = &line[column];
                text.push(cell.character);
                if let Some(marks) = &cell.combining {
                    text.push_str(marks);
                }
                column += 1;
                let alone = !cell.character.is_ascii() || cell.wide;
                if alone || column >= line.len() {
                    break;
                }
                let next = &line[column];
                if next.spacer || !next.character.is_ascii() || next.foreground != cell.foreground || next.style != cell.style || next.selected != cell.selected {
                    break;
                }
            }
            let colour = foreground(cell, &theme);
            let run = TextRun {
                len: text.len(),
                font: Font {
                    weight: if cell.style.bold { FontWeight::BOLD } else { FontWeight::NORMAL },
                    style: if cell.style.italic { FontStyle::Italic } else { FontStyle::Normal },
                    ..font.clone()
                },
                color: colour,
                background_color: None,
                underline: cell.style.underline.then_some(UnderlineStyle { thickness: px(1.), color: Some(colour), wavy: false }),
                strikethrough: cell.style.strikethrough.then_some(StrikethroughStyle { thickness: px(1.), color: Some(colour) }),
            };
            let shaped = window.text_system().shape_line(text.into(), font_size, &[run], None);
            let origin = cell_bounds(row, start, 1).origin;
            let _ = shaped.paint(origin, line_height, TextAlign::Left, None, window, cx);
        }
    }

    let Some(cursor) = snapshot.cursor else {
        return;
    };
    let Some(cell) = snapshot.lines.get(cursor.row).and_then(|line| line.get(cursor.column)) else {
        return;
    };
    let width = if cell.wide { 2 } else { 1 };
    let bounds = cell_bounds(cursor.row, cursor.column, width);
    match (cursor.kind, focused) {
        (CursorKind::Block, true) => {
            window.paint_quad(fill(bounds, theme.accent));
            if cell.character != ' ' {
                let run = TextRun { len: cell.character.len_utf8(), font, color: theme.on_accent, ..TextRun::default() };
                let shaped = window.text_system().shape_line(cell.character.to_string().into(), font_size, &[run], None);
                let _ = shaped.paint(bounds.origin, line_height, TextAlign::Left, None, window, cx);
            }
        }
        (CursorKind::Beam, _) => window.paint_quad(fill(Bounds::new(bounds.origin, size(px(2.), line_height)), theme.accent)),
        (CursorKind::Underline, _) => window.paint_quad(fill(
            Bounds::new(point(bounds.origin.x, bounds.origin.y + line_height - px(2.)), size(bounds.size.width, px(2.))),
            theme.accent,
        )),
        _ => window.paint_quad(outline(bounds, theme.accent_line, gpui::BorderStyle::Solid)),
    }
}

/// What a keystroke sends to the shell, or `None` for keys the terminal leaves to the app.
pub fn encode(keystroke: &Keystroke, app_cursor: bool) -> Option<Vec<u8>> {
    let modifiers = keystroke.modifiers;
    if modifiers.platform {
        return None;
    }
    let (shift, alt, control) = (modifiers.shift, modifiers.alt, modifiers.control);
    let escaped = |bytes: &[u8]| {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        if alt {
            out.push(0x1b);
        }
        out.extend_from_slice(bytes);
        Some(out)
    };
    // xterm's modifier parameter: 1 + shift + 2·alt + 4·control.
    let parameter = 1 + shift as u8 + 2 * alt as u8 + 4 * control as u8;
    let cursor_key = |letter: char| {
        Some(if parameter > 1 {
            format!("\x1b[1;{parameter}{letter}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        })
    };
    let tilde = |number: u8| {
        Some(if parameter > 1 { format!("\x1b[{number};{parameter}~") } else { format!("\x1b[{number}~") }.into_bytes())
    };
    match keystroke.key.as_str() {
        "enter" => escaped(b"\r"),
        "backspace" if control => escaped(b"\x08"),
        "backspace" => escaped(b"\x7f"),
        "tab" if shift => Some(b"\x1b[Z".to_vec()),
        "tab" => escaped(b"\t"),
        "escape" => escaped(b"\x1b"),
        "space" if control => Some(vec![0]),
        "space" => escaped(b" "),
        // ⌥← and ⌥→ move by word, as in Terminal.app.
        "left" if alt && !shift && !control => Some(b"\x1bb".to_vec()),
        "right" if alt && !shift && !control => Some(b"\x1bf".to_vec()),
        "up" => cursor_key('A'),
        "down" => cursor_key('B'),
        "right" => cursor_key('C'),
        "left" => cursor_key('D'),
        "home" => cursor_key('H'),
        "end" => cursor_key('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => Some(b"\x1bOP".to_vec()),
        "f2" => Some(b"\x1bOQ".to_vec()),
        "f3" => Some(b"\x1bOR".to_vec()),
        "f4" => Some(b"\x1bOS".to_vec()),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        key if control => {
            let mut characters = key.chars();
            let (Some(character), None) = (characters.next(), characters.next()) else {
                return None;
            };
            let byte = match character.to_ascii_lowercase() {
                letter @ 'a'..='z' => letter as u8 - b'a' + 1,
                '@' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '/' | '7' => 0x1f,
                '8' | '?' => 0x7f,
                _ => return None,
            };
            escaped(&[byte])
        }
        _ => keystroke.key_char.as_ref().filter(|text| !text.is_empty()).map(|text| text.as_bytes().to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(keys: &str, app_cursor: bool) -> Option<Vec<u8>> {
        let mut keystroke = Keystroke::parse(keys).unwrap();
        // What macOS reports as typed for an unmodified printable key.
        if keystroke.key_char.is_none() && keystroke.key.chars().count() == 1 && !keystroke.modifiers.control {
            keystroke.key_char = Some(keystroke.key.clone());
        }
        encode(&keystroke, app_cursor)
    }

    #[test]
    fn keys_become_what_a_terminal_sends() {
        assert_eq!(send("a", false), Some(b"a".to_vec()));
        assert_eq!(send("enter", false), Some(b"\r".to_vec()));
        assert_eq!(send("backspace", false), Some(b"\x7f".to_vec()));
        assert_eq!(send("ctrl-c", false), Some(vec![3]));
        assert_eq!(send("ctrl-[", false), Some(vec![0x1b]));
        assert_eq!(send("escape", false), Some(vec![0x1b]));
        assert_eq!(send("shift-tab", false), Some(b"\x1b[Z".to_vec()));
        assert_eq!(send("up", false), Some(b"\x1b[A".to_vec()));
        assert_eq!(send("up", true), Some(b"\x1bOA".to_vec()), "application cursor mode");
        assert_eq!(send("ctrl-right", false), Some(b"\x1b[1;5C".to_vec()));
        assert_eq!(send("alt-left", false), Some(b"\x1bb".to_vec()));
        assert_eq!(send("delete", false), Some(b"\x1b[3~".to_vec()));
        assert_eq!(send("f5", false), Some(b"\x1b[15~".to_vec()));
        assert_eq!(send("cmd-c", false), None, "⌘ shortcuts stay the app's");
        assert_eq!(send("cmd-n", false), None);
    }
}
