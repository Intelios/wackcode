use std::path::Path;

use async_trait::async_trait;
use globset::{Glob, GlobMatcher};
use regex::RegexBuilder;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wack_agent::{Loading, Tool, ToolContext, ToolSpec};

use crate::{
    text::{MAX_BYTES, cut, display, looks_binary, parse, resolve},
    walk::walk,
};

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1_000;
const MAX_LINE_BYTES: usize = 500;
/// Files larger than this are skipped: they are almost never source.
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

pub struct Grep;

#[derive(Deserialize)]
struct Input {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    #[serde(default)]
    ignore_case: bool,
    #[serde(default)]
    literal: bool,
    #[serde(default)]
    context: usize,
    limit: Option<usize>,
}

#[async_trait]
impl Tool for Grep {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "grep".into(),
            description: format!("Searches file contents for a regular expression and returns matching lines as path:line: text. Respects .gitignore and skips binary files. Returns up to {DEFAULT_LIMIT} matches unless you set a limit."),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "A regular expression (Rust regex syntax), or plain text when literal is true." },
                    "path": { "type": "string", "description": "The folder or file to search. Defaults to the working directory." },
                    "glob": { "type": "string", "description": "Only search files matching this glob, such as *.rs or src/**/*.ts." },
                    "ignore_case": { "type": "boolean" },
                    "literal": { "type": "boolean", "description": "Treat the pattern as plain text." },
                    "context": { "type": "integer", "description": "Lines to show before and after each match." },
                    "limit": { "type": "integer", "description": format!("The most matches to return, up to {MAX_LIMIT}.") }
                },
                "required": ["pattern"]
            }),
            loading: Loading::Always,
            parallel_safe: true,
            hint: "search file contents by regular expression".into(),
        }
    }

    async fn run(&self, input: Value, context: &ToolContext) -> Result<String, String> {
        let input: Input = parse(input)?;
        let root = resolve(&context.cwd, input.path.as_deref().unwrap_or("."));
        let cancel = context.cancel.clone();
        // File reading and matching block, so they run off the async threads.
        tokio::task::spawn_blocking(move || search(&input, &root, &cancel))
            .await
            .map_err(|error| format!("The search failed: {error}."))?
    }
}

pub(crate) fn glob_matcher(pattern: &str) -> Result<GlobMatcher, String> {
    Glob::new(pattern).map(|glob| glob.compile_matcher()).map_err(|error| format!("The glob is not valid: {error}."))
}

/// A glob with no slash names files anywhere; one with a slash is a path under the search root.
pub(crate) fn glob_matches(matcher: &GlobMatcher, pattern: &str, relative: &Path) -> bool {
    if pattern.contains('/') { matcher.is_match(relative) } else { relative.file_name().is_some_and(|name| matcher.is_match(name)) }
}

