use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use wack_agent::{Loading, Tool, ToolContext, ToolOutput, ToolSpec};

use crate::{
    grep::{glob_matcher, glob_matches},
    text::{parse, resolve},
    walk::walk,
};

const DEFAULT_LIMIT: usize = 500;
const MAX_LIMIT: usize = 5_000;

pub struct Find;

#[derive(Deserialize)]
struct Input {
    pattern: String,
    path: Option<String>,
    limit: Option<usize>,
}

#[async_trait]
impl Tool for Find {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "find".into(),
            description: format!("Finds files and folders by name. The pattern is a glob: one without a slash (*.rs, Cargo.toml) matches names anywhere under the path; one with a slash (src/**/*.rs) matches paths. Respects .gitignore. Folders end with a slash. Returns up to {DEFAULT_LIMIT} paths unless you set a limit."),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "A glob such as *.rs, **/*.test.ts or src/**/mod.rs." },
                    "path": { "type": "string", "description": "The folder to search. Defaults to the working directory." },
                    "limit": { "type": "integer", "description": format!("The most paths to return, up to {MAX_LIMIT}.") }
                },
                "required": ["pattern"]
            }),
            loading: Loading::Always,
            parallel_safe: true,
            hint: "find files and folders by name or glob".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let input: Input = parse(input)?;
        let root = resolve(&context.cwd, input.path.as_deref().unwrap_or("."));
        if !root.is_dir() {
            return Err(format!("{} is not a folder.", input.path.as_deref().unwrap_or(".")));
        }
        let matcher = glob_matcher(&input.pattern)?;
        let limit = input.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let cancel = context.cancel.clone();
        tokio::task::spawn_blocking(move || {
            let mut found = Vec::new();
            let mut more = false;
            for entry in walk(&root).flatten() {
                if cancel.is_cancelled() {
                    return Err("The search was stopped.".to_string());
                }
                let Ok(relative) = entry.path().strip_prefix(&root) else { continue };
                if relative.as_os_str().is_empty() || !glob_matches(&matcher, &input.pattern, relative) {
                    continue;
                }
                if found.len() == limit {
                    more = true;
                    break;
                }
                let slash = if entry.file_type().is_some_and(|kind| kind.is_dir()) { "/" } else { "" };
                found.push(format!("{}{slash}", relative.display()));
            }
            if found.is_empty() {
                return Ok("Nothing found.".to_string());
            }
            let mut out = found.join("\n");
            if more {
                out.push_str(&format!("\n\n[Stopped at {limit} paths. Narrow the pattern or path, or set a higher limit.]"));
            }
            Ok(out)
        })
        .await
        .map_err(|error| format!("The search failed: {error}."))?
        .map(ToolOutput::from)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]
    async fn finds_by_name_and_by_path() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        std::fs::create_dir_all(root.join("src/ui")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "node_modules\n").unwrap();
        for file in ["src/main.rs", "src/ui/view.rs", "src/ui/view.css", "node_modules/pkg/index.rs", "README.md"] {
            std::fs::write(root.join(file), "").unwrap();
        }
        let context = ToolContext::detached(root.to_path_buf());
        let find = |input: Value| async { Find.run(input, &context).await.map(|output| output.text) };

        assert_eq!(find(json!({ "pattern": "*.rs" })).await.unwrap(), "src/main.rs\nsrc/ui/view.rs");
        assert_eq!(find(json!({ "pattern": "src/*.rs" })).await.unwrap(), "src/main.rs\nsrc/ui/view.rs", "a star crosses folders, as in a shell's **");
        assert_eq!(find(json!({ "pattern": "ui" })).await.unwrap(), "src/ui/");
        assert_eq!(find(json!({ "pattern": "view.*", "path": "src/ui" })).await.unwrap(), "view.css\nview.rs");
        assert_eq!(find(json!({ "pattern": "*.go" })).await.unwrap(), "Nothing found.");
        assert!(find(json!({ "pattern": "*", "limit": 2 })).await.unwrap().contains("[Stopped at 2 paths."));
        assert!(find(json!({ "pattern": "*.rs", "path": "README.md" })).await.is_err());
    }
}
