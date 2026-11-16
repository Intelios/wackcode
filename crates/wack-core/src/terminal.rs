//! A terminal: a login shell on a PTY, and the screen it draws, as plain data.
//!
//! `alacritty_terminal` does the emulation (escape sequences, the grid, scrollback) and runs the
//! PTY on its own thread. This module keeps those types to itself: the UI asks for a
//! [`Snapshot`] of what is on screen, in our own types, and sends input as bytes. Nothing here
//! knows how it is drawn, so the drawing stays in `wack-ui` and the emulator could be swapped.
//!
//! Invariants:
//! - Dropping a [`Terminal`] ends it: the event loop shuts down and the shell gets SIGHUP.
//! - The shell is the user's login shell (through `/usr/bin/login` on macOS), started in the
//!   project folder, so it has the PATH and settings the user's own terminal has. It inherits
//!   the app's environment plus `TERM`, `COLORTERM` and `TERM_PROGRAM`; no credential is ever
//!   added to it.
//! - [`Terminal::replay`] builds a terminal with no shell from fixed bytes, for tests and gallery
//!   scenes; everything but input works the same on it.

use std::{
    borrow::Cow,
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

use alacritty_terminal::{
    event::{Event as AlacrittyEvent, EventListener, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{Config, Term, TermMode, cell::Flags},
    tty,
    vte::ansi::{ClearMode, Color, CursorShape, Handler, NamedColor, Processor, StdSyncHandler},
};

/// Lines kept above the screen.
const SCROLLBACK: usize = 10_000;

/// What the terminal tells its owner, from the PTY's thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    /// The screen changed.
    Wakeup,
    /// The shell set its title (usually the running command or the folder).
    Title(String),
    ResetTitle,
    Bell,
    /// The shell exited; the terminal shows what it last drew.
    Exited(Option<i32>),
}

/// A colour as the program asked for it. The UI turns it into a theme colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour {
    /// The terminal's own text colour.
    Foreground,
    /// The terminal's own ground.
    Background,
    /// One of the 256 indexed colours: 0–15 are the ANSI palette.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// Drawn as ground only (concealed text).
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub character: char,
    /// Combining marks drawn on the same cell.
    pub combining: Option<String>,
    pub foreground: Colour,
    pub background: Colour,
    pub style: Style,
    /// Takes two columns; the next cell is its spacer and draws nothing.
    pub wide: bool,
    pub spacer: bool,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorKind {
    Block,
    Underline,
    Beam,
    HollowBlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub column: usize,
    pub kind: CursorKind,
}

/// The screen as plain data: `rows` lines of `columns` cells.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub columns: usize,
    pub rows: usize,
    pub lines: Vec<Vec<Cell>>,
    /// `None` when the program hid it or the view is scrolled up past it.
    pub cursor: Option<Cursor>,
    /// How far the view is scrolled up into the scrollback, in lines.
    pub scrolled: usize,
}

/// The modes that change what keys and pastes send.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// Arrow keys send `ESC O A` rather than `ESC [ A`.
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    /// A full-screen program (an editor, a pager) is showing; there is no scrollback.
    pub alt_screen: bool,
}

/// How a selection grows from where it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    Characters,
    Words,
    Lines,
}

