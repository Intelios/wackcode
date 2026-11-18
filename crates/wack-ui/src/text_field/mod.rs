//! A text field. GPUI ships none, so this is WackCode's own.
//!
//! [`edit::Buffer`] holds the text and selection and does every edit; this module draws it, wraps
//! it, moves the caret by what is on screen (↑↓ and ⌘←→ follow wrapped rows), and speaks to macOS
//! through GPUI's input handler, which is what makes input methods, the character palette and
//! dictation work.
//!
//! The owner decides what Return means: the field emits [`TextFieldEvent::Enter`] and never
//! inserts a line break for it. ⇧↩ and ⌥↩ insert one in a multi-line field. A masked field draws a
//! bullet per character and never copies or hands its text to the platform; its owner can show it
//! plainly for a look and mask it again ([`TextField::set_masked`]).
//!
//! A list can borrow the field's keys while it is open over it (the composer's `/` menu): with
//! [`TextField::set_completing`] on, ↑ ↓ ⇥ ↩ ⎋ become [`TextFieldEvent::Menu`] for the owner,
//! through the `TextField && completing` key context. [`TextField::set_highlight`] draws a
//! range in the accent colour (a recognised `/command`).

pub mod edit;

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, ContentMask, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, Font, FontStyle, GlobalElementId, Hsla, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent,
    SharedString, Size, Style, TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine, actions, div,
    fill, point, prelude::*, px, relative, size,
};

pub use edit::{Buffer, EditKind};

use crate::{theme::Theme, typography::Type};

actions!(
    text_field,
    [
        Backspace,
        DeleteWordBack,
        DeleteLineBack,
        Delete,
        DeleteWordForward,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        LineStart,
        LineEnd,
        DocStart,
        DocEnd,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectLineStart,
        SelectLineEnd,
        SelectDocStart,
        SelectDocEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        Enter,
        NewLine,
        SendNow,
        CharacterPalette,
        MenuUp,
        MenuDown,
        MenuPick,
        MenuDismiss,
    ]
);

const CONTEXT: &str = "TextField";
const COMPLETING: &str = "completing";
const COMPLETING_PREDICATE: &str = "TextField && completing";
const BULLET: char = '•';

