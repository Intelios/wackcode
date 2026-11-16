//! Shared VT grid measurement, painting, hit-testing and key encoding for `TerminalPane` and
//! `RunPane`.
//!
//! Pure layout-and-drawing primitives here take any engine [`Snapshot`]; neither type knows
//! whether the backing PTY is an interactive shell or a Build/Run process.

use gpui::{
    App, Bounds, Font, FontStyle, FontWeight, Hsla, Keystroke, Pixels, Point, ScrollWheelEvent, StrikethroughStyle,
    TextAlign, TextRun, UnderlineStyle, Window, fill, font, outline, point, px, size,
};
use wack_core::terminal::{Cell, Colour, CursorKind, Snapshot};

use crate::{theme::Theme, typography::Type};

pub const TERMINAL_CONTEXT: &str = "Terminal";
pub const PADDING_X: f32 = 12.;
pub const PADDING_Y: f32 = 8.;

/// Where the grid was last drawn, to turn mouse positions into cells.
#[derive(Clone, Copy)]
pub struct Grid {
    pub origin: Point<Pixels>,
    pub cell_width: f32,
    pub line_height: f32,
    pub columns: usize,
    pub rows: usize,
}

impl Grid {
    pub fn measure(bounds: Bounds<Pixels>, window: &Window) -> (Self, Font, Pixels) {
        let style = Type::Terminal.style();
        let mono = font(style.family);
        let font_size = px(style.size);
        let font_id = window.text_system().resolve_font(&mono);
        let cell_width = window
            .text_system()
            .advance(font_id, font_size, 'm')
            .map(|advance| f32::from(advance.width))
            .unwrap_or(7.2);
        let line_height = style.line_height;
        let width = f32::from(bounds.size.width) - 2. * PADDING_X;
        let height = f32::from(bounds.size.height) - 2. * PADDING_Y;
        let columns = (width / cell_width).floor().max(0.) as usize;
        let rows = (height / line_height).floor().max(0.) as usize;
        let grid = Self {
            origin: bounds.origin + point(px(PADDING_X), px(PADDING_Y)),
            cell_width,
            line_height,
            columns,
            rows,
        };
        (grid, mono, font_size)
    }

    pub fn cell_px(&self) -> (u16, u16) {
        (self.cell_width.round() as u16, self.line_height.round() as u16)
    }

    /// The cell under `position`, and whether its right half was hit.
    pub fn cell_at(&self, position: Point<Pixels>) -> (usize, usize, bool) {
        let x = f32::from(position.x - self.origin.x).max(0.);
        let y = f32::from(position.y - self.origin.y).max(0.);
        let column = ((x / self.cell_width) as usize).min(self.columns.saturating_sub(1));
        let row = ((y / self.line_height) as usize).min(self.rows.saturating_sub(1));
        let right_half = x - column as f32 * self.cell_width > self.cell_width / 2.;
        (row, column, right_half)
    }
}

pub struct Prepared {
    pub snapshot: Option<Snapshot>,
    pub grid: Grid,
    pub font: Font,
    pub font_size: Pixels,
    pub theme: Theme,
    pub focused: bool,
    pub show_cursor: bool,
}

pub fn scroll_lines(grid: Grid, event: &ScrollWheelEvent, remainder: &mut f32) -> i32 {
    *remainder += f32::from(event.delta.pixel_delta(px(grid.line_height)).y);
    let lines = (*remainder / grid.line_height).trunc() as i32;
    if lines != 0 {
        *remainder -= lines as f32 * grid.line_height;
    }
    lines
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

pub fn paint(prepared: Prepared, window: &mut Window, cx: &mut App) {
    let Prepared {
        snapshot,
        grid,
        font,
        font_size,
        theme,
        focused,
        show_cursor,
    } = prepared;
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
                if next.spacer
                    || !next.character.is_ascii()
                    || next.foreground != cell.foreground
                    || next.style != cell.style
                    || next.selected != cell.selected
                {
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
                underline: cell.style.underline.then_some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(colour),
                    wavy: false,
                }),
                strikethrough: cell.style.strikethrough.then_some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(colour),
                }),
            };
            let shaped = window.text_system().shape_line(text.into(), font_size, &[run], None);
            let origin = cell_bounds(row, start, 1).origin;
            let _ = shaped.paint(origin, line_height, TextAlign::Left, None, window, cx);
        }
    }

    if !show_cursor {
        return;
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
                let run = TextRun {
                    len: cell.character.len_utf8(),
                    font,
                    color: theme.on_accent,
                    ..TextRun::default()
                };
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

/// What a keystroke sends to the PTY, or `None` for keys the terminal leaves to the app.
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
