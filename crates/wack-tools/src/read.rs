use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use wack_agent::{Loading, Tool, ToolContext, ToolSpec};

use crate::text::{MAX_BYTES, MAX_LINES, cut, looks_binary, parse, resolve};

/// Larger files are refused rather than loaded whole into memory.
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

pub struct Read;

#[derive(Deserialize)]
struct Input {
    path: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[async_trait]
impl Tool for Read {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read".into(),
            description: format!("Reads a text file. Returns up to {MAX_LINES} lines or {} KB, whichever comes first, and says how to continue when there is more. Use offset and limit to read part of a large file.", MAX_BYTES / 1024),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file to read, absolute or relative to the working directory." },
                    "offset": { "type": "integer", "description": "The line to start from, counting from 1." },
                    "limit": { "type": "integer", "description": "The most lines to return." }
                },
                "required": ["path"]
            }),
            loading: Loading::Always,
            parallel_safe: true,
            hint: "read a file's contents".into(),
            guidelines: Vec::new(),
            ends_run: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<String, String> {
        let input: Input = parse(input)?;
        let path = resolve(&context.cwd, &input.path);
        let size = tokio::fs::metadata(&path).await.map_err(|error| format!("Could not read {}: {error}.", input.path))?;
        if size.is_dir() {
            return Err(format!("{} is a folder. Use ls to list it.", input.path));
        }
        if size.len() > MAX_FILE_BYTES {
            return Err(format!("{} is too large to read ({} MB). Search it with grep, or read part of it with a shell command.", input.path, size.len() / 1024 / 1024));
        }
        let bytes = tokio::fs::read(&path).await.map_err(|error| format!("Could not read {}: {error}.", input.path))?;
        if looks_binary(&bytes) {
            return Err(format!("{} is not a text file.", input.path));
        }
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return Ok("(The file is empty.)".to_string());
        }
        let start = input.offset.unwrap_or(1).max(1) - 1;
        if start >= lines.len() {
            return Err(format!("The file has only {} lines.", lines.len()));
        }

        let most = input.limit.unwrap_or(MAX_LINES).clamp(1, MAX_LINES);
        let mut shown = String::new();
        let mut end = start;
        while end < lines.len() && end - start < most {
            let line = lines[end];
            if shown.len() + line.len() + 1 > MAX_BYTES {
                if end == start {
                    // One line larger than a whole result: show its start rather than nothing.
                    shown.push_str(cut(line, MAX_BYTES));
                    shown.push_str("\n[This line is too long to show in full.]\n");
                    end += 1;
                }
                break;
            }
            shown.push_str(line);
            shown.push('\n');
            end += 1;
        }
        if end < lines.len() {
            shown.push_str(&format!("\n[Lines {}-{} of {}. Continue with offset={}.]", start + 1, end, lines.len(), end + 1));
        }
        Ok(shown)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    fn context(folder: &tempfile::TempDir) -> ToolContext<'static> {
        ToolContext::detached(folder.path().to_path_buf())
    }

    #[tokio::test]
    async fn reads_a_file_whole_and_in_parts() {
        let folder = tempfile::tempdir().unwrap();
        let body: String = (1..=10).map(|line| format!("line {line}\n")).collect();
        std::fs::write(folder.path().join("a.txt"), &body).unwrap();
        let context = context(&folder);

        assert_eq!(Read.run(json!({ "path": "a.txt" }), &context).await.unwrap(), body);
        let part = Read.run(json!({ "path": "a.txt", "offset": 4, "limit": 2 }), &context).await.unwrap();
        assert_eq!(part, "line 4\nline 5\n\n[Lines 4-5 of 10. Continue with offset=6.]");
        assert!(Read.run(json!({ "path": "a.txt", "offset": 11 }), &context).await.unwrap_err().contains("only 10 lines"));
    }

    #[tokio::test]
    async fn large_files_stop_at_the_size_limit_and_say_how_to_go_on() {
        let folder = tempfile::tempdir().unwrap();
        let body: String = (1..=5_000).map(|line| format!("{line} {}\n", "x".repeat(40))).collect();
        std::fs::write(folder.path().join("big.txt"), body).unwrap();
        let shown = Read.run(json!({ "path": "big.txt" }), &context(&folder)).await.unwrap();
        assert!(shown.len() <= MAX_BYTES + 100);
        let next: usize = shown.rsplit_once("offset=").unwrap().1.trim_end_matches(".]").parse().unwrap();
        assert!(shown.contains(&format!("\n{} x", next - 1)) && !shown.contains(&format!("\n{next} x")));
    }

    #[tokio::test]
    async fn refuses_what_is_not_a_text_file() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("image.png"), [0x89, b'P', b'N', b'G', 0, 0, 1]).unwrap();
        std::fs::write(folder.path().join("empty.txt"), "").unwrap();
        let context = context(&folder);
        assert!(Read.run(json!({ "path": "image.png" }), &context).await.unwrap_err().contains("not a text file"));
        assert!(Read.run(json!({ "path": "." }), &context).await.unwrap_err().contains("is a folder"));
        assert!(Read.run(json!({ "path": "missing.txt" }), &context).await.is_err());
        assert!(Read.run(json!({ "file": "a" }), &context).await.unwrap_err().contains("input is wrong"));
        assert_eq!(Read.run(json!({ "path": "empty.txt" }), &context).await.unwrap(), "(The file is empty.)");
    }
}