/// The field's key bindings. Called once at start.
pub fn init(cx: &mut App) {
    let context = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("shift-backspace", Backspace, context),
        KeyBinding::new("alt-backspace", DeleteWordBack, context),
        KeyBinding::new("cmd-backspace", DeleteLineBack, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("ctrl-d", Delete, context),
        KeyBinding::new("alt-delete", DeleteWordForward, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("alt-left", WordLeft, context),
        KeyBinding::new("alt-right", WordRight, context),
        KeyBinding::new("cmd-left", LineStart, context),
        KeyBinding::new("cmd-right", LineEnd, context),
        KeyBinding::new("ctrl-a", LineStart, context),
        KeyBinding::new("ctrl-e", LineEnd, context),
        KeyBinding::new("home", LineStart, context),
        KeyBinding::new("end", LineEnd, context),
        KeyBinding::new("cmd-up", DocStart, context),
        KeyBinding::new("cmd-down", DocEnd, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("alt-shift-left", SelectWordLeft, context),
        KeyBinding::new("alt-shift-right", SelectWordRight, context),
        KeyBinding::new("cmd-shift-left", SelectLineStart, context),
        KeyBinding::new("cmd-shift-right", SelectLineEnd, context),
        KeyBinding::new("shift-home", SelectLineStart, context),
        KeyBinding::new("shift-end", SelectLineEnd, context),
        KeyBinding::new("cmd-shift-up", SelectDocStart, context),
        KeyBinding::new("cmd-shift-down", SelectDocEnd, context),
        KeyBinding::new("cmd-a", SelectAll, context),
        KeyBinding::new("cmd-c", Copy, context),
        KeyBinding::new("cmd-x", Cut, context),
        KeyBinding::new("cmd-v", Paste, context),
        KeyBinding::new("cmd-z", Undo, context),
        KeyBinding::new("cmd-shift-z", Redo, context),
        KeyBinding::new("enter", Enter, context),
        KeyBinding::new("shift-enter", NewLine, context),
        KeyBinding::new("alt-enter", NewLine, context),
        KeyBinding::new("cmd-enter", SendNow, context),
        KeyBinding::new("ctrl-cmd-space", CharacterPalette, context),
    ]);
    // While a list is open over the field, these keys are the list's.
    let completing = Some(COMPLETING_PREDICATE);
    cx.bind_keys([
        KeyBinding::new("up", MenuUp, completing),
        KeyBinding::new("ctrl-p", MenuUp, completing),
        KeyBinding::new("down", MenuDown, completing),
        KeyBinding::new("ctrl-n", MenuDown, completing),
        KeyBinding::new("tab", MenuPick, completing),
        KeyBinding::new("enter", MenuPick, completing),
        KeyBinding::new("escape", MenuDismiss, completing),
    ]);
}

/// The key a list open over the field takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    Up,
    Down,
    Pick,
    Dismiss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFieldEvent {
    Changed,
    /// Return, without ⇧ or ⌥.
    Enter,
    /// ⌘↩.
    SendNow,
    /// The caret moved without the text changing.
    Moved,
    /// A list's key, while [`TextField::set_completing`] is on.
    Menu(MenuKey),
    /// Pictures or files were pasted into a field that takes them ([`TextField::takes_pictures`]):
    /// collect them with [`TextField::take_pasted`].
    PastedPictures,
}

/// Something pasted into a field that takes pictures.
#[derive(Clone, Debug, PartialEq)]
pub enum Pasted {
    /// A picture's bytes, as the clipboard held it.
    Picture(Vec<u8>),
    /// A file copied in the Finder.
    File(std::path::PathBuf),
}

/// What the last frame drew, for turning positions into offsets and back.
struct Drawn {
    /// One per hard line, in display offsets.
    lines: Vec<WrappedLine>,
    starts: Vec<usize>,
    /// Where the first line's top-left was drawn, scroll included.
    origin: Point<Pixels>,
    line_height: Pixels,
    content_height: Pixels,
    visible_height: Pixels,
}

pub struct TextField {
    focus: FocusHandle,
    buffer: Buffer,
    placeholder: SharedString,
    kind: Type,
    multiline: bool,
    masked: bool,
    /// Rows shown before the field scrolls.
    max_rows: usize,
    drawn: Option<Drawn>,
    scroll: Pixels,
    /// Bring the caret into view on the next frame.
    reveal: bool,
    selecting: bool,
    /// The x the caret keeps while moving up and down through shorter rows.
    goal_x: Option<Pixels>,
    /// A list open over the field has its keys.
    completing: bool,
    /// Drawn in the accent colour.
    highlight: Option<Range<usize>>,
    /// Pastes pictures and files instead of ignoring them.
    pictures: bool,
    pasted: Vec<Pasted>,
}

impl EventEmitter<TextFieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextField {
    pub fn new(kind: Type, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle().tab_stop(true),
            buffer: Buffer::default(),
            placeholder: placeholder.into(),
            kind,
            multiline: false,
            masked: false,
            max_rows: 1,
            drawn: None,
            scroll: px(0.),
            reveal: false,
            selecting: false,
            goal_x: None,
            completing: false,
            highlight: None,
            pictures: false,
            pasted: Vec::new(),
        }
    }

    /// Takes pasted pictures and files: a paste with no text holds them for
    /// [`TextField::take_pasted`] and emits [`TextFieldEvent::PastedPictures`].
    pub fn takes_pictures(mut self) -> Self {
        self.pictures = true;
        self
    }

    /// What was pasted since the last call.
    pub fn take_pasted(&mut self) -> Vec<Pasted> {
        std::mem::take(&mut self.pasted)
    }

    /// Lets the text run over several rows, scrolling past `max_rows`.
    pub fn multiline(mut self, max_rows: usize) -> Self {
        self.multiline = true;
        self.max_rows = max_rows.max(1);
        self
    }

    /// For secrets: bullets on screen, no copying.
    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    /// Whether the text is drawn as bullets.
    pub fn is_masked(&self) -> bool {
        self.masked
    }

    /// Shows the text as bullets, or plainly again: the owner's choice, as at a key field's eye.
    pub fn set_masked(&mut self, masked: bool, cx: &mut Context<Self>) {
        if self.masked != masked {
            self.masked = masked;
            // Every glyph changes width, so the caret must be brought back into view.
            self.reveal = true;
            cx.notify();
        }
    }

    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.buffer.set_text(text);
        self.reveal = true;
        cx.emit(TextFieldEvent::Changed);
        cx.notify();
    }

    /// Sets the text with the caret at its start, so a long text opens at its top.
    pub fn set_text_at_start(&mut self, text: &str, cx: &mut Context<Self>) {
        self.set_text(text, cx);
        self.buffer.move_to(0, false);
    }

    /// A line break at the caret, for an owner whose Return means "new line" (a note's body).
    pub fn insert_line_break(&mut self, cx: &mut Context<Self>) {
        if self.multiline {
            self.insert("\n", EditKind::Typing, cx);
        }
    }

    /// Where the caret is, as a byte offset into [`TextField::text`].
    pub fn caret(&self) -> usize {
        self.buffer.cursor()
    }

    /// Replaces the whole text as one undo step and puts the caret at `caret`.
    pub fn replace_text(&mut self, text: &str, caret: usize, cx: &mut Context<Self>) {
        let length = self.buffer.text().len();
        self.buffer.replace(Some(0..length), text, EditKind::Other);
        self.buffer.move_to(caret.min(text.len()), false);
        self.changed(cx);
    }

    /// Lends ↑ ↓ ⇥ ↩ ⎋ to a list open over the field.
    pub fn set_completing(&mut self, completing: bool, cx: &mut Context<Self>) {
        if self.completing != completing {
            self.completing = completing;
            cx.notify();
        }
    }

    /// Draws `range` of the text in the accent colour; `None` draws it all plain.
    pub fn set_highlight(&mut self, range: Option<Range<usize>>, cx: &mut Context<Self>) {
        if self.highlight != range {
            self.highlight = range;
            cx.notify();
        }
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    /// Selects the whole text, so typing replaces it (a rename field opening on the old name).
    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.buffer.select_all();
        self.moved(cx);
    }

    // What is shown, which differs from the text only when masked.

    fn display_text(&self) -> String {
        if self.masked {
            BULLET.to_string().repeat(self.buffer.text().chars().count())
        } else {
            self.buffer.text().to_string()
        }
    }

    fn display_offset(&self, offset: usize) -> usize {
        if self.masked { self.buffer.text()[..offset].chars().count() * BULLET.len_utf8() } else { offset }
    }

    fn real_offset(&self, offset: usize) -> usize {
        if !self.masked {
            return offset.min(self.buffer.text().len());
        }
        let characters = offset / BULLET.len_utf8();
        self.buffer.text().char_indices().nth(characters).map_or(self.buffer.text().len(), |(index, _)| index)
    }

    // Geometry from the last frame.

    /// Where `offset` sits, relative to the first line's top-left.
    fn locate(&self, offset: usize) -> Option<Point<Pixels>> {
        let drawn = self.drawn.as_ref()?;
        let display = self.display_offset(offset);
        let mut top = px(0.);
        for (index, line) in drawn.lines.iter().enumerate() {
            let start = drawn.starts[index];
            let end = start + line.len();
            if display <= end {
                let position = line.position_for_index(display - start, drawn.line_height)?;
                return Some(point(position.x, top + position.y));
            }
            top += line.size(drawn.line_height).height;
        }
        None
    }

    /// The offset closest to `position`, relative to the first line's top-left.
    fn offset_at(&self, position: Point<Pixels>) -> usize {
        let Some(drawn) = self.drawn.as_ref() else {
            return self.buffer.text().len();
        };
        if position.y < px(0.) {
            return 0;
        }
        let mut top = px(0.);
        for (index, line) in drawn.lines.iter().enumerate() {
            let height = line.size(drawn.line_height).height;
            if position.y < top + height || index + 1 == drawn.lines.len() {
                let within = point(position.x.max(px(0.)), (position.y - top).min(height - px(1.)));
                let local = match line.closest_index_for_position(within, drawn.line_height) {
                    Ok(index) | Err(index) => index,
                };
                return self.real_offset(drawn.starts[index] + local);
            }
            top += height;
        }
        self.buffer.text().len()
    }

    fn offset_for_window_point(&self, position: Point<Pixels>) -> usize {
        match &self.drawn {
            Some(drawn) => self.offset_at(position - drawn.origin),
            None => self.buffer.text().len(),
        }
    }

    // Movement and editing, shared by the actions.

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.reveal = true;
        self.goal_x = None;
        cx.emit(TextFieldEvent::Changed);
        cx.notify();
    }

    fn moved(&mut self, cx: &mut Context<Self>) {
        self.reveal = true;
        cx.emit(TextFieldEvent::Moved);
        cx.notify();
    }

    fn move_to(&mut self, offset: usize, select: bool, cx: &mut Context<Self>) {
        self.buffer.move_to(offset, select);
        self.goal_x = None;
        self.moved(cx);
    }

    /// One row up or down from the caret, keeping its column. Past the first or last row the
    /// caret goes to the start or end, as macOS fields do.
    fn vertical(&mut self, rows: f32, select: bool, cx: &mut Context<Self>) {
        let caret = self.buffer.cursor();
        let target = match (self.locate(caret), self.drawn.as_ref()) {
            (Some(position), Some(drawn)) => {
                let x = *self.goal_x.get_or_insert(position.x);
                let y = position.y + drawn.line_height * rows + drawn.line_height / 2.;
                if y < px(0.) {
                    0
                } else if y >= drawn.content_height {
                    self.buffer.text().len()
                } else {
                    self.offset_at(point(x, y))
                }
            }
            _ if rows < 0. => 0,
            _ => self.buffer.text().len(),
        };
        let goal = self.goal_x;
        self.buffer.move_to(target, select);
        self.goal_x = goal;
        self.moved(cx);
    }

    /// The start or end of the row the caret is on, as drawn.
    fn row_edge(&self, end: bool) -> usize {
        let caret = self.buffer.cursor();
        match (self.locate(caret), self.drawn.as_ref()) {
            (Some(position), Some(_)) => {
                let x = if end { px(1.0e6) } else { px(0.) };
                self.offset_at(point(x, position.y + px(1.)))
            }
            _ if end => self.buffer.line_end(caret),
            _ => self.buffer.line_start(caret),
        }
    }

    fn insert(&mut self, text: &str, kind: EditKind, cx: &mut Context<Self>) {
        self.buffer.replace(None, text, kind);
        self.changed(cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.buffer.previous_grapheme(self.buffer.cursor());
        if self.buffer.delete_back_to(to) {
            self.changed(cx);
        }
    }

    fn delete_word_back(&mut self, _: &DeleteWordBack, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.buffer.previous_word(self.buffer.cursor());
        if self.buffer.delete_back_to(to) {
            self.changed(cx);
        }
    }

    fn delete_line_back(&mut self, _: &DeleteLineBack, _: &mut Window, cx: &mut Context<Self>) {
        let caret = self.buffer.cursor();
        let start = self.row_edge(false);
        // At the very start of a row, ⌘⌫ joins it to the row above.
        let to = if start == caret { self.buffer.previous_grapheme(caret) } else { start };
        if self.buffer.delete_back_to(to) {
            self.changed(cx);
        }
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.buffer.next_grapheme(self.buffer.cursor());
        if self.buffer.delete_forward_to(to) {
            self.changed(cx);
        }
    }

    fn delete_word_forward(&mut self, _: &DeleteWordForward, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.buffer.next_word(self.buffer.cursor());
        if self.buffer.delete_forward_to(to) {
            self.changed(cx);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let selection = self.buffer.selection();
        let to = if selection.is_empty() { self.buffer.previous_grapheme(selection.start) } else { selection.start };
        self.move_to(to, false, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let selection = self.buffer.selection();
        let to = if selection.is_empty() { self.buffer.next_grapheme(selection.end) } else { selection.end };
        self.move_to(to, false, cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1., false, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1., false, cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.previous_word(self.buffer.cursor()), false, cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.next_word(self.buffer.cursor()), false, cx);
    }

    fn line_start(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_edge(false), false, cx);
    }

    fn line_end(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_edge(true), false, cx);
    }

    fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, false, cx);
    }

    fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.text().len(), false, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.previous_grapheme(self.buffer.cursor()), true, cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.next_grapheme(self.buffer.cursor()), true, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1., true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1., true, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.previous_word(self.buffer.cursor()), true, cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.next_word(self.buffer.cursor()), true, cx);
    }

    fn select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_edge(false), true, cx);
    }

    fn select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_edge(true), true, cx);
    }

    fn select_doc_start(&mut self, _: &SelectDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, true, cx);
    }

    fn select_doc_end(&mut self, _: &SelectDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.text().len(), true, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.buffer.select_all();
        self.moved(cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.masked && !self.buffer.selection().is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buffer.selected_text().to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.masked && !self.buffer.selection().is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buffer.selected_text().to_string()));
            self.insert("", EditKind::Other, cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        // Files copied in the Finder come as paths (their names ride along as text). Otherwise text
        // wins: a copy from a web page or an editor often carries a picture of itself too.
        let text = item.text();
        if self.pictures {
            let files: Vec<Pasted> = item
                .entries()
                .iter()
                .filter_map(|entry| match entry {
                    gpui::ClipboardEntry::ExternalPaths(paths) => Some(paths.paths().iter().cloned().map(Pasted::File)),
                    _ => None,
                })
                .flatten()
                .collect();
            let pictures: Vec<Pasted> = item
                .entries()
                .iter()
                .filter_map(|entry| match entry {
                    gpui::ClipboardEntry::Image(image) => Some(Pasted::Picture(image.bytes.clone())),
                    _ => None,
                })
                .collect();
            let no_text = text.as_deref().is_none_or(|text| text.trim().is_empty());
            let pasted = if !files.is_empty() { files } else if no_text { pictures } else { Vec::new() };
            if !pasted.is_empty() {
                self.pasted.extend(pasted);
                cx.emit(TextFieldEvent::PastedPictures);
                return;
            }
        }
        let Some(text) = text else {
            return;
        };
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let text = if self.multiline { text } else { text.replace('\n', " ") };
        self.insert(&text, EditKind::Other, cx);
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.buffer.undo() {
            self.changed(cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.buffer.redo() {
            self.changed(cx);
        }
    }

    fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextFieldEvent::Enter);
    }

    fn new_line(&mut self, _: &NewLine, _: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.insert("\n", EditKind::Typing, cx);
        }
    }

    fn send_now(&mut self, _: &SendNow, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextFieldEvent::SendNow);
    }

    fn menu_up(&mut self, _: &MenuUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextFieldEvent::Menu(MenuKey::Up));
    }

    fn menu_down(&mut self, _: &MenuDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextFieldEvent::Menu(MenuKey::Down));
    }

    fn menu_pick(&mut self, _: &MenuPick, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextFieldEvent::Menu(MenuKey::Pick));
    }

    fn menu_dismiss(&mut self, _: &MenuDismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextFieldEvent::Menu(MenuKey::Dismiss));
    }

    fn character_palette(&mut self, _: &CharacterPalette, window: &mut Window, _: &mut Context<Self>) {
        window.show_character_palette();
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let offset = self.offset_for_window_point(event.position);
        match event.click_count {
            2 => self.buffer.select(self.buffer.word_at(offset)),
            3 => self.buffer.select(self.buffer.line_start(offset)..self.buffer.line_end(offset)),
            _ => self.buffer.move_to(offset, event.modifiers.shift),
        }
        self.selecting = true;
        self.goal_x = None;
        self.moved(cx);
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            let offset = self.offset_for_window_point(event.position);
            self.move_to(offset, true, cx);
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drawn) = &self.drawn else {
            return;
        };
        let room = (drawn.content_height - drawn.visible_height).max(px(0.));
        if room <= px(0.) {
            return;
        }
        let delta = event.delta.pixel_delta(drawn.line_height).y;
        self.scroll = (self.scroll - delta).clamp(px(0.), room);
        cx.stop_propagation();
        cx.notify();
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.buffer.range_from_utf16(&range);
        adjusted.replace(self.buffer.range_to_utf16(&range));
        if self.masked {
            return Some(BULLET.to_string().repeat(self.buffer.text()[range].chars().count()));
        }
        Some(self.buffer.text()[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.buffer.range_to_utf16(&self.buffer.selection()),
            reversed: self.buffer.reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.buffer.marked().map(|range| self.buffer.range_to_utf16(&range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.buffer.unmark();
        cx.notify();
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range = range.map(|range| self.buffer.range_from_utf16(&range));
        let text = if self.multiline { text.to_string() } else { text.replace('\n', " ") };
        // Plain keystrokes arrive here one character at a time: they group as typing.
        let kind = if text.chars().count() <= 1 && self.buffer.marked().is_none() { EditKind::Typing } else { EditKind::Other };
        self.buffer.replace(range, &text, kind);
        self.changed(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.map(|range| self.buffer.range_from_utf16(&range));
        let selected = selected.map(|selected| {
            let within = Buffer::new(text);
            within.range_from_utf16(&selected)
        });
        self.buffer.replace_and_mark(range, text, selected);
        self.changed(cx);
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let start = self.buffer.from_utf16(range.start);
        let position = self.locate(start)?;
        let drawn = self.drawn.as_ref()?;
        Some(Bounds::new(drawn.origin + position, size(px(2.), drawn.line_height)))
    }

    fn character_index_for_point(&mut self, position: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let offset = self.offset_for_window_point(position);
        Some(self.buffer.to_utf16(offset))
    }
}

impl Render for TextField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut context = gpui::KeyContext::new_with_defaults();
        context.add(CONTEXT);
        if self.completing {
            context.add(COMPLETING);
        }
        div()
            .id("text-field")
            .key_context(context)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .w_full()
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete_word_back))
            .on_action(cx.listener(Self::delete_line_back))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_forward))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::line_start))
            .on_action(cx.listener(Self::line_end))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::select_doc_start))
            .on_action(cx.listener(Self::select_doc_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::new_line))
            .on_action(cx.listener(Self::send_now))
            .on_action(cx.listener(Self::character_palette))
            .on_action(cx.listener(Self::menu_up))
            .on_action(cx.listener(Self::menu_down))
            .on_action(cx.listener(Self::menu_pick))
            .on_action(cx.listener(Self::menu_dismiss))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .child(FieldElement { field: cx.entity() })
    }
}

