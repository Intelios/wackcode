//! Syntax colours for code blocks.
//!
//! syntect's bundled grammars find the scopes; this module maps them onto the theme's six code
//! tokens rather than using a syntect colour theme, so code follows the user's accent like the rest
//! of the app. Results are cached by language and text: a finished block is highlighted once, and
//! a streaming one once per change.

use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::OnceLock,
};

use gpui::{FontStyle, Hsla, StyledText, TextRun, font};
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use crate::{theme::Theme, typography::MONO_FONT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Plain,
    Keyword,
    String,
    Number,
    Title,
    Meta,
    Comment,
}

/// Larger than any one chat needs; cleared whole when full.
const CACHE_ENTRIES: usize = 512;
/// Past this a block is drawn plain: highlighting it would stall the frame.
const MAX_BYTES: usize = 200_000;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

thread_local! {
    static CACHE: RefCell<HashMap<u64, Vec<(usize, Token)>>> = RefCell::new(HashMap::new());
}

fn syntax_for(language: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let name = match language.to_ascii_lowercase().as_str() {
        "" | "text" | "txt" | "plain" | "plaintext" | "output" | "console" => return None,
        "shell" | "zsh" | "sh" => "bash".to_string(),
        "ts" | "typescript" | "tsx" => "js".to_string(),
        "jsonc" | "json5" => "json".to_string(),
        "toml" => "toml".to_string(),
        "golang" => "go".to_string(),
        other => other.to_string(),
    };
    set.find_syntax_by_token(&name)
}

/// The token for a stack of scopes, innermost first.
fn token_for(stack: &ScopeStack) -> Token {
    static PREFIXES: OnceLock<Vec<(Scope, Token)>> = OnceLock::new();
    let prefixes = PREFIXES.get_or_init(|| {
        [
            ("comment", Token::Comment),
            ("string", Token::String),
            ("constant.numeric", Token::Number),
            ("constant.character", Token::String),
            ("constant.language", Token::Number),
            ("keyword", Token::Keyword),
            ("storage", Token::Keyword),
            ("entity.name", Token::Title),
            ("support.function", Token::Title),
            ("support.type", Token::Title),
            ("support.class", Token::Title),
            ("variable.function", Token::Title),
            ("meta.annotation", Token::Meta),
            ("meta.attribute", Token::Meta),
            ("entity.other.attribute-name", Token::Meta),
            ("meta.preprocessor", Token::Meta),
            ("punctuation.definition.annotation", Token::Meta),
            ("variable.language", Token::Keyword),
        ]
        .into_iter()
        .filter_map(|(prefix, token)| Scope::new(prefix).ok().map(|scope| (scope, token)))
        .collect()
    });
    for scope in stack.as_slice().iter().rev() {
        if let Some((_, token)) = prefixes.iter().find(|(prefix, _)| prefix.is_prefix_of(*scope)) {
            return *token;
        }
    }
    Token::Plain
}

/// `(length, token)` runs covering `text`, or `None` for an unknown language.
pub fn tokens(language: &str, text: &str) -> Option<Vec<(usize, Token)>> {
    if text.len() > MAX_BYTES {
        return None;
    }
    let syntax = syntax_for(language)?;
    let mut hasher = DefaultHasher::new();
    (language, text).hash(&mut hasher);
    let key = hasher.finish();
    if let Some(found) = CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return Some(found);
    }

    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut runs: Vec<(usize, Token)> = Vec::new();
    let mut push = |length: usize, token: Token| {
        if length == 0 {
            return;
        }
        match runs.last_mut() {
            Some((last, kind)) if *kind == token => *last += length,
            _ => runs.push((length, token)),
        }
    };
    for line in text.split_inclusive('\n') {
        let Ok(operations) = state.parse_line(line, syntaxes()) else {
            push(line.len(), Token::Plain);
            continue;
        };
        let mut at = 0;
        for (offset, operation) in operations {
            push(offset - at, token_for(&stack));
            at = offset;
            let _ = stack.apply(&operation);
        }
        push(line.len() - at, token_for(&stack));
    }

    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= CACHE_ENTRIES {
            cache.clear();
        }
        cache.insert(key, runs.clone());
    });
    Some(runs)
}

fn colour(token: Token, theme: &Theme) -> Hsla {
    match token {
        Token::Plain => theme.text_body,
        Token::Keyword => theme.code_keyword,
        Token::String => theme.code_string,
        Token::Number => theme.code_number,
        Token::Title => theme.code_title,
        Token::Meta => theme.code_meta,
        Token::Comment => theme.code_comment,
    }
}

/// `text` in mono with its syntax coloured.
pub fn highlighted(language: &str, text: &str, theme: &Theme) -> StyledText {
    let text = if text.is_empty() { " " } else { text };
    let runs = tokens(language, text).unwrap_or_else(|| vec![(text.len(), Token::Plain)]);
    let runs = runs
        .into_iter()
        .map(|(len, token)| {
            let mut face = font(MONO_FONT);
            if token == Token::Comment {
                face.style = FontStyle::Italic;
            }
            TextRun { len, font: face, color: colour(token, theme), background_color: None, underline: None, strikethrough: None }
        })
        .collect();
    StyledText::new(text.to_string()).with_runs(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_of(language: &str, text: &str) -> Vec<(String, Token)> {
        let mut at = 0;
        tokens(language, text)
            .unwrap()
            .into_iter()
            .map(|(length, token)| {
                let piece = text[at..at + length].to_string();
                at += length;
                (piece, token)
            })
            .collect()
    }

    #[test]
    fn rust_gets_keywords_strings_and_comments() {
        let found = tokens_of("rust", "fn main() {\n    let s = \"hi\"; // greet\n}\n");
        assert!(found.iter().any(|(text, token)| text == "fn" && *token == Token::Keyword), "{found:?}");
        assert!(found.iter().any(|(text, token)| text.contains("hi") && *token == Token::String), "{found:?}");
        assert!(found.iter().any(|(text, token)| text.contains("greet") && *token == Token::Comment), "{found:?}");
        let total: usize = tokens("rust", "fn main() {}\n").unwrap().iter().map(|(length, _)| length).sum();
        assert_eq!(total, "fn main() {}\n".len(), "runs cover the text exactly");
    }

    #[test]
    fn unknown_languages_are_plain() {
        assert!(tokens("", "anything").is_none());
        assert!(tokens("not-a-language", "anything").is_none());
        assert!(tokens("sh", "echo hi").is_some());
    }
}
