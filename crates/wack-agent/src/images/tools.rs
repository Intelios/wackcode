//! `list_images` and `read_image`: the model's way back to a picture a request elided, ported in
//! idea from version 2's chat-image tools.
//!
//! The list comes from the conversation the call was made in (`ToolContext::seen`), so it names
//! exactly the pictures the model was shown, labelled as they were; `read_image` reopens one from
//! the chat's saved originals as a fresh viewing copy. Both only read, so they run side by side
//! and Plan mode allows them. They are deferred: a model reaches for them only once an elision
//! line has told it a picture exists.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use serde_json::{Value, json};
use wack_llm::{Block, Role};

use super::{reference, view};
use crate::tool::{Loading, Tool, ToolContext, ToolOutput, ToolSpec};

pub const LIST_IMAGES: &str = "list_images";
pub const READ_IMAGE: &str = "read_image";

/// The two tools.
pub fn tools() -> Vec<std::sync::Arc<dyn Tool>> {
    vec![std::sync::Arc::new(ListImages), std::sync::Arc::new(ReadImage)]
}

struct ListImages;
struct ReadImage;

/// One picture of the conversation, as `list_images` shows it.
#[derive(Debug, PartialEq)]
struct Listed {
    id: String,
    label: Option<String>,
    /// The tool that returned it, or `None` for one the user attached.
    tool: Option<String>,
}

/// The conversation's saved pictures, newest first, each once.
fn listed(messages: &[wack_llm::Message]) -> Vec<Listed> {
    let names: HashMap<&str, &str> =
        messages.iter().filter(|message| message.role == Role::Assistant).flat_map(|message| message.tool_uses()).map(|(id, name, _)| (id, name)).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for block in messages.iter().rev().flat_map(|message| message.content.iter().rev()) {
        let found: Vec<(&Option<String>, &Option<String>, Option<String>)> = match block {
            Block::ToolResult { tool_use_id, images, .. } => {
                images.iter().rev().map(|image| (&image.id, &image.label, names.get(tool_use_id.as_str()).map(|name| (*name).to_string()))).collect()
            }
            Block::Image { id, label, .. } => vec![(id, label, None)],
            _ => Vec::new(),
        };
        for (id, label, tool) in found {
            if let Some(id) = id
                && seen.insert(id.clone())
            {
                out.push(Listed { id: id.clone(), label: label.clone(), tool });
            }
        }
    }
    out
}

#[async_trait]
impl Tool for ListImages {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: LIST_IMAGES.into(),
            description: "List the pictures in this conversation, newest first: the ones tools returned (screenshots, image files read) and the ones the user attached. Each has a reference such as img:3f2a9c01b4d7 that read_image reopens.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "offset": { "type": "integer", "description": "How many of the newest to skip. Default 0." },
                    "limit": { "type": "integer", "description": "How many to list, at most 100. Default 20." }
                }
            }),
            loading: Loading::Deferred,
            parallel_safe: true,
            hint: "list this conversation's pictures (screenshots, image files, the user's attachments) with their references".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let offset = input.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 100) as usize;
        let all = listed(context.seen.messages);
        if all.is_empty() {
            return Ok("There are no pictures in this conversation.".into());
        }
        let lines: Vec<String> = all
            .iter()
            .skip(offset)
            .take(limit)
            .map(|item| {
                let what = item.label.as_deref().unwrap_or("a picture");
                let from = match &item.tool { Some(tool) => format!("from {tool}"), None => "attached by the user".to_string() };
                format!("{}  {what} ({from})", reference(&item.id))
            })
            .collect();
        if lines.is_empty() {
            return Ok(format!("There are {} pictures; none past {offset}.", all.len()).into());
        }
        let mut text = lines.join("\n");
        if offset + lines.len() < all.len() {
            text.push_str(&format!("\n({} more: list again with offset {}.)", all.len() - offset - lines.len(), offset + lines.len()));
        }
        Ok(text.into())
    }
}