/// Everything needed to shape the field's text at some width.
#[derive(Clone)]
struct Shaping {
    text: SharedString,
    runs: Vec<TextRun>,
    font_size: Pixels,
    line_height: Pixels,
    max_rows: usize,
}

impl Shaping {
    fn shape(&self, width: Pixels, window: &Window) -> Vec<WrappedLine> {
        window
            .text_system()
            .shape_text(self.text.clone(), self.font_size, &self.runs, Some(width), None)
            .map(|lines| lines.into_iter().collect())
            .unwrap_or_default()
    }

    fn height(&self, lines: &[WrappedLine]) -> Pixels {
        lines.iter().map(|line| line.size(self.line_height).height).fold(px(0.), |total, height| total + height)
    }
}

struct FieldElement {
    field: Entity<TextField>,
}

struct Prepainted {
    lines: Vec<WrappedLine>,
    placeholder: bool,
    selections: Vec<Bounds<Pixels>>,
    caret: Option<Bounds<Pixels>>,
    selection_colour: Hsla,
    caret_colour: Hsla,
}

impl IntoElement for FieldElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

fn shaping(field: &TextField, theme: &Theme) -> Shaping {
    let style = field.kind.style();
    let font = Font {
        family: style.family.into(),
        features: Default::default(),
        fallbacks: None,
        weight: style.weight,
        style: if style.italic { FontStyle::Italic } else { FontStyle::Normal },
    };
    let display = field.display_text();
    let placeholder = display.is_empty();
    let text: SharedString = if placeholder { field.placeholder.clone() } else { display.into() };
    let colour = if placeholder { theme.text_faint } else { theme.text };
    let run = |len: usize, underline: bool, lit: bool| {
        let color = if lit { theme.accent_text } else { colour };
        TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: underline.then_some(UnderlineStyle { color: Some(color), thickness: px(1.), wavy: false }),
            strikethrough: None,
        }
    };
    // Runs change wherever composing text (underlined) or the highlight (lit) starts or ends.
    let marked = field.buffer.marked().filter(|_| !placeholder).map(|marked| field.display_offset(marked.start)..field.display_offset(marked.end));
    let lit = field
        .highlight
        .clone()
        .filter(|range| !placeholder && !field.masked && range.end <= field.buffer.text().len())
        .map(|range| field.display_offset(range.start)..field.display_offset(range.end));
    let mut edges = vec![0, text.len()];
    for range in marked.iter().chain(lit.iter()) {
        edges.extend([range.start.min(text.len()), range.end.min(text.len())]);
    }
    edges.sort_unstable();
    edges.dedup();
    let runs = edges
        .windows(2)
        .filter(|edge| edge[1] > edge[0])
        .map(|edge| {
            let within = |range: &Option<Range<usize>>| range.as_ref().is_some_and(|range| range.start <= edge[0] && edge[1] <= range.end);
            run(edge[1] - edge[0], within(&marked), within(&lit))
        })
        .collect::<Vec<_>>();
    let runs = if runs.is_empty() { vec![run(text.len(), false, false)] } else { runs };
    Shaping {
        text,
        runs,
        font_size: px(style.size),
        line_height: px(style.line_height),
        max_rows: if field.multiline { field.max_rows } else { 1 },
    }
}

