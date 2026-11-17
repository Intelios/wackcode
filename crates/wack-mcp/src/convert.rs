//! Pure helpers that turn MCP tools and results into the agent's shapes: the names the model
//! sees, input schemas, and tool output cut to the shared limits. Ported from version 2's
//! `tools.ts`, with its tests.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::spec::hex;

pub const MCP_PREFIX: &str = "mcp__";
/// Providers cap tool names; 64 characters of `[A-Za-z0-9_-]` is the strictest common limit.
const MAX_TOOL_NAME: usize = 64;
/// The same limits as every other tool result (`wack-tools/src/text.rs`).
pub const MAX_LINES: usize = 2_000;
pub const MAX_BYTES: usize = 50 * 1024;

/// `mcp__<server slug>__<tool>`. A tool name that needs changing to fit the allowed characters or
/// the length limit gets a short hash of the original, so two such tools can never collide.
pub fn tool_name(slug: &str, tool: &str) -> String {
    let prefix = format!("{MCP_PREFIX}{slug}__");
    let clean: String = tool.chars().map(|character| if character.is_ascii_alphanumeric() || character == '_' || character == '-' { character } else { '_' }).collect();
    if clean == tool && prefix.len() + clean.len() <= MAX_TOOL_NAME {
        return prefix + &clean;
    }
    let suffix = format!("_{}", &hex(&Sha256::digest(tool.as_bytes()))[..6]);
    let room = MAX_TOOL_NAME.saturating_sub(prefix.len() + suffix.len());
    // `clean` is ASCII, so any byte index is a character boundary.
    format!("{prefix}{}{suffix}", &clean[..clean.len().min(room)])
}

/// The tool's input schema as the agent offers it: plain JSON Schema with the dialect marker
/// dropped and an object at the root.
pub fn parameters(schema: &Value) -> Value {
    let mut parameters: Map<String, Value> = schema.as_object().cloned().unwrap_or_default();
    parameters.remove("$schema");
    parameters.insert("type".into(), Value::String("object".into()));
    if !parameters.get("properties").is_some_and(Value::is_object) {
        parameters.insert("properties".into(), Value::Object(Map::new()));
    }
    Value::Object(parameters)
}

/// A `tools/call` result as tool output. Text passes through; links and embedded text resources
/// become text; images, audio and binary resources are named but left out (tool results carry
/// text only, for now). An error result is an `Err`. Output past the shared limits is cut.
pub fn result_text(result: &Value) -> Result<String, String> {
    let blocks = result.get("content").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut texts: Vec<String> = Vec::new();
    for block in &blocks {
        let text = |name: &str| block.get(name).and_then(Value::as_str);
        match text("type") {
            Some("text") => texts.extend(text("text").map(str::to_string)),
            Some("image") => texts.push(format!("[Image omitted: {}]", text("mimeType").unwrap_or("image"))),
            Some("audio") => texts.push("[Audio omitted]".into()),
            Some("resource_link") => {
                let name = text("name").filter(|name| !name.is_empty()).map(|name| format!(" ({name})")).unwrap_or_default();
                texts.push(format!("Resource: {}{name}", text("uri").unwrap_or_default()));
            }
            Some("resource") => {
                let resource = block.get("resource").cloned().unwrap_or_default();
                texts.push(match resource.get("text").and_then(Value::as_str) {
                    Some(text) => text.to_string(),
                    None => format!("[Binary resource omitted: {}]", resource.get("uri").and_then(Value::as_str).unwrap_or_default()),
                });
            }
            _ => {}
        }
    }
    if blocks.is_empty()
        && let Some(structured) = result.get("structuredContent")
    {
        texts.push(serde_json::to_string_pretty(structured).unwrap_or_default());
    }
    let text = texts.join("\n\n");
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        let message = text.trim();
        return Err(if message.is_empty() { "The MCP tool reported an error.".into() } else { head(message).0 });
    }
    if text.is_empty() {
        return Ok("(no output)".into());
    }
    let (kept, truncated) = head(&text);
    Ok(if truncated { format!("{kept}\n\n[Output truncated to {} KB.]", MAX_BYTES / 1024) } else { kept })
}