#[derive(Clone, Copy)]
struct Size {
    columns: usize,
    rows: usize,
    cell_width: u16,
    cell_height: u16,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

impl Size {
    fn window(&self) -> WindowSize {
        WindowSize {
            num_lines: self.rows as u16,
            num_cols: self.columns as u16,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

type ReplyFn = Arc<dyn Fn(Vec<u8>) + Send + Sync>;

/// Hands the emulator's events on, answering the ones that need a reply to the shell itself.
#[derive(Clone)]
pub(crate) struct Listener {
    sink: Arc<dyn Fn(TerminalEvent) + Send + Sync>,
    /// Set once the PTY exists; replies written before that are dropped.
    reply: Arc<Mutex<Option<ReplyFn>>>,
    size: Arc<Mutex<WindowSize>>,
}

impl Listener {
    fn reply(&self, text: String) {
        if let Some(reply) = self.reply.lock().ok().and_then(|slot| slot.clone()) {
            reply(text.into_bytes());
        }
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: AlacrittyEvent) {
        match event {
            AlacrittyEvent::Wakeup => (self.sink)(TerminalEvent::Wakeup),
            AlacrittyEvent::Title(title) => (self.sink)(TerminalEvent::Title(title)),
            AlacrittyEvent::ResetTitle => (self.sink)(TerminalEvent::ResetTitle),
            AlacrittyEvent::Bell => (self.sink)(TerminalEvent::Bell),
            AlacrittyEvent::ChildExit(status) => (self.sink)(TerminalEvent::Exited(status.code())),
            AlacrittyEvent::PtyWrite(text) => self.reply(text),
            AlacrittyEvent::TextAreaSizeRequest(format) => {
                let size = self.size.lock().map(|size| *size).ok();
                if let Some(size) = size {
                    self.reply(format(size));
                }
            }
            // Programs may not read or write the clipboard, or ask for colours: the first is a
            // way to leak what the user copied, and the second has no theme-independent answer.
            _ => {}
        }
    }
}

/// Pure PTY-agnostic terminal screen buffer & VT emulator, shared by interactive shells and
/// Build/Run command outputs.
pub struct Screen {
    term: Arc<FairMutex<Term<Listener>>>,
    parser: Mutex<Processor<StdSyncHandler>>,
    size: Size,
    listener: Listener,
}

impl Screen {
    pub fn new(
        columns: usize,
        rows: usize,
        cell: (u16, u16),
        events: impl Fn(TerminalEvent) + Send + Sync + 'static,
    ) -> Self {
        let size = Size { columns: columns.max(2), rows: rows.max(1), cell_width: cell.0, cell_height: cell.1 };
        let listener = Listener {
            sink: Arc::new(events),
            reply: Arc::new(Mutex::new(None)),
            size: Arc::new(Mutex::new(size.window())),
        };
        let term = Arc::new(FairMutex::new(Term::new(config(), &size, listener.clone())));
        Self {
            term,
            parser: Mutex::new(Processor::new()),
            size,
            listener,
        }
    }

    pub fn replay(columns: usize, rows: usize, bytes: &[u8]) -> Self {
        let screen = Self::new(columns, rows, (8, 16), |_| {});
        screen.advance(bytes);
        screen
    }

    pub(crate) fn set_reply(&self, reply: Option<ReplyFn>) {
        if let Ok(mut slot) = self.listener.reply.lock() {
            *slot = reply;
        }
    }

    pub(crate) fn term(&self) -> &Arc<FairMutex<Term<Listener>>> {
        &self.term
    }

    pub(crate) fn listener(&self) -> Listener {
        self.listener.clone()
    }

    pub(crate) fn window_size(&self) -> WindowSize {
        self.size.window()
    }

    pub fn columns(&self) -> usize {
        self.size.columns
    }

    pub fn rows(&self) -> usize {
        self.size.rows
    }

    /// Feeds bytes from a PTY or fixture into the emulator. Returns true if the screen should redraw.
    pub fn advance(&self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return false;
        }
        let mut parser = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        let mut term = self.term.lock();
        parser.advance(&mut *term, bytes);
        parser.sync_bytes_count() < bytes.len()
    }

    /// Clears the viewport, scrollback history, and selection, and homes the cursor without
    /// touching the underlying process.
    pub fn clear(&self) {
        let mut term = self.term.lock();
        term.selection = None;
        term.scroll_display(Scroll::Bottom);
        Handler::clear_screen(&mut *term, ClearMode::All);
        Handler::clear_screen(&mut *term, ClearMode::Saved);
        Handler::goto(&mut *term, 0, 0);
    }

    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(Scroll::Bottom);
    }

    /// Fits the screen to a new size. Returns `true` when the size actually changed.
    pub fn resize(&mut self, columns: usize, rows: usize, cell: (u16, u16)) -> bool {
        let size = Size { columns: columns.max(2), rows: rows.max(1), cell_width: cell.0, cell_height: cell.1 };
        if size.columns == self.size.columns && size.rows == self.size.rows {
            return false;
        }
        self.size = size;
        if let Ok(mut window) = self.listener.size.lock() {
            *window = size.window();
        }
        self.term.lock().resize(size);
        true
    }

    /// Scrolls the view through the scrollback: positive is up.
    pub fn scroll(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
    }

    pub fn modes(&self) -> Modes {
        let term = self.term.lock();
        let mode = term.mode();
        Modes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
        }
    }

    /// Starts a selection at a cell on screen; `right_half` is which half of the cell was hit.
    pub fn select_from(&self, row: usize, column: usize, right_half: bool, kind: SelectionKind) {
        let mut term = self.term.lock();
        let point = screen_point(&term, row, column);
        let kind = match kind {
            SelectionKind::Characters => SelectionType::Simple,
            SelectionKind::Words => SelectionType::Semantic,
            SelectionKind::Lines => SelectionType::Lines,
        };
        term.selection = Some(Selection::new(kind, point, side(right_half)));
    }