impl Element for FieldElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let shaping = shaping(self.field.read(cx), Theme::get(cx));
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let layout = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known.width.or(match available.width {
                gpui::AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let rows_height = match width {
                Some(width) => shaping.height(&shaping.shape(width, window)),
                None => shaping.line_height,
            };
            let height = rows_height.clamp(shaping.line_height, shaping.line_height * shaping.max_rows as f32);
            Size { width: width.unwrap_or(px(0.)), height }
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepainted {
        let theme = Theme::get(cx).clone();
        let field = self.field.read(cx);
        let shaping = shaping(field, &theme);
        let lines = shaping.shape(bounds.size.width, window);
        let placeholder = field.buffer.text().is_empty();
        let line_height = shaping.line_height;
        let content_height = shaping.height(&lines).max(line_height);
        let visible_height = bounds.size.height;

        let mut starts = Vec::with_capacity(lines.len());
        let mut start = 0;
        for line in &lines {
            starts.push(start);
            start += line.len() + 1;
        }

        // Keep the caret in view after it moved; otherwise keep the user's scroll.
        let room = (content_height - visible_height).max(px(0.));
        let mut scroll = field.scroll.min(room);
        let reveal = field.reveal;
        let selection = field.buffer.selection();
        let cursor = field.buffer.cursor();
        let display_cursor = field.display_offset(cursor);
        let display_selection = field.display_offset(selection.start)..field.display_offset(selection.end);

        let locate = |display: usize| -> Point<Pixels> {
            let mut top = px(0.);
            for (index, line) in lines.iter().enumerate() {
                let end = starts[index] + line.len();
                if display <= end {
                    let position = line.position_for_index(display - starts[index], line_height).unwrap_or_default();
                    return point(position.x, top + position.y);
                }
                top += line.size(line_height).height;
            }
            point(px(0.), top)
        };
        let caret_at = if placeholder { point(px(0.), px(0.)) } else { locate(display_cursor) };
        if reveal {
            if caret_at.y < scroll {
                scroll = caret_at.y;
            } else if caret_at.y + line_height > scroll + visible_height {
                scroll = caret_at.y + line_height - visible_height;
            }
        }
        let origin = point(bounds.origin.x, bounds.origin.y - scroll);

        let mut selections = Vec::new();
        if !selection.is_empty() && !placeholder {
            let from = locate(display_selection.start);
            let to = locate(display_selection.end);
            let width = bounds.size.width;
            let row = |y: Pixels, left: Pixels, right: Pixels| {
                Bounds::from_corners(point(origin.x + left, origin.y + y), point(origin.x + right, origin.y + y + line_height))
            };
            if from.y == to.y {
                selections.push(row(from.y, from.x, to.x));
            } else {
                selections.push(row(from.y, from.x, width));
                let mut y = from.y + line_height;
                while y < to.y {
                    selections.push(row(y, px(0.), width));
                    y += line_height;
                }
                selections.push(row(to.y, px(0.), to.x.max(px(4.))));
            }
        }
        let caret = selection
            .is_empty()
            .then(|| Bounds::new(point(origin.x + caret_at.x, origin.y + caret_at.y), size(px(2.), line_height)));

        let drawn = Drawn {
            // Shaped again for the field's own queries: the second call is served from GPUI's
            // per-frame line cache, and `WrappedLine` cannot be cloned.
            lines: if placeholder { Vec::new() } else { shaping.shape(bounds.size.width, window) },
            starts,
            origin,
            line_height,
            content_height: if placeholder { line_height } else { content_height },
            visible_height,
        };
        self.field.update(cx, |field, _| {
            field.scroll = scroll;
            field.reveal = false;
            field.drawn = Some(drawn);
        });

        Prepainted {
            lines,
            placeholder,
            selections,
            caret,
            selection_colour: theme.selection,
            caret_colour: theme.accent_text,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepainted: &mut Prepainted,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.field.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.field.clone()), cx);
        let (origin, line_height) = {
            let field = self.field.read(cx);
            let drawn = field.drawn.as_ref();
            (drawn.map_or(bounds.origin, |drawn| drawn.origin), drawn.map_or(px(18.), |drawn| drawn.line_height))
        };
        let focused = focus.is_focused(window);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if focused {
                for selection in prepainted.selections.drain(..) {
                    window.paint_quad(fill(selection, prepainted.selection_colour));
                }
            }
            let mut top = origin.y;
            for line in &prepainted.lines {
                let _ = line.paint(point(origin.x, top), line_height, TextAlign::Left, None, window, cx);
                top += line.size(line_height).height;
            }
            if focused && let Some(caret) = prepainted.caret.take() {
                window.paint_quad(fill(caret, prepainted.caret_colour));
            }
        });
        let _ = prepainted.placeholder;
    }
}