/// The start of `text` that fits a tool result, in whole lines where it can be, and whether
/// anything was left out.
pub fn head(text: &str) -> (String, bool) {
    let mut end = 0;
    for (lines, line) in text.split_inclusive('\n').enumerate() {
        if lines >= MAX_LINES || end + line.len() > MAX_BYTES {
            break;
        }
        end += line.len();
    }
    if end == text.len() {
        return (text.to_string(), false);
    }
    if end == 0 {
        // One enormous first line: a hard cut.
        let mut cut = MAX_BYTES.min(text.len());
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        end = cut;
    }
    (text[..end].trim_end_matches('\n').to_string(), true)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn names_keep_valid_tools_as_they_are() {
        assert_eq!(tool_name("github", "create_issue"), "mcp__github__create_issue");
        assert_eq!(tool_name("github", "get-file"), "mcp__github__get-file");
    }

    #[test]
    fn names_with_other_characters_get_a_hash_so_they_cannot_collide() {
        let dotted = tool_name("fs", "read.file");
        assert!(dotted.starts_with("mcp__fs__read_file_") && dotted.len() == "mcp__fs__read_file_".len() + 6, "{dotted}");
        assert_ne!(dotted, tool_name("fs", "read_file"));
    }

    #[test]
    fn long_names_fit_64_characters_and_stay_distinct() {
        let first = tool_name("a_very_long_server_n", "retrieve_all_of_the_pull_request_review_comments_for_a_repo");
        let second = tool_name("a_very_long_server_n", "retrieve_all_of_the_pull_request_review_comments_for_a_user");
        assert!(first.len() <= 64 && second.len() <= 64);
        assert_ne!(first, second);
        assert!(first.chars().all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-'));
        assert!(tool_name("s", "日本語").starts_with("mcp__s_____"));
    }

    #[test]
    fn schemas_drop_the_dialect_and_always_have_an_object_root() {
        assert_eq!(
            parameters(&json!({ "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object", "properties": { "q": { "type": "string" } }, "required": ["q"] })),
            json!({ "type": "object", "properties": { "q": { "type": "string" } }, "required": ["q"] })
        );
        assert_eq!(parameters(&Value::Null), json!({ "type": "object", "properties": {} }));
        assert_eq!(parameters(&json!({ "properties": [] })), json!({ "type": "object", "properties": {} }));
    }

    #[test]
    fn results_pass_text_and_describe_what_they_cannot_show() {
        let text = result_text(&json!({ "content": [
            { "type": "text", "text": "Hello" },
            { "type": "image", "data": "aGk=", "mimeType": "image/png" },
            { "type": "resource_link", "uri": "file:///a.txt", "name": "a.txt" },
            { "type": "resource", "resource": { "uri": "file:///b.txt", "text": "B contents" } },
            { "type": "resource", "resource": { "uri": "file:///c.bin", "blob": "AAAA" } },
            { "type": "audio", "data": "AAAA", "mimeType": "audio/wav" },
        ] }))
        .unwrap();
        assert_eq!(
            text,
            "Hello\n\n[Image omitted: image/png]\n\nResource: file:///a.txt (a.txt)\n\nB contents\n\n[Binary resource omitted: file:///c.bin]\n\n[Audio omitted]"
        );
    }

    #[test]
    fn structured_content_shows_when_there_is_no_content() {
        assert_eq!(result_text(&json!({ "content": [], "structuredContent": { "count": 2 } })).unwrap(), "{\n  \"count\": 2\n}");
        assert_eq!(result_text(&json!({ "content": [] })).unwrap(), "(no output)");
    }

    #[test]
    fn an_error_result_is_an_error() {
        assert_eq!(result_text(&json!({ "content": [{ "type": "text", "text": "Nope." }], "isError": true })).unwrap_err(), "Nope.");
        assert_eq!(result_text(&json!({ "isError": true })).unwrap_err(), "The MCP tool reported an error.");
    }

    #[test]
    fn long_output_is_cut_and_says_so() {
        let lines: String = (0..5_000).map(|index| format!("line {index}\n")).collect();
        let text = result_text(&json!({ "content": [{ "type": "text", "text": lines }] })).unwrap();
        assert!(text.len() < lines.len());
        assert!(text.ends_with("[Output truncated to 50 KB.]"));
        assert_eq!(text.lines().filter(|line| line.starts_with("line ")).count(), MAX_LINES);
        let (single, truncated) = head(&"é".repeat(60_000));
        assert!(truncated && single.len() <= MAX_BYTES);
    }
}
