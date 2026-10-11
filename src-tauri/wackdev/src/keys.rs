//! Key chords (`"cmd+shift+z"`, `"Enter"`) to macOS virtual key codes, and text chunking for
//! typing. Pure, so it is unit-tested.
//!
//! Key codes are the ANSI US layout's. Typed text never goes through them — it is sent as
//! Unicode strings, so it is layout-independent — but a shortcut letter on another layout may
//! land on a different key (see `UCKeyTranslate` in the Phase 2 notes).

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub command: bool,
    pub control: bool,
    pub option: bool,
    pub shift: bool,
    pub function: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub key_code: u16,
    pub modifiers: Modifiers,
}

/// ⌃⌥⌘. stops computer use. The agent may never press it.
pub const STOP_KEY_CODE: u16 = 47;

pub fn is_stop_chord(chord: &Chord) -> bool {
    chord.key_code == STOP_KEY_CODE && chord.modifiers.command && chord.modifiers.control && chord.modifiers.option
}

fn named_key(name: &str) -> Option<u16> {
    Some(match name {
        "enter" | "return" => 36,
        "tab" => 48,
        "space" | "spacebar" => 49,
        "escape" | "esc" => 53,
        "backspace" | "delete" => 51,
        "forwarddelete" | "del" => 117,
        "left" | "arrowleft" => 123,
        "right" | "arrowright" => 124,
        "down" | "arrowdown" => 125,
        "up" | "arrowup" => 126,
        "home" => 115,
        "end" => 119,
        "pageup" => 116,
        "pagedown" => 121,
        "help" => 114,
        "plus" => 24,
        "minus" => 27,
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

fn char_key(ch: char) -> Option<u16> {
    Some(match ch.to_ascii_lowercase() {
        'a' => 0, 's' => 1, 'd' => 2, 'f' => 3, 'h' => 4, 'g' => 5, 'z' => 6, 'x' => 7, 'c' => 8,
        'v' => 9, 'b' => 11, 'q' => 12, 'w' => 13, 'e' => 14, 'r' => 15, 'y' => 16, 't' => 17,
        '1' => 18, '2' => 19, '3' => 20, '4' => 21, '6' => 22, '5' => 23, '=' => 24, '9' => 25,
        '7' => 26, '-' => 27, '8' => 28, '0' => 29, ']' => 30, 'o' => 31, 'u' => 32, '[' => 33,
        'i' => 34, 'p' => 35, 'l' => 37, 'j' => 38, '\'' => 39, 'k' => 40, ';' => 41, '\\' => 42,
        ',' => 43, '/' => 44, 'n' => 45, 'm' => 46, '.' => 47, '`' => 50, ' ' => 49,
        _ => return None,
    })
}

/// Parses one key or chord. Modifiers and key are joined with `+`; a trailing `++` means the
/// `+` key itself (shift+=).
pub fn parse_chord(raw: &str) -> Result<Chord, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return Err("keypress needs a key such as \"Enter\" or a chord such as \"cmd+s\".".into());
    }
    let (body, plus_key) = match trimmed.strip_suffix("++") {
        Some(body) => (body, true),
        None if trimmed == "+" => ("", true),
        None => (trimmed, false),
    };
    let mut parts: Vec<&str> = if body.is_empty() { Vec::new() } else { body.split('+').map(str::trim).collect() };
    let key = if plus_key {
        None
    } else {
        Some(parts.pop().filter(|part| !part.is_empty()).ok_or_else(|| format!("“{trimmed}” has no key after its modifiers."))?)
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
    let key_code = match key {
        None => {
            modifiers.shift = true;
            24
        }
        Some(key) => {
            let lower = key.to_ascii_lowercase();
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => char_key(ch),
                _ => named_key(&lower),
            }
            .ok_or_else(|| format!("“{key}” isn't a key computer use knows. Use a letter, digit, punctuation, or a name like Enter, Tab, Escape, Backspace, Up, F5."))?
        }
    };
    let chord = Chord { key_code, modifiers };
    if is_stop_chord(&chord) {
        return Err("⌃⌥⌘. is the user's stop shortcut; computer use can't press it.".into());
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

    #[test]
    fn parses_named_keys_and_chords() {
        assert_eq!(parse_chord("Enter").unwrap(), Chord { key_code: 36, modifiers: Modifiers::default() });
        let chord = parse_chord("cmd+shift+z").unwrap();
        assert_eq!(chord.key_code, 6);
        assert!(chord.modifiers.command && chord.modifiers.shift && !chord.modifiers.control);
        assert_eq!(parse_chord("Option+Left").unwrap().key_code, 123);
        assert!(parse_chord("option+left").unwrap().modifiers.option);
        assert_eq!(parse_chord("cmd+,").unwrap().key_code, 43);
        assert_eq!(parse_chord("F5").unwrap().key_code, 96);
        let plus = parse_chord("cmd++").unwrap();
        assert!(plus.modifiers.command && plus.modifiers.shift && plus.key_code == 24);
    }

    #[test]
    fn rejects_unknown_keys_and_modifiers() {
        assert!(parse_chord("").is_err());
        assert!(parse_chord("cmd+").is_err());
        assert!(parse_chord("hyper+a").is_err());
        assert!(parse_chord("launchpad").is_err());
    }

    #[test]
    fn the_stop_chord_is_refused_in_any_spelling() {
        assert!(parse_chord("ctrl+alt+cmd+.").is_err());
        assert!(parse_chord("Command+Option+Control+.").is_err());
        assert!(parse_chord("cmd+.").is_ok());
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
