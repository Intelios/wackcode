//! A text field's contents and selection, and every edit and movement on them, without drawing.
//!
//! Offsets are UTF-8 byte offsets into the text and always sit on a grapheme boundary, except
//! while an input method is composing (the marked range), where the platform decides. The platform
//! speaks UTF-16; `to_utf16` and `from_utf16` convert at that boundary only.
//!
//! Undo restores whole snapshots. Typing and deleting group into one step until the user pauses
//! the run of the same edit (moves the caret, switches between typing and deleting, or pastes).

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Deleting,
    /// Paste, cut, an input method: always its own undo step.
    Other,
}

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    text: String,
    selection: Range<usize>,
    reversed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Buffer {
    text: String,
    selection: Range<usize>,
    /// The caret is at the selection's start rather than its end.
    reversed: bool,
    /// Text an input method is still composing.
    marked: Option<Range<usize>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The kind of the last edit, while it can still be grouped with the next.
    open_group: Option<EditKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Punctuation,
}

fn class(character: char) -> Class {
    if character.is_whitespace() {
        Class::Space
    } else if character.is_alphanumeric() || character == '_' {
        Class::Word
    } else {
        Class::Punctuation
    }
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        let mut buffer = Self::default();
        buffer.set_text(text);
        buffer
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces everything, with the caret at the end and no undo history.
    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.selection = self.text.len()..self.text.len();
        self.reversed = false;
        self.marked = None;
        self.undo.clear();
        self.redo.clear();
        self.open_group = None;
    }

    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    pub fn reversed(&self) -> bool {
        self.reversed
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// Where the caret is: the moving end of the selection.
    pub fn cursor(&self) -> usize {
        if self.reversed { self.selection.start } else { self.selection.end }
    }

    /// The fixed end of the selection.
    pub fn anchor(&self) -> usize {
        if self.reversed { self.selection.end } else { self.selection.start }
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.clone()]
    }

    // Movement.

    /// Moves the caret to `offset`, extending the selection from the anchor when `select`.
    pub fn move_to(&mut self, offset: usize, select: bool) {
        let offset = offset.min(self.text.len());
        self.open_group = None;
        if select {
            let anchor = self.anchor();
            self.reversed = offset < anchor;
            self.selection = anchor.min(offset)..anchor.max(offset);
        } else {
            self.reversed = false;
            self.selection = offset..offset;
        }
    }

    /// Selects `range` with the caret at its end.
    pub fn select(&mut self, range: Range<usize>) {
        let end = range.end.min(self.text.len());
        self.selection = range.start.min(end)..end;
        self.reversed = false;
        self.open_group = None;
    }

    pub fn select_all(&mut self) {
        self.select(0..self.text.len());
    }

    pub fn previous_grapheme(&self, offset: usize) -> usize {
        self.text[..offset].grapheme_indices(true).next_back().map_or(0, |(index, _)| index)
    }

    pub fn next_grapheme(&self, offset: usize) -> usize {
        self.text[offset..].graphemes(true).next().map_or(self.text.len(), |grapheme| offset + grapheme.len())
    }

    /// The start of the word before `offset`, as ⌥← finds it: skip spaces, then one run of word
    /// characters or of punctuation.
    pub fn previous_word(&self, offset: usize) -> usize {
        let mut characters = self.text[..offset].char_indices().rev().peekable();
        while characters.next_if(|(_, character)| class(*character) == Class::Space).is_some() {}
        let Some(&(_, first)) = characters.peek() else {
            return 0;
        };
        let run = class(first);
        let mut start = offset;
        while let Some((index, _)) = characters.next_if(|(_, character)| class(*character) == run) {
            start = index;
        }
        start.min(offset)
    }

    /// The end of the word after `offset`, as ⌥→ finds it.
    pub fn next_word(&self, offset: usize) -> usize {
        let mut characters = self.text[offset..].char_indices().peekable();
        while characters.next_if(|(_, character)| class(*character) == Class::Space).is_some() {}
        let Some(&(_, first)) = characters.peek() else {
            return self.text.len();
        };
        let run = class(first);
        let mut end = self.text.len();
        for (index, character) in characters {
            if class(character) != run {
                end = offset + index;
                break;
            }
        }
        end
    }

    /// The start of the hard line `offset` is on.
    pub fn line_start(&self, offset: usize) -> usize {
        self.text[..offset].rfind('\n').map_or(0, |index| index + 1)
    }

    /// The end of the hard line `offset` is on, before its line break.
    pub fn line_end(&self, offset: usize) -> usize {
        self.text[offset..].find('\n').map_or(self.text.len(), |index| offset + index)
    }

    /// The word around `offset`, for a double click.
    pub fn word_at(&self, offset: usize) -> Range<usize> {
        let offset = offset.min(self.text.len());
        for (start, word) in self.text.split_word_bound_indices() {
            let end = start + word.len();
            if offset < end || end == self.text.len() {
                return start..end;
            }
        }
        offset..offset
    }

    // Editing.

    /// Replaces `range` (or what an input method is composing, or the selection) with `text`, and
    /// leaves the caret after it.
    pub fn replace(&mut self, range: Option<Range<usize>>, text: &str, kind: EditKind) {
        let range = range.or_else(|| self.marked.clone()).unwrap_or_else(|| self.selection.clone());
        let range = range.start.min(self.text.len())..range.end.min(self.text.len());
        if range.is_empty() && text.is_empty() && self.marked.is_none() {
            return;
        }
        if self.marked.is_some() {
            // Committing a composition: its undo step was saved when it began.
            self.redo.clear();
            self.open_group = None;
        } else {
            self.checkpoint(kind);
        }
        self.text.replace_range(range.clone(), text);
        let caret = range.start + text.len();
        self.selection = caret..caret;
        self.reversed = false;
        self.marked = None;
    }

    /// An input method's composing text: replaces `range` and marks the result, with the
    /// selection placed inside it as the platform asks.
    pub fn replace_and_mark(&mut self, range: Option<Range<usize>>, text: &str, selected_within: Option<Range<usize>>) {
        let range = range.or_else(|| self.marked.clone()).unwrap_or_else(|| self.selection.clone());
        let range = range.start.min(self.text.len())..range.end.min(self.text.len());
        // A composition is one undo step from its first keystroke to its last.
        if self.marked.is_none() {
            self.checkpoint(EditKind::Other);
        }
        self.text.replace_range(range.clone(), text);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        self.selection = match selected_within {
            Some(within) => {
                let end = (range.start + within.end).min(range.start + text.len());
                (range.start + within.start).min(end)..end
            }
            None => range.start + text.len()..range.start + text.len(),
        };
        self.reversed = false;
        self.open_group = None;
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// Deletes the selection, or from the caret back to `to` when nothing is selected.
    pub fn delete_back_to(&mut self, to: usize) -> bool {
        if self.selection.is_empty() {
            let caret = self.cursor();
            if to >= caret {
                return false;
            }
            self.replace(Some(to..caret), "", EditKind::Deleting);
        } else {
            self.replace(None, "", EditKind::Deleting);
        }
        true
    }

    /// Deletes the selection, or from the caret forward to `to` when nothing is selected.
    pub fn delete_forward_to(&mut self, to: usize) -> bool {
        if self.selection.is_empty() {
            let caret = self.cursor();
            if to <= caret {
                return false;
            }
            self.replace(Some(caret..to), "", EditKind::Deleting);
        } else {
            self.replace(None, "", EditKind::Deleting);
        }
        true
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text.clone(), selection: self.selection.clone(), reversed: self.reversed }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.text = snapshot.text;
        self.selection = snapshot.selection;
        self.reversed = snapshot.reversed;
        self.marked = None;
        self.open_group = None;
    }

    /// Saves the state before an edit, unless the edit continues the open group.
    fn checkpoint(&mut self, kind: EditKind) {
        let continues = kind != EditKind::Other && self.open_group == Some(kind);
        if !continues {
            self.undo.push(self.snapshot());
            // Long sessions of typing keep a bounded history.
            if self.undo.len() > 500 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.open_group = (kind != EditKind::Other).then_some(kind);
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(snapshot);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(snapshot);
        true
    }

    // The platform's UTF-16 offsets.

    pub fn to_utf16(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())].encode_utf16().count()
    }

    pub fn from_utf16(&self, offset: usize) -> usize {
        let mut units = 0;
        for (index, character) in self.text.char_indices() {
            if units >= offset {
                return index;
            }
            units += character.len_utf16();
        }
        self.text.len()
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.to_utf16(range.start)..self.to_utf16(range.end)
    }

    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.from_utf16(range.start)..self.from_utf16(range.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Buffer {
        let mut buffer = Buffer::new("");
        for character in text.chars() {
            buffer.replace(None, &character.to_string(), EditKind::Typing);
        }
        buffer
    }

    #[test]
    fn typing_groups_into_one_undo_step_until_the_caret_moves() {
        let mut buffer = typed("hello");
        buffer.move_to(5, false);
        for character in " world".chars() {
            buffer.replace(None, &character.to_string(), EditKind::Typing);
        }
        assert_eq!(buffer.text(), "hello world");
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "hello");
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "");
        assert!(!buffer.undo());
        assert!(buffer.redo());
        assert!(buffer.redo());
        assert_eq!(buffer.text(), "hello world");
        assert_eq!(buffer.cursor(), 11);
    }

    #[test]
    fn switching_from_typing_to_deleting_starts_a_new_step() {
        let mut buffer = typed("abc");
        buffer.delete_back_to(buffer.previous_grapheme(buffer.cursor()));
        buffer.delete_back_to(buffer.previous_grapheme(buffer.cursor()));
        assert_eq!(buffer.text(), "a");
        buffer.undo();
        assert_eq!(buffer.text(), "abc");
    }

    #[test]
    fn graphemes_are_never_split() {
        let buffer = Buffer::new("e\u{301}👍🏽!");
        assert_eq!(buffer.next_grapheme(0), 3);
        assert_eq!(buffer.next_grapheme(3), 11);
        assert_eq!(buffer.previous_grapheme(11), 3);
        assert_eq!(buffer.previous_grapheme(3), 0);
    }

    #[test]
    fn words_move_like_option_arrows() {
        let buffer = Buffer::new("let foo_bar = baz.qux();  ");
        assert_eq!(buffer.next_word(0), 3);
        assert_eq!(buffer.next_word(3), 11);
        assert_eq!(buffer.next_word(11), 13);
        assert_eq!(buffer.previous_word(17), 14);
        assert_eq!(buffer.previous_word(14), 12);
        assert_eq!(buffer.previous_word(26), 21, "`();` is one run of punctuation");
        assert_eq!(buffer.next_word(24), 26);
        assert_eq!(buffer.previous_word(0), 0);
    }

    #[test]
    fn lines_and_words_are_found_around_an_offset() {
        let buffer = Buffer::new("first\nsecond line\nthird");
        assert_eq!(buffer.line_start(9), 6);
        assert_eq!(buffer.line_end(9), 17);
        assert_eq!(buffer.line_end(23), 23);
        assert_eq!(buffer.word_at(8), 6..12);
    }

    #[test]
    fn selecting_backwards_keeps_the_anchor() {
        let mut buffer = Buffer::new("hello world");
        buffer.move_to(6, false);
        buffer.move_to(2, true);
        assert_eq!((buffer.selection(), buffer.reversed(), buffer.cursor()), (2..6, true, 2));
        buffer.move_to(9, true);
        assert_eq!((buffer.selection(), buffer.reversed()), (6..9, false));
        buffer.replace(None, "W", EditKind::Typing);
        assert_eq!(buffer.text(), "hello Wld");
    }

    #[test]
    fn an_input_method_composes_then_commits_as_one_step() {
        let mut buffer = typed("a");
        buffer.replace_and_mark(None, "n", None);
        buffer.replace_and_mark(None, "に", Some(0..3));
        assert_eq!(buffer.marked(), Some(1..4));
        assert_eq!(buffer.selection(), 1..4);
        buffer.replace(None, "日", EditKind::Other);
        assert_eq!(buffer.text(), "a日");
        assert_eq!(buffer.marked(), None);
        buffer.undo();
        assert_eq!(buffer.text(), "a");
    }

    #[test]
    fn utf16_offsets_round_trip() {
        let buffer = Buffer::new("a😀b");
        assert_eq!(buffer.to_utf16(5), 3);
        assert_eq!(buffer.from_utf16(3), 5);
        assert_eq!(buffer.range_from_utf16(&(1..3)), 1..5);
        assert_eq!(buffer.from_utf16(99), 6);
    }

    #[test]
    fn deleting_with_nothing_to_delete_does_nothing() {
        let mut buffer = Buffer::new("x");
        buffer.move_to(0, false);
        assert!(!buffer.delete_back_to(0));
        buffer.move_to(1, false);
        assert!(!buffer.delete_forward_to(1));
        assert_eq!(buffer.text(), "x");
    }
}
