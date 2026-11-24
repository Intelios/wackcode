//! Key chords (`"cmd+shift+z"`, `"Enter"`) to macOS virtual key codes, and text chunking for
//! typing. Pure, so it is unit-tested; ported from version 2's `wackdev/src/keys.rs`.
//!
//! Invariants:
//! - Named keys (Enter, arrows, F-keys…) sit in the same place on every layout, so they have
//!   fixed codes. A **character** key (the `s` of ⌘S) is looked up in a [`Layout`]: the user's
//!   current keyboard layout in the app (`native::keyboard`), so a shortcut lands on the key that
//!   types that character there. Version 2 used ANSI-US codes, which pressed the wrong key on
//!   AZERTY or Dvorak. [`Layout::ansi`] is the fallback when the layout can't be read.
//! - Typed text never goes through key codes: it is sent as Unicode strings, so it is
//!   layout-independent.
//! - ⌃⌥⌘. is the user's stop shortcut; no spelling of it parses.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub command: bool,
    pub control: bool,
    pub option: bool,
    pub shift: bool,
    pub function: bool,
}

impl Modifiers {
    pub fn any(&self) -> bool {
        self.command || self.control || self.option || self.shift || self.function
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub key_code: u16,
    pub modifiers: Modifiers,
    /// The key's name as written, for the answer ("Enter", "s").
    pub named: bool,
}

/// The `.` key's code on ANSI; the stop chord is checked by character, not code, too.
const PERIOD_ANSI: u16 = 47;

/// Which key types which character, on one keyboard layout.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    keys: HashMap<char, u16>,
}

impl Layout {
    /// From `(character, key code)` pairs as the layout reports them; the first code for a
    /// character wins (a layout can type one character on two keys).
    pub fn from_pairs(pairs: impl IntoIterator<Item = (char, u16)>) -> Self {
        let mut keys = HashMap::new();
        for (ch, code) in pairs {
            keys.entry(ch.to_lowercase().next().unwrap_or(ch)).or_insert(code);
        }
        Self { keys }
    }

    /// The ANSI US layout.
    pub fn ansi() -> Self {
        Self::from_pairs([
            ('a', 0), ('s', 1), ('d', 2), ('f', 3), ('h', 4), ('g', 5), ('z', 6), ('x', 7), ('c', 8), ('v', 9), ('b', 11),
            ('q', 12), ('w', 13), ('e', 14), ('r', 15), ('y', 16), ('t', 17), ('1', 18), ('2', 19), ('3', 20), ('4', 21),
            ('6', 22), ('5', 23), ('=', 24), ('9', 25), ('7', 26), ('-', 27), ('8', 28), ('0', 29), (']', 30), ('o', 31),
            ('u', 32), ('[', 33), ('i', 34), ('p', 35), ('l', 37), ('j', 38), ('\'', 39), ('k', 40), (';', 41), ('\\', 42),
            (',', 43), ('/', 44), ('n', 45), ('m', 46), ('.', PERIOD_ANSI), ('`', 50), (' ', 49),
        ])
    }