#[async_trait]
impl Tool for ReadImage {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: READ_IMAGE.into(),
            description: "Reopen a picture from this conversation by its reference (img:…), as an elision line or list_images gives it. Use it only when you need to look at a picture again: a picture you have already seen and described rarely needs reopening.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "The picture's reference, such as img:3f2a9c01b4d7." }
                },
                "required": ["ref"]
            }),
            loading: Loading::Deferred,
            parallel_safe: true,
            hint: "reopen an earlier picture of this conversation by its img: reference".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let wanted = input.get("ref").and_then(Value::as_str).ok_or("Give `ref`, the picture's reference (img:…).")?;
        let path = context.images.find(wanted)?;
        let bytes = tokio::fs::read(&path).await.map_err(|error| format!("Could not open the picture: {error}."))?;
        let id = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_string();
        let label = listed(context.seen.messages).into_iter().find(|item| item.id == id).and_then(|item| item.label);
        let (width, height) = view::dimensions(&bytes).unwrap_or_default();
        let name = reference(&id);
        let what = label.as_deref().map(|label| format!(": {label}")).unwrap_or_default();
        let text = format!("{name}{what}, {width} × {height} pixels. The original is at {}.", path.display());
        if !context.seen.settings.is_none_or(|settings| settings.accepts_images) {
            return Ok(format!("{text} This model can't view pictures.").into());
        }
        let media = view::media_type(&bytes).ok_or("The saved picture is not one a model can view.")?;
        let mut image = wack_llm::Image::new(media, view::encode(&bytes));
        image.label = Some(label.unwrap_or_else(|| format!("reopened {name}")));
        Ok(ToolOutput::with_images(text, vec![image]))
    }
}

#[cfg(test)]
mod tests {
    use wack_llm::{Image, Message};

    use super::*;

    fn picture(id: &str, label: &str) -> Image {
        let mut image = Image::new("image/png", "AAAA").labelled(label);
        image.id = Some(id.into());
        image
    }

    #[test]
    fn lists_newest_first_once_each_naming_the_tool() {
        let messages = vec![
            Message { role: Role::User, content: vec![Block::Image { media_type: "image/png".into(), data: "A".into(), label: Some("design.png".into()), id: Some("aaaaaaaaaaaaaaaa".into()) }] },
            Message { role: Role::Assistant, content: vec![Block::ToolUse { id: "t1".into(), name: "browser_screenshot".into(), input: json!({}) }] },
            Message {
                role: Role::User,
                content: vec![Block::ToolResult {
                    tool_use_id: "t1".into(),
                    content: String::new(),
                    is_error: false,
                    images: vec![picture("bbbbbbbbbbbbbbbb", "screenshot of a"), picture("aaaaaaaaaaaaaaaa", "again")],
                }],
            },
        ];
        let found = listed(&messages);
        assert_eq!(
            found,
            vec![
                Listed { id: "aaaaaaaaaaaaaaaa".into(), label: Some("again".into()), tool: Some("browser_screenshot".into()) },
                Listed { id: "bbbbbbbbbbbbbbbb".into(), label: Some("screenshot of a".into()), tool: Some("browser_screenshot".into()) },
            ]
        );
    }

    #[tokio::test]
    async fn reopens_a_saved_picture_by_its_reference() {
        let folder = tempfile::tempdir().unwrap();
        let store = super::super::Images::at(folder.path());
        let png = b"\x89PNG\r\n\x1a\n-not-really".to_vec();
        let id = store.save(&png, "image/png").unwrap();
        let mut context = ToolContext::detached(folder.path());
        context.images = store;
        let output = ReadImage.run(json!({ "ref": reference(&id) }), &context).await.unwrap();
        assert_eq!(output.images.len(), 1);
        assert_eq!(view::decode(&output.images[0].data).unwrap(), png);
        assert!(output.text.starts_with(&reference(&id)));
        assert!(ReadImage.run(json!({ "ref": "img:0000000000" }), &context).await.unwrap_err().contains("No picture"));
    }
}
