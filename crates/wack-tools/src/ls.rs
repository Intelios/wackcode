use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use wack_agent::{Loading, Tool, ToolContext, ToolOutput, ToolSpec};

use crate::text::{parse, resolve};

const DEFAULT_LIMIT: usize = 500;

pub struct Ls;

#[derive(Deserialize)]
struct Input {
    path: Option<String>,
    limit: Option<usize>,
}

#[async_trait]
impl Tool for Ls {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "ls".into(),
            description: format!("Lists a folder: everything directly inside it, dotfiles and ignored files included, sorted by name. Folders end with a slash. Returns up to {DEFAULT_LIMIT} entries unless you set a limit."),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The folder to list. Defaults to the working directory." },
                    "limit": { "type": "integer", "description": "The most entries to return." }
                }
            }),
            loading: Loading::Always,
            parallel_safe: true,
            hint: "list what is in a folder".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let input: Input = parse(input)?;
        let shown = input.path.as_deref().unwrap_or(".");
        let path = resolve(&context.cwd, shown);
        let mut reader = tokio::fs::read_dir(&path).await.map_err(|error| format!("Could not list {shown}: {error}."))?;
        let mut names = Vec::new();
        while let Ok(Some(entry)) = reader.next_entry().await {
            // Following links, so a link to a folder reads as the folder it behaves as.
            let slash = if tokio::fs::metadata(entry.path()).await.is_ok_and(|metadata| metadata.is_dir()) { "/" } else { "" };
            names.push(format!("{}{slash}", entry.file_name().to_string_lossy()));
        }
        if names.is_empty() {
            return Ok("(The folder is empty.)".to_string().into());
        }
        names.sort_by_key(|name| name.to_lowercase());
        let limit = input.limit.unwrap_or(DEFAULT_LIMIT).max(1);
        let total = names.len();
        names.truncate(limit);
        let mut out = names.join("\n");
        if total > limit {
            out.push_str(&format!("\n\n[{limit} of {total} entries shown. Set a higher limit to see the rest.]"));
        }
        Ok(out.into())
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]
    async fn lists_sorted_with_folders_marked() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(folder.path().join("src")).unwrap();
        std::fs::create_dir_all(folder.path().join("empty")).unwrap();
        for file in ["Zed.md", "apple.txt", ".env"] {
            std::fs::write(folder.path().join(file), "").unwrap();
        }
        let context = ToolContext::detached(folder.path().to_path_buf());
        assert_eq!(Ls.run(json!({}), &context).await.unwrap().text, ".env\napple.txt\nempty/\nsrc/\nZed.md");
        assert_eq!(Ls.run(json!({ "path": "empty" }), &context).await.unwrap().text, "(The folder is empty.)");
        assert!(Ls.run(json!({ "limit": 2 }), &context).await.unwrap().text.ends_with("[2 of 5 entries shown. Set a higher limit to see the rest.]"));
        assert!(Ls.run(json!({ "path": "missing" }), &context).await.is_err());
    }
}
