//! Helpers the tools share: paths, input parsing, and keeping output a size a model can take.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde_json::Value;
use wack_agent::ToolContext;

/// A tool result never exceeds these, whichever is reached first.
pub const MAX_LINES: usize = 2_000;
pub const MAX_BYTES: usize = 50 * 1024;

pub fn parse<T: DeserializeOwned>(input: Value) -> Result<T, String> {
    serde_json::from_value(input).map_err(|error| format!("The tool's input is wrong: {error}."))
}

/// The path `tool` should use for one the model wrote. Every file tool takes its paths here, so a
/// confined run (a Chat chat, `ToolContext::confine`) can never reach a path that was not
/// checked: it gets back the checked, real location or the reason it was refused.
pub fn resolve(context: &ToolContext<'_>, tool: &str, raw: &str) -> Result<PathBuf, String> {
    match &context.confine {
        Some(root) => crate::confine::confine(tool, raw, root),
        None => Ok(join(&context.cwd, raw)),
    }
}

/// A path the model wrote, unchecked: `~` is the home folder, anything relative is under `cwd`.
pub fn join(cwd: &Path, raw: &str) -> PathBuf {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix('~').filter(|rest| rest.is_empty() || rest.starts_with('/'))
        && let Some(home) = std::env::var_os("HOME")
    {
        return Path::new(&home).join(rest.trim_start_matches('/'));
    }
    cwd.join(raw)
}

/// A path as the model should see it: relative to `base` when it is inside it.
pub fn display(path: &Path, base: &Path) -> String {
    path.strip_prefix(base).unwrap_or(path).display().to_string()
}

/// Files with a NUL byte near the start are treated as binary and never shown as text.
pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8_192).any(|byte| *byte == 0)
}

/// Cuts a string to at most `max` bytes without splitting a character.
pub fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The end of `text` that fits a tool result, and whether anything was left out.
pub fn tail(text: &str) -> (&str, bool) {
    let mut start = text.len();
    let mut lines = 0;
    for (index, _) in text.rmatch_indices('\n') {
        // The newline that ends the text does not start a line.
        if index + 1 == text.len() {
            continue;
        }
        if lines + 1 >= MAX_LINES || text.len() - (index + 1) > MAX_BYTES {
            break;
        }
        lines += 1;
        start = index + 1;
    }
    if lines + 1 < MAX_LINES && text.len() <= MAX_BYTES {
        return (text, false);
    }
    if start == text.len() {
        // One enormous last line: keep its end.
        let mut from = text.len() - MAX_BYTES.min(text.len());
        while !text.is_char_boundary(from) {
            from += 1;
        }
        return (&text[from..], from > 0);
    }
    (&text[start..], start > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_home_and_absolute_paths() {
        let cwd = Path::new("/work/app");
        assert_eq!(join(cwd, "src/main.rs"), Path::new("/work/app/src/main.rs"));
        assert_eq!(join(cwd, "/etc/hosts"), Path::new("/etc/hosts"));
        assert_eq!(join(cwd, "~name"), Path::new("/work/app/~name"));
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(join(cwd, "~/notes.md"), Path::new(&home).join("notes.md"));
        }
    }

    #[test]
    fn the_tail_keeps_whole_lines_from_the_end() {
        assert_eq!(tail("a\nb\n"), ("a\nb\n", false));
        let many: String = (0..5_000).map(|line| format!("line {line}\n")).collect();
        let (kept, cut) = tail(&many);
        assert!(cut);
        assert!(kept.ends_with("line 4999\n"));
        assert!(kept.lines().count() <= MAX_LINES && kept.len() <= MAX_BYTES);
        assert!(kept.starts_with("line "));

        let wide = "é".repeat(MAX_BYTES);
        let (kept, cut) = tail(&wide);
        assert!(cut && kept.len() <= MAX_BYTES);
    }

    #[test]
    fn cutting_never_splits_a_character() {
        assert_eq!(cut("héllo", 2), "h");
        assert_eq!(cut("héllo", 3), "hé");
    }
}