fn search(input: &Input, root: &Path, cancel: &CancellationToken) -> Result<String, String> {
    if !root.exists() {
        return Err(format!("{} does not exist.", root.display()));
    }
    let pattern = if input.literal { regex::escape(&input.pattern) } else { input.pattern.clone() };
    let regex = RegexBuilder::new(&pattern)
        .case_insensitive(input.ignore_case)
        .build()
        .map_err(|error| format!("The pattern is not a valid regular expression: {error}"))?;
    let glob = input.glob.as_deref().map(glob_matcher).transpose()?;
    let limit = input.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    // Paths are shown relative to the folder searched, or to a single file's folder.
    let base = if root.is_dir() { root } else { root.parent().unwrap_or(root) };

    let mut out = String::new();
    let mut matches = 0;
    let mut stopped_early = false;
    'files: for entry in walk(root).flatten() {
        if cancel.is_cancelled() {
            return Err("The search was stopped.".to_string());
        }
        let path = entry.path();
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let relative = path.strip_prefix(base).unwrap_or(path);
        if let (Some(matcher), Some(pattern)) = (&glob, &input.glob)
            && !glob_matches(matcher, pattern, relative)
        {
            continue;
        }
        if entry.metadata().is_ok_and(|metadata| metadata.len() > MAX_FILE_BYTES) {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else { continue };
        if looks_binary(&bytes) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().collect();
        let name = display(path, base);
        let mut shown_to = 0;
        for (index, line) in lines.iter().enumerate() {
            if !regex.is_match(line) {
                continue;
            }
            if matches == limit || out.len() > MAX_BYTES {
                stopped_early = true;
                break 'files;
            }
            matches += 1;
            let from = index.saturating_sub(input.context).max(shown_to);
            let to = (index + input.context + 1).min(lines.len());
            if input.context > 0 && from > shown_to && !out.is_empty() {
                out.push_str("--\n");
            }
            for (number, line) in lines.iter().enumerate().take(to).skip(from) {
                // A context line that is itself a match is printed when its own turn comes.
                if number > index && regex.is_match(line) {
                    shown_to = number;
                    break;
                }
                let mark = if number == index { ':' } else { '-' };
                out.push_str(&format!("{name}{mark}{}{mark} {}\n", number + 1, cut(line, MAX_LINE_BYTES)));
                shown_to = number + 1;
            }
        }
    }
    if matches == 0 {
        return Ok("No matches.".to_string());
    }
    if stopped_early {
        out.push_str(&format!("\n[Stopped at {matches} matches. Narrow the pattern or path, or set a higher limit.]"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {\n    parse();\n}\n\nfn parse() {}\n").unwrap();
        std::fs::write(root.join("src/deep/lexer.ts"), "export function Parse() {}\n").unwrap();
        std::fs::write(root.join("target/out.rs"), "fn parse() {}\n").unwrap();
        std::fs::write(root.join(".git/config"), "parse\n").unwrap();
        std::fs::write(root.join(".env.example"), "PARSE_MODE=fast\n").unwrap();
        std::fs::write(root.join("blob.bin"), b"parse\0parse").unwrap();
        folder
    }

    async fn grep(folder: &tempfile::TempDir, input: Value) -> String {
        let context = ToolContext { cwd: folder.path().to_path_buf(), cancel: CancellationToken::new() };
        Grep.run(input, &context).await.unwrap()
    }

    #[tokio::test]
    async fn finds_matches_and_skips_ignored_binary_and_git_files() {
        let folder = project();
        assert_eq!(grep(&folder, json!({ "pattern": "parse" })).await, "src/main.rs:2:     parse();\nsrc/main.rs:5: fn parse() {}\n");
        let any_case = grep(&folder, json!({ "pattern": "parse", "ignore_case": true })).await;
        assert!(any_case.contains(".env.example:1: PARSE_MODE=fast") && any_case.contains("src/deep/lexer.ts:1:"));
        assert!(!any_case.contains("target") && !any_case.contains(".git/") && !any_case.contains("blob"));
    }

    #[tokio::test]
    async fn narrows_by_glob_path_and_literal_text() {
        let folder = project();
        assert_eq!(grep(&folder, json!({ "pattern": "parse", "ignore_case": true, "glob": "*.ts" })).await, "src/deep/lexer.ts:1: export function Parse() {}\n");
        assert_eq!(grep(&folder, json!({ "pattern": "Parse", "glob": "src/**/*.ts" })).await.lines().count(), 1);
        assert_eq!(grep(&folder, json!({ "pattern": "parse()", "literal": true, "path": "src/main.rs" })).await.lines().count(), 2);
        assert_eq!(grep(&folder, json!({ "pattern": "parse()" })).await.lines().count(), 2, "an empty group still matches");
        assert_eq!(grep(&folder, json!({ "pattern": "nothing_here" })).await, "No matches.");
    }

    #[tokio::test]
    async fn shows_context_once_and_stops_at_the_limit() {
        let folder = project();
        let with_context = grep(&folder, json!({ "pattern": "parse", "context": 1 })).await;
        assert_eq!(
            with_context,
            "src/main.rs-1- fn main() {\nsrc/main.rs:2:     parse();\nsrc/main.rs-3- }\nsrc/main.rs-4- \nsrc/main.rs:5: fn parse() {}\n"
        );
        let limited = grep(&folder, json!({ "pattern": "parse", "limit": 1 })).await;
        assert!(limited.starts_with("src/main.rs:2:") && limited.contains("[Stopped at 1 matches."));
    }

    #[tokio::test]
    async fn a_bad_pattern_is_explained() {
        let folder = project();
        let context = ToolContext { cwd: folder.path().to_path_buf(), cancel: CancellationToken::new() };
        assert!(Grep.run(json!({ "pattern": "(" }), &context).await.unwrap_err().contains("not a valid regular expression"));
        assert!(Grep.run(json!({ "pattern": "x", "path": "nowhere" }), &context).await.unwrap_err().contains("does not exist"));
    }
}
