use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use wack_agent::{Loading, Tool, ToolContext, ToolOutput, ToolSpec};

use crate::text::{parse, resolve};

pub struct Write;

#[derive(Deserialize)]
struct Input {
    path: String,
    content: String,
}

#[async_trait]
impl Tool for Write {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write".into(),
            description: "Writes a file, creating it and any missing folders, or replacing it entirely if it exists. To change part of an existing file, use edit instead.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file to write, absolute or relative to the working directory." },
                    "content": { "type": "string", "description": "The file's whole new content." }
                },
                "required": ["path", "content"]
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "create a file or replace one entirely".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let input: Input = parse(input)?;
        let path = resolve(context, "write", &input.path)?;
        if let Some(folder) = path.parent() {
            tokio::fs::create_dir_all(folder).await.map_err(|error| format!("Could not create the folder for {}: {error}.", input.path))?;
        }
        let existed = path.exists();
        // What it replaces, so the result can say what changed: the transcript and the turn's
        // Changes card read these counts, and a whole file counted as added disagreed with Git.
        let old = if existed { tokio::fs::read_to_string(&path).await.ok() } else { None };
        tokio::fs::write(&path, &input.content).await.map_err(|error| format!("Could not write {}: {error}.", input.path))?;
        let lines = input.content.lines().count();
        let noun = if lines == 1 { "line" } else { "lines" };
        Ok(match (existed, old) {
            (false, _) => format!("Created {} ({lines} {noun}).", input.path),
            (true, Some(old)) => {
                let (added, removed) = line_changes(&old, &input.content);
                format!("Replaced {} ({lines} {noun}, +{added} −{removed}).", input.path)
            }
            (true, None) => format!("Replaced {} ({lines} {noun}).", input.path),
        }
        .into())
    }
}

/// How many lines `new` adds to `old` and removes from it: the lines outside their longest
/// common subsequence. A change too big to compare line by line counts its whole middle.
pub fn line_changes(old: &str, new: &str) -> (usize, usize) {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
    let (old, new) = (&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]);
    if old.len().saturating_mul(new.len()) > MAX_COMPARED {
        return (new.len(), old.len());
    }
    // One row of the LCS table at a time.
    let mut row = vec![0u32; new.len() + 1];
    for a in old {
        let mut diagonal = 0;
        for (j, b) in new.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if a == b { diagonal + 1 } else { above.max(row[j]) };
            diagonal = above;
        }
    }
    let common = row[new.len()] as usize;
    (new.len() - common, old.len() - common)
}

/// Line pairs compared at most when counting a replaced file's changes (a few milliseconds).
const MAX_COMPARED: usize = 4_000_000;

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]
    async fn creates_folders_and_replaces_files() {
        let folder = tempfile::tempdir().unwrap();
        let context = ToolContext::detached(folder.path().to_path_buf());
        let created = Write.run(json!({ "path": "src/deep/a.rs", "content": "one\ntwo\n" }), &context).await.unwrap().text;
        assert_eq!(created, "Created src/deep/a.rs (2 lines).");
        let replaced = Write.run(json!({ "path": "src/deep/a.rs", "content": "one\nthree\n" }), &context).await.unwrap().text;
        assert_eq!(replaced, "Replaced src/deep/a.rs (2 lines, +1 −1).");
        assert_eq!(std::fs::read_to_string(folder.path().join("src/deep/a.rs")).unwrap(), "one\nthree\n");
    }

    #[test]
    fn counts_what_a_replacement_changed() {
        assert_eq!(line_changes("a\nb\nc\n", "a\nb\nc\n"), (0, 0));
        assert_eq!(line_changes("a\nb\nc", "a\nx\nc\nd"), (2, 1));
        // Moved lines inside a changed middle are not counted twice.
        assert_eq!(line_changes("h\n1\n2\n3\nt", "h\n0\n1\n2\n3\n4\nt"), (2, 0));
        assert_eq!(line_changes("", "a\nb"), (2, 0));
    }
}