    pub fn select_to(&self, row: usize, column: usize, right_half: bool) {
        let mut term = self.term.lock();
        let point = screen_point(&term, row, column);
        if let Some(selection) = term.selection.as_mut() {
            selection.update(point, side(right_half));
        }
    }

    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }

    /// The selected text, if anything non-empty is selected.
    pub fn selected_text(&self) -> Option<String> {
        self.term.lock().selection_to_string().filter(|text| !text.is_empty())
    }

    /// What is on screen now.
    pub fn snapshot(&self) -> Snapshot {
        let term = self.term.lock();
        snapshot_from_term(&term)
    }
}

pub struct Terminal {
    screen: Screen,
    pty: Option<EventLoopSender>,
}

impl Terminal {
    /// Starts the user's login shell in `folder`, sized `columns` × `rows`. `events` is called
    /// from the PTY's thread.
    pub fn spawn(
        folder: &Path,
        columns: usize,
        rows: usize,
        cell: (u16, u16),
        events: impl Fn(TerminalEvent) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let screen = Screen::new(columns, rows, cell, events);
        let env = HashMap::from([
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
            ("TERM_PROGRAM".to_string(), "WackCode".to_string()),
        ]);
        let options = tty::Options {
            shell: None,
            working_directory: Some(folder.to_path_buf()),
            drain_on_exit: true,
            env,
        };
        let pty = tty::new(&options, screen.window_size(), 0).map_err(|error| format!("Could not start a shell: {error}."))?;
        let event_loop = EventLoop::new(screen.term().clone(), screen.listener(), pty, true, false)
            .map_err(|error| format!("Could not start the terminal: {error}."))?;
        let sender = event_loop.channel();
        let reply_sender = sender.clone();
        screen.set_reply(Some(Arc::new(move |bytes| {
            let _ = reply_sender.send(Msg::Input(Cow::Owned(bytes)));
        })));
        event_loop.spawn();
        Ok(Self { screen, pty: Some(sender) })
    }

    /// A terminal with no shell that has been sent `bytes`: for tests and gallery scenes.
    pub fn replay(columns: usize, rows: usize, bytes: &[u8]) -> Self {
        Self {
            screen: Screen::replay(columns, rows, bytes),
            pty: None,
        }
    }

    pub fn columns(&self) -> usize {
        self.screen.columns()
    }

    pub fn rows(&self) -> usize {
        self.screen.rows()
    }

    /// Sends bytes to the shell, as if typed. Typing also brings the view back to the bottom.
    pub fn write(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        self.screen.scroll_to_bottom();
        if let Some(pty) = &self.pty {
            let _ = pty.send(Msg::Input(Cow::Owned(bytes)));
        }
    }

    /// Pastes text: line endings become returns, and a program that asked for bracketed paste
    /// gets it bracketed, so a pasted command is not run before the user presses return.
    pub fn paste(&self, text: &str) {
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.modes().bracketed_paste {
            // A paste must not be able to end the bracket early.
            let text = text.replace("\x1b[201~", "");
            self.write(format!("\x1b[200~{text}\x1b[201~").into_bytes());
        } else {
            self.write(text.into_bytes());
        }
    }

    /// Fits the terminal to a new size. A no-op when nothing changed.
    pub fn resize(&mut self, columns: usize, rows: usize, cell: (u16, u16)) {
        if !self.screen.resize(columns, rows, cell) {
            return;
        }
        if let Some(pty) = &self.pty {
            let _ = pty.send(Msg::Resize(self.screen.window_size()));
        }
    }

    /// Scrolls the view through the scrollback: positive is up.
    pub fn scroll(&self, lines: i32) {
        self.screen.scroll(lines);
    }

    pub fn modes(&self) -> Modes {
        self.screen.modes()
    }

    /// Starts a selection at a cell on screen; `right_half` is which half of the cell was hit.
    pub fn select_from(&self, row: usize, column: usize, right_half: bool, kind: SelectionKind) {
        self.screen.select_from(row, column, right_half, kind);
    }

    pub fn select_to(&self, row: usize, column: usize, right_half: bool) {
        self.screen.select_to(row, column, right_half);
    }

    pub fn clear_selection(&self) {
        self.screen.clear_selection();
    }

    /// The selected text, if anything non-empty is selected.
    pub fn selected_text(&self) -> Option<String> {
        self.screen.selected_text()
    }

    /// What is on screen now.
    pub fn snapshot(&self) -> Snapshot {
        self.screen.snapshot()
    }
}