    pub fn code(&self, ch: char) -> Option<u16> {
        self.keys.get(&ch.to_lowercase().next().unwrap_or(ch)).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

fn named_key(name: &str) -> Option<u16> {
    Some(match name {
        "enter" | "return" => 36,
        "tab" => 48,
        "space" | "spacebar" => 49,
        "escape" | "esc" => 53,
        "backspace" | "delete" => 51,
        "forwarddelete" | "forward_delete" | "del" => 117,
        "left" | "arrowleft" => 123,
        "right" | "arrowright" => 124,
        "down" | "arrowdown" => 125,
        "up" | "arrowup" => 126,
        "home" => 115,
        "end" => 119,
        "pageup" => 116,
        "pagedown" => 121,
        "help" => 114,
        "f1" => 122,
        "f2" => 120,
        "f3" => 99,
        "f4" => 118,
        "f5" => 96,
        "f6" => 97,
        "f7" => 98,
        "f8" => 100,
        "f9" => 101,
        "f10" => 109,
        "f11" => 103,
        "f12" => 111,
        _ => return None,
    })
}

fn named_char(name: &str) -> Option<char> {
    Some(match name {
        "plus" => '+',
        "minus" => '-',
        "period" | "dot" => '.',
        "comma" => ',',
        "slash" => '/',
        _ => return None,
    })
}

/// The system key chords no agent may press: the stop shortcut, quitting or force-quitting,
/// switching apps, locking the screen, logging out, and screenshots of the whole screen.
pub fn system_chord(chord: &Chord, ch: Option<char>) -> Option<&'static str> {
    let m = chord.modifiers;
    match ch {
        Some('.') if m.command && m.control && m.option => Some("⌃⌥⌘. is the user's stop shortcut; computer use can't press it."),
        Some('q') if m.command && m.control => Some("⌃⌘Q locks the screen; computer use can't press it."),
        Some('q') if m.command && m.shift => Some("⇧⌘Q logs the user out; computer use can't press it."),
        Some('q') if m.command => Some("⌘Q quits the app: use its menu (File or the app menu › Quit) only if the user asked for it."),
        Some('\u{1b}') if m.command && m.option => Some("⌥⌘Esc is Force Quit; computer use can't press it."),
        _ if chord.key_code == 48 && m.command => Some("⌘Tab switches apps; computer use can't press it."),
        Some(' ') if m.command => Some("⌘Space opens Spotlight; computer use can't press it."),
        Some('3' | '4' | '5') if m.command && m.shift => Some("Screen captures with ⇧⌘3, 4 and 5 are refused."),
        _ => None,
    }
}

/// Parses one key or chord on `layout`. Modifiers and key are joined with `+`; a trailing `++`
/// means the `+` key itself.
pub fn parse_chord(raw: &str, layout: &Layout) -> Result<Chord, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return Err("key needs a key such as \"Enter\" or a chord such as \"cmd+s\".".into());
    }
    let (body, plus_key) = match trimmed.strip_suffix("++") {
        Some(body) => (body, true),
        None if trimmed == "+" => ("", true),
        None => (trimmed, false),
    };
    let mut parts: Vec<&str> = if body.is_empty() { Vec::new() } else { body.split('+').map(str::trim).collect() };
    let key = if plus_key {
        "+"
    } else {
        parts.pop().filter(|part| !part.is_empty()).ok_or_else(|| format!("“{trimmed}” has no key after its modifiers."))?
    };
    let mut modifiers = Modifiers::default();
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" | "super" | "⌘" => modifiers.command = true,
            "ctrl" | "control" | "⌃" => modifiers.control = true,
            "alt" | "option" | "opt" | "⌥" => modifiers.option = true,
            "shift" | "⇧" => modifiers.shift = true,
            "fn" | "function" => modifiers.function = true,
            other => return Err(format!("“{other}” isn't a modifier. Use cmd, ctrl, alt/option, shift or fn.")),
        }
    }
    let lower = key.to_lowercase();
    let mut chars = key.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(ch), None) => Some(ch),
        _ => named_char(&lower),
    };
    let (key_code, named, ch) = if let Some(code) = single.is_none().then(|| named_key(&lower)).flatten() {
        (code, true, if code == 53 { Some('\u{1b}') } else if code == 49 { Some(' ') } else { None })
    } else if let Some(ch) = single {
        // `+` has no key of its own on most layouts: it is shift and `=`.
        let lookup = if ch == '+' && layout.code('+').is_none() {
            modifiers.shift = true;
            '='
        } else {
            ch
        };
        let code = layout.code(lookup).ok_or_else(|| format!("No key types “{ch}” on the current keyboard layout."))?;
        (code, false, Some(lookup.to_lowercase().next().unwrap_or(lookup)))
    } else {
        return Err(format!("“{key}” isn't a key computer use knows. Use a letter, digit, punctuation, or a name like Enter, Tab, Escape, Backspace, Up, F5."));
    };
    let chord = Chord { key_code, modifiers, named };
    if let Some(reason) = system_chord(&chord, ch) {
        return Err(reason.into());
    }
    Ok(chord)
}

