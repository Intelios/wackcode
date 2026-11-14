use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use wack_agent::{Loading, Tool, ToolContext, ToolSpec};

use crate::text::{parse, resolve};

pub struct Edit;

#[derive(Deserialize)]
struct Input {
    path: String,
    edits: Vec<Change>,
}

#[derive(Deserialize)]
struct Change {
    old_text: String,
    new_text: String,
}

#[async_trait]
impl Tool for Edit {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "edit".into(),
            description: "Changes a file by replacing exact pieces of its text. Each old_text must appear exactly once in the file as it is now, whitespace included, and the pieces must not overlap; every edit is matched against the original file, not against the result of the edits before it. Either all the edits apply or none do. Include enough surrounding lines to make each piece unique, and no more.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file to change, absolute or relative to the working directory." },
                    "edits": {
                        "type": "array",
                        "description": "The replacements to make.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_text": { "type": "string", "description": "The exact text to replace." },
                                "new_text": { "type": "string", "description": "What to put in its place. Empty to delete." }
                            },
                            "required": ["old_text", "new_text"]
                        }
                    }
                },
                "required": ["path", "edits"]
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "change parts of an existing file by exact text replacement".into(),
        }
    }

    async fn run(&self, input: Value, context: &ToolContext) -> Result<String, String> {
        let input: Input = parse(input)?;
        let path = resolve(&context.cwd, &input.path);
        let bytes = tokio::fs::read(&path).await.map_err(|error| format!("Could not read {}: {error}.", input.path))?;
        let original = String::from_utf8(bytes).map_err(|_| format!("{} is not a text file.", input.path))?;
        let changed = apply(&original, &input.edits)?;
        tokio::fs::write(&path, changed).await.map_err(|error| format!("Could not write {}: {error}.", input.path))?;
        let noun = if input.edits.len() == 1 { "change" } else { "changes" };
        Ok(format!("Edited {} ({} {noun}).", input.path, input.edits.len()))
    }
}

/// Applies every edit to `original`, or explains why none can apply. A file with Windows line
/// endings is matched as if it had Unix ones and written back as it was, since models write `\n`.
fn apply(original: &str, edits: &[Change]) -> Result<String, String> {
    if edits.is_empty() {
        return Err("There are no edits to make.".to_string());
    }
    let windows = original.contains("\r\n");
    let unix = |text: &str| if windows { text.replace("\r\n", "\n") } else { text.to_string() };
    let text = unix(original);

    let mut spans = Vec::new();
    for (index, edit) in edits.iter().enumerate() {
        let number = index + 1;
        let old = unix(&edit.old_text);
        if old.is_empty() {
            return Err(format!("Edit {number} has an empty old_text. To create a file, use write."));
        }
        let mut found = text.match_indices(&old);
        let Some((start, _)) = found.next() else {
            return Err(format!("Edit {number}: old_text was not found in the file. It must match exactly, including spaces and line breaks. Read the file again if it may have changed."));
        };
        let more = found.count();
        if more > 0 {
            return Err(format!("Edit {number}: old_text appears {} times in the file. Add surrounding lines so it matches only once.", more + 1));
        }
        spans.push((start, start + old.len(), unix(&edit.new_text), number));
    }
    spans.sort_by_key(|span| span.0);
    for pair in spans.windows(2) {
        if pair[1].0 < pair[0].1 {
            return Err(format!("Edits {} and {} overlap. Merge them into one.", pair[0].3, pair[1].3));
        }
    }

    let mut result = String::with_capacity(text.len());
    let mut at = 0;
    for (start, end, new, _) in &spans {
        result.push_str(&text[at..*start]);
        result.push_str(new);
        at = *end;
    }
    result.push_str(&text[at..]);
    if result == text {
        return Err("The edits change nothing: each new_text is the same as its old_text.".to_string());
    }
    Ok(if windows { result.replace('\n', "\r\n") } else { result })
}

#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

    use super::*;

    fn change(old: &str, new: &str) -> Change {
        Change { old_text: old.into(), new_text: new.into() }
    }

    #[test]
    fn edits_apply_against_the_original_in_any_order() {
        let original = "fn a() {}\nfn b() {}\nfn c() {}\n";
        let edits = [change("fn c() {}", "fn c() { 3 }"), change("fn a() {}", "fn a() { 1 }")];
        assert_eq!(apply(original, &edits).unwrap(), "fn a() { 1 }\nfn b() {}\nfn c() { 3 }\n");
    }

    #[test]
    fn nothing_applies_unless_everything_does() {
        let original = "let x = 1;\nlet y = 1;\n";
        assert!(apply(original, &[change("= 1;", "= 2;")]).unwrap_err().contains("appears 2 times"));
        assert!(apply(original, &[change("let z", "let w")]).unwrap_err().contains("was not found"));
        assert!(apply(original, &[change("let x = 1;", "a"), change("x = 1;\nlet y", "b")]).unwrap_err().contains("overlap"));
        assert!(apply(original, &[change("let x", "let x")]).unwrap_err().contains("change nothing"));
        assert!(apply(original, &[change("", "x")]).unwrap_err().contains("empty old_text"));
        assert!(apply(original, &[]).is_err());
    }

    #[test]
    fn windows_line_endings_are_matched_and_kept() {
        let original = "one\r\ntwo\r\nthree\r\n";
        assert_eq!(apply(original, &[change("one\ntwo", "1\n2")]).unwrap(), "1\r\n2\r\nthree\r\n");
    }

    #[tokio::test]
    async fn a_failed_edit_leaves_the_file_alone() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("a.rs");
        std::fs::write(&path, "alpha\nbeta\n").unwrap();
        let context = ToolContext { cwd: folder.path().to_path_buf(), cancel: CancellationToken::new() };

        let input = json!({ "path": "a.rs", "edits": [{ "old_text": "alpha", "new_text": "ALPHA" }, { "old_text": "gamma", "new_text": "x" }] });
        assert!(Edit.run(input, &context).await.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "alpha\nbeta\n");

        let input = json!({ "path": "a.rs", "edits": [{ "old_text": "beta", "new_text": "BETA" }] });
        assert_eq!(Edit.run(input, &context).await.unwrap(), "Edited a.rs (1 change).");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "alpha\nBETA\n");
    }
}