fn snapshot_from_term(term: &Term<Listener>) -> Snapshot {
        let content = term.renderable_content();
        let (columns, rows) = (term.columns(), term.screen_lines());
        let scrolled = content.display_offset;
        let colours = content.colors;
        let selection = content.selection;
        let blank = Cell {
            character: ' ',
            combining: None,
            foreground: Colour::Foreground,
            background: Colour::Background,
            style: Style::default(),
            wide: false,
            spacer: false,
            selected: false,
        };
        let mut lines = vec![vec![blank; columns]; rows];
        for indexed in content.display_iter {
            let row = indexed.point.line.0 + scrolled as i32;
            let column = indexed.point.column.0;
            if row < 0 || row as usize >= rows || column >= columns {
                continue;
            }
            let cell = &indexed.cell;
            let flags = cell.flags;
            let mut foreground = colour(cell.fg, colours);
            let mut background = colour(cell.bg, colours);
            let mut dim = flags.contains(Flags::DIM);
            if let Color::Named(name) = cell.fg
                && is_dim(name)
            {
                dim = true;
            }
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut foreground, &mut background);
            }
            lines[row as usize][column] = Cell {
                character: cell.c,
                combining: cell.zerowidth().map(|marks| marks.iter().collect()),
                foreground,
                background,
                style: Style {
                    bold: flags.contains(Flags::BOLD),
                    dim,
                    italic: flags.contains(Flags::ITALIC),
                    underline: flags.intersects(Flags::ALL_UNDERLINES),
                    strikethrough: flags.contains(Flags::STRIKEOUT),
                    hidden: flags.contains(Flags::HIDDEN),
                },
                wide: flags.contains(Flags::WIDE_CHAR),
                spacer: flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
                selected: selection.is_some_and(|range| range.contains(indexed.point)),
            };
        }
        let cursor_row = content.cursor.point.line.0 + scrolled as i32;
        let cursor = match content.cursor.shape {
            CursorShape::Hidden => None,
            _ if cursor_row < 0 || cursor_row as usize >= rows => None,
            shape => Some(Cursor {
                row: cursor_row as usize,
                column: content.cursor.point.column.0.min(columns.saturating_sub(1)),
                kind: match shape {
                    CursorShape::Underline => CursorKind::Underline,
                    CursorShape::Beam => CursorKind::Beam,
                    CursorShape::HollowBlock => CursorKind::HollowBlock,
                    _ => CursorKind::Block,
                },
            }),
        };
        Snapshot { columns, rows, lines, cursor, scrolled }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(pty) = &self.pty {
            let _ = pty.send(Msg::Shutdown);
        }
    }
}

fn config() -> Config {
    Config { scrolling_history: SCROLLBACK, ..Config::default() }
}

fn side(right_half: bool) -> Side {
    if right_half { Side::Right } else { Side::Left }
}

/// A cell on screen as a grid point, allowing for how far the view is scrolled.
fn screen_point(term: &Term<Listener>, row: usize, column: usize) -> Point {
    let offset = term.grid().display_offset() as i32;
    let row = row.min(term.screen_lines().saturating_sub(1)) as i32;
    Point::new(Line(row - offset), Column(column.min(term.columns().saturating_sub(1))))
}

fn is_dim(name: NamedColor) -> bool {
    matches!(
        name,
        NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite
            | NamedColor::DimForeground
    )
}