/// Splits text into UTF-16 chunks of at most `max` units without splitting a surrogate pair.
pub fn utf16_chunks(text: &str, max: usize) -> Vec<Vec<u16>> {
    let max = max.max(2);
    let mut chunks = Vec::new();
    let mut current: Vec<u16> = Vec::new();
    for ch in text.chars() {
        let mut buffer = [0u16; 2];
        let units = ch.encode_utf16(&mut buffer);
        if current.len() + units.len() > max {
            chunks.push(std::mem::take(&mut current));
        }
        current.extend_from_slice(units);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ansi(raw: &str) -> Result<Chord, String> {
        parse_chord(raw, &Layout::ansi())
    }

    #[test]
    fn parses_named_keys_and_chords() {
        assert_eq!(ansi("Enter").unwrap(), Chord { key_code: 36, modifiers: Modifiers::default(), named: true });
        let chord = ansi("cmd+shift+z").unwrap();
        assert_eq!(chord.key_code, 6);
        assert!(chord.modifiers.command && chord.modifiers.shift && !chord.modifiers.control);
        assert_eq!(ansi("Option+Left").unwrap().key_code, 123);
        assert!(ansi("option+left").unwrap().modifiers.option);
        assert_eq!(ansi("cmd+,").unwrap().key_code, 43);
        assert_eq!(ansi("F5").unwrap().key_code, 96);
        assert_eq!(ansi("space").unwrap().key_code, 49);
        let plus = ansi("cmd++").unwrap();
        assert!(plus.modifiers.command && plus.modifiers.shift && plus.key_code == 24);
        assert_eq!(ansi("cmd+plus").unwrap().key_code, 24);
    }

    #[test]
    fn character_keys_follow_the_layout() {
        // AZERTY puts A where ANSI has Q, and Q where ANSI has A.
        let azerty = Layout::from_pairs([('q', 0), ('a', 12), ('s', 1), ('z', 13), ('w', 6)]);
        assert_eq!(parse_chord("cmd+a", &azerty).unwrap().key_code, 12);
        assert_eq!(parse_chord("cmd+A", &azerty).unwrap().key_code, 12);
        assert_eq!(parse_chord("cmd+z", &azerty).unwrap().key_code, 13);
        // Named keys don't move.
        assert_eq!(parse_chord("Enter", &azerty).unwrap().key_code, 36);
        // A character the layout can't type is refused, not guessed.
        assert!(parse_chord("cmd+ß", &Layout::ansi()).unwrap_err().contains("No key types"));
    }

    #[test]
    fn rejects_unknown_keys_and_modifiers() {
        assert!(ansi("").is_err());
        assert!(ansi("cmd+").is_err());
        assert!(ansi("hyper+a").is_err());
        assert!(ansi("launchpad").is_err());
    }

    #[test]
    fn the_stop_chord_and_system_chords_are_refused_in_any_spelling() {
        assert!(ansi("ctrl+alt+cmd+.").is_err());
        assert!(ansi("Command+Option+Control+.").is_err());
        assert!(ansi("ctrl+alt+cmd+period").is_err());
        assert!(ansi("cmd+.").is_ok());
        for chord in ["cmd+q", "cmd+ctrl+q", "cmd+shift+q", "cmd+tab", "cmd+space", "cmd+shift+4", "cmd+alt+escape"] {
            assert!(ansi(chord).is_err(), "{chord}");
        }
        assert!(ansi("ctrl+tab").is_ok());
        assert!(ansi("cmd+shift+z").is_ok());
    }

    #[test]
    fn chunks_never_split_surrogate_pairs() {
        let text = "ab😀cd😀";
        let chunks = utf16_chunks(text, 3);
        let joined: Vec<u16> = chunks.iter().flatten().copied().collect();
        assert_eq!(String::from_utf16(&joined).unwrap(), text);
        for chunk in &chunks {
            assert!(chunk.len() <= 3);
            assert!(String::from_utf16(chunk).is_ok(), "a chunk split a surrogate pair");
        }
        assert!(utf16_chunks("", 20).is_empty());
    }
}
