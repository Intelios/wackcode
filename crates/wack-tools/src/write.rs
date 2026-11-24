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
        tokio::fs::write(&path, &input.content).await.map_err(|error| format!("Could not write {}: {error}.", input.path))?;
        let verb = if existed { "Replaced" } else { "Created" };
        let lines = input.content.lines().count();
        Ok(format!("{verb} {} ({lines} {}).", input.path, if lines == 1 { "line" } else { "lines" }).into())
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]
    async fn creates_folders_and_replaces_files() {
        let folder = tempfile::tempdir().unwrap();
        let context = ToolContext::detached(folder.path().to_path_buf());
        let created = Write.run(json!({ "path": "src/deep/a.rs", "content": "one\ntwo\n" }), &context).await.unwrap().text;
        assert_eq!(created, "Created src/deep/a.rs (2 lines).");
        let replaced = Write.run(json!({ "path": "src/deep/a.rs", "content": "three" }), &context).await.unwrap().text;
        assert!(replaced.starts_with("Replaced"));
        assert_eq!(std::fs::read_to_string(folder.path().join("src/deep/a.rs")).unwrap(), "three");
    }
}