/// Our colour for the emulator's, honouring palette entries a program redefined.
fn colour(colour: Color, overrides: &alacritty_terminal::term::color::Colors) -> Colour {
    let indexed = |index: usize| match overrides[index] {
        Some(rgb) => Colour::Rgb(rgb.r, rgb.g, rgb.b),
        None => Colour::Indexed(index as u8),
    };
    match colour {
        Color::Spec(rgb) => Colour::Rgb(rgb.r, rgb.g, rgb.b),
        Color::Indexed(index) => indexed(index as usize),
        Color::Named(name) => match name {
            NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground | NamedColor::Cursor => {
                Colour::Foreground
            }
            NamedColor::Background => Colour::Background,
            NamedColor::DimBlack => indexed(0),
            NamedColor::DimRed => indexed(1),
            NamedColor::DimGreen => indexed(2),
            NamedColor::DimYellow => indexed(3),
            NamedColor::DimBlue => indexed(4),
            NamedColor::DimMagenta => indexed(5),
            NamedColor::DimCyan => indexed(6),
            NamedColor::DimWhite => indexed(7),
            other => indexed(other as usize),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn text(snapshot: &Snapshot, row: usize) -> String {
        snapshot.lines[row].iter().filter(|cell| !cell.spacer).map(|cell| cell.character).collect::<String>().trim_end().to_string()
    }

    #[test]
    fn replays_text_colours_and_styles() {
        let terminal = Terminal::replay(20, 4, b"plain\r\n\x1b[31mred\x1b[0m \x1b[1;38;2;1;2;3mrgb\x1b[0m\r\n\x1b[7minv\x1b[0m");
        let snapshot = terminal.snapshot();
        assert_eq!((snapshot.columns, snapshot.rows), (20, 4));
        assert_eq!(text(&snapshot, 0), "plain");
        assert_eq!(text(&snapshot, 1), "red rgb");
        assert_eq!(snapshot.lines[1][0].foreground, Colour::Indexed(1));
        assert_eq!(snapshot.lines[1][4].foreground, Colour::Rgb(1, 2, 3));
        assert!(snapshot.lines[1][4].style.bold);
        assert_eq!(snapshot.lines[2][0].background, Colour::Foreground, "inverse swaps the colours");
        assert_eq!(snapshot.cursor.map(|cursor| (cursor.row, cursor.column)), Some((2, 3)));
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let snapshot = Terminal::replay(10, 2, "a中b".as_bytes()).snapshot();
        assert!(snapshot.lines[0][1].wide && snapshot.lines[0][2].spacer);
        assert_eq!(text(&snapshot, 0), "a中b");
    }

    #[test]
    fn scrollback_and_selection() {
        let mut bytes = Vec::new();
        for number in 0..30 {
            bytes.extend(format!("line {number}\r\n").as_bytes());
        }
        let mut terminal = Terminal::replay(12, 5, &bytes);
        assert_eq!(text(&terminal.snapshot(), 3), "line 29");
        terminal.scroll(10);
        let scrolled = terminal.snapshot();
        assert_eq!(scrolled.scrolled, 10);
        assert_eq!(text(&scrolled, 0), "line 16");
        assert!(scrolled.cursor.is_none(), "the cursor is below the view");

        terminal.select_from(0, 0, false, SelectionKind::Characters);
        terminal.select_to(1, 6, true);
        assert_eq!(terminal.selected_text().as_deref(), Some("line 16\nline 17"));
        assert!(terminal.snapshot().lines[0][2].selected);
        terminal.clear_selection();
        assert_eq!(terminal.selected_text(), None);

        terminal.resize(20, 8, (8, 16));
        let resized = terminal.snapshot();
        assert_eq!((resized.columns, resized.rows), (20, 8));
    }

    #[test]
    fn pastes_are_bracketed_when_asked() {
        let terminal = Terminal::replay(10, 2, b"\x1b[?2004h");
        assert!(terminal.modes().bracketed_paste);
        assert!(!Terminal::replay(10, 2, b"").modes().bracketed_paste);
    }

    /// Runs a real shell: `echo` through the PTY comes back on screen.
    #[test]
    fn a_shell_runs_in_the_folder() {
        let folder = tempfile::tempdir().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let terminal = Terminal::spawn(folder.path(), 80, 24, (8, 16), move |event| {
            let _ = sender.send(event);
        })
        .unwrap();
        terminal.write(b"echo wack-$((6*7)); pwd\r".to_vec());
        let deadline = Instant::now() + Duration::from_secs(20);
        let folder_name = folder.path().file_name().unwrap().to_string_lossy().to_string();
        loop {
            let _ = receiver.recv_timeout(Duration::from_millis(200));
            let screen: Vec<String> = {
                let snapshot = terminal.snapshot();
                (0..snapshot.rows).map(|row| text(&snapshot, row)).collect()
            };
            if screen.iter().any(|line| line == "wack-42") && screen.iter().any(|line| line.ends_with(&folder_name)) {
                break;
            }
            assert!(Instant::now() < deadline, "the shell never answered: {screen:#?}");
        }
    }

    #[test]
    fn clear_wipes_viewport_and_scrollback() {
        let mut bytes = Vec::new();
        for number in 0..20 {
            bytes.extend(format!("line {number}\r\n").as_bytes());
        }
        let screen = Screen::replay(20, 4, &bytes);
        screen.scroll(5);
        assert_eq!(screen.snapshot().scrolled, 5);

        screen.clear();
        let after = screen.snapshot();
        assert_eq!(after.scrolled, 0);
        for row in 0..after.rows {
            assert_eq!(text(&after, row), "", "row {row} must be blank after clear");
        }
        screen.advance(b"fresh");
        assert_eq!(text(&screen.snapshot(), 0), "fresh");
    }
}
