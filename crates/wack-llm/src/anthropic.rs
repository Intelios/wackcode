//! Anthropic's Messages API (`anthropic-version: 2026-10-21`).
//!
//! Sampling parameters are never sent: this API version rejects them. Reasoning is requested as
//! adaptive thinking with an effort level, the only form current models accept.

use serde_json::{Map, Value, json};

use crate::{
    Block, Delta, DeltaSink, Effort, LlmError, Message, Request, Response, Role, StopReason, Usage, WireFormat,
    error::ErrorKind,
};

pub const VERSION: &str = "2026-10-21";
const DEFAULT_MAX_TOKENS: u32 = 32_000;
const SEARCH_TOOL: &str = "tool_search_tool_bm25_20251119";

/// Builds the request. `first_party` is true for Anthropic's own endpoint, which gets the newer
/// request fields (automatic caching, eager tool input, tool search). A proxy or another host
/// speaking this format gets only fields that have been part of it for years, since an unknown
/// field is usually refused outright.
pub fn body(request: &Request, first_party: bool) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert("max_tokens".into(), json!(request.max_output_tokens.unwrap_or(DEFAULT_MAX_TOKENS)));
    body.insert("stream".into(), json!(true));
    let cache = json!({ "type": "ephemeral" });
    if first_party {
        // The provider places the cache breakpoint on the last cacheable block itself.
        body.insert("cache_control".into(), cache.clone());
    }

    let mut system: Vec<Value> =
        request.system.iter().map(|part| json!({ "type": "text", "text": part.text })).collect();
    if !first_party && let Some(last) = system.last_mut() {
        last["cache_control"] = cache.clone();
    }
    if !system.is_empty() {
        body.insert("system".into(), Value::Array(system));
    }

    let mut tools = Vec::new();
    let mut any_deferred = false;
    for tool in &request.tools {
        let mut entry = json!({ "name": tool.name, "description": tool.description, "input_schema": tool.input_schema });
        if first_party {
            // Large inputs (a whole file) stream as they are written instead of arriving in one
            // burst. The price is that the input is no longer validated for us: see `tool_input`.
            entry["eager_input_streaming"] = json!(true);
            if tool.deferred {
                entry["defer_loading"] = json!(true);
                any_deferred = true;
            }
        }
        tools.push(entry);
    }
    if any_deferred {
        tools.push(json!({ "type": SEARCH_TOOL, "name": "tool_search_tool_bm25" }));
    }
    if !tools.is_empty() {
        body.insert("tools".into(), Value::Array(tools));
    }

    if let Some(effort) = request.effort {
        // Messages has no minimal effort: its lowest is low.
        let effort = if effort == Effort::Minimal { Effort::Low } else { effort };
        body.insert("thinking".into(), json!({ "type": "adaptive", "display": "summarized" }));
        body.insert("output_config".into(), json!({ "effort": effort.as_str() }));
    }

    let mut messages: Vec<Value> = request.messages.iter().filter_map(message).collect();
    // Marks the end of the conversation so far, which the next request will share.
    if !first_party
        && let Some(block) = messages.last_mut().and_then(|message| message["content"].as_array_mut()).and_then(|content| content.last_mut())
    {
        block["cache_control"] = cache;
    }
    body.insert("messages".into(), Value::Array(messages));
    Value::Object(body)
}

fn message(message: &Message) -> Option<Value> {
    let content: Vec<Value> = message.content.iter().filter_map(block).collect();
    if content.is_empty() {
        return None;
    }
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    Some(json!({ "role": role, "content": content }))
}

fn block(block: &Block) -> Option<Value> {
    Some(match block {
        Block::Text { text } if text.is_empty() => return None,
        Block::Text { text } => json!({ "type": "text", "text": text }),
        Block::Thinking { text, signature: Some(signature) } => {
            json!({ "type": "thinking", "thinking": text, "signature": signature })
        }
        Block::Thinking { signature: None, .. } => return None,
        Block::ToolUse { id, name, input } => {
            let input = if input.is_object() { input.clone() } else { json!({}) };
            json!({ "type": "tool_use", "id": id, "name": name, "input": input })
        }
        Block::ToolResult { tool_use_id, content, is_error, images } if images.is_empty() => {
            json!({ "type": "tool_result", "tool_use_id": tool_use_id, "content": content, "is_error": is_error })
        }
        // Pictures go inside the result, after its text, so the model reads them as the tool's.
        Block::ToolResult { tool_use_id, content, is_error, images } => {
            let mut parts = Vec::new();
            if !content.is_empty() {
                parts.push(json!({ "type": "text", "text": content }));
            }
            parts.extend(images.iter().map(|image| image_block(&image.media_type, &image.data)));
            json!({ "type": "tool_result", "tool_use_id": tool_use_id, "content": parts, "is_error": is_error })
        }
        Block::Image { media_type, data, .. } => image_block(media_type, data),
        Block::Opaque { format: WireFormat::AnthropicMessages, value } => value.clone(),
        Block::Opaque { .. } => return None,
    })
}

fn image_block(media_type: &str, data: &str) -> Value {
    json!({ "type": "image", "source": { "type": "base64", "media_type": media_type, "data": data } })
}

enum Partial {
    Text(String),
    Thinking { text: String, signature: Option<String> },
    ToolUse { id: String, name: String, json: String },
    /// A block we do not model, kept whole. `json` collects its input when it streams one.
    Other { value: Value, json: String },
}

#[derive(Default)]
pub struct Decoder {
    blocks: Vec<Option<Partial>>,
    stop: Option<String>,
    usage: Option<Usage>,
}

impl Decoder {
    pub fn event(&mut self, data: &Value, deltas: DeltaSink<'_>) -> Result<(), LlmError> {
        match data["type"].as_str().unwrap_or_default() {
            "message_start" => self.read_usage(&data["message"]["usage"]),
            "content_block_start" => {
                let index = data["index"].as_u64().unwrap_or(0) as usize;
                let start = &data["content_block"];
                let partial = match start["type"].as_str().unwrap_or_default() {
                    "text" => Partial::Text(text(&start["text"])),
                    "thinking" => Partial::Thinking {
                        text: text(&start["thinking"]),
                        signature: start["signature"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
                    },
                    "tool_use" => {
                        let (id, name) = (text(&start["id"]), text(&start["name"]));
                        deltas(Delta::ToolStart { id: id.clone(), name: name.clone() });
                        Partial::ToolUse { id, name, json: String::new() }
                    }
                    _ => Partial::Other { value: start.clone(), json: String::new() },
                };
                if self.blocks.len() <= index {
                    self.blocks.resize_with(index + 1, || None);
                }
                self.blocks[index] = Some(partial);
            }
            "content_block_delta" => {
                let index = data["index"].as_u64().unwrap_or(0) as usize;
                let delta = &data["delta"];
                let Some(Some(partial)) = self.blocks.get_mut(index) else {
                    return Ok(());
                };
                match (delta["type"].as_str().unwrap_or_default(), partial) {
                    ("text_delta", Partial::Text(buffer)) => {
                        let piece = text(&delta["text"]);
                        buffer.push_str(&piece);
                        deltas(Delta::Text(piece));
                    }
                    ("thinking_delta", Partial::Thinking { text: buffer, .. }) => {
                        let piece = text(&delta["thinking"]);
                        buffer.push_str(&piece);
                        deltas(Delta::Thinking(piece));
                    }
                    ("signature_delta", Partial::Thinking { signature, .. }) => {
                        signature.get_or_insert_with(String::new).push_str(&text(&delta["signature"]));
                    }
                    ("input_json_delta", Partial::ToolUse { id, json, .. }) => {
                        let piece = text(&delta["partial_json"]);
                        json.push_str(&piece);
                        deltas(Delta::ToolInput { id: id.clone(), json: piece });
                    }
                    ("input_json_delta", Partial::Other { json, .. }) => json.push_str(&text(&delta["partial_json"])),
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(stop) = data["delta"]["stop_reason"].as_str() {
                    self.stop = Some(stop.to_string());
                }
                self.read_usage(&data["usage"]);
            }
            "error" => {
                let error = &data["error"];
                return Err(LlmError::from_stream(
                    error["type"].as_str().unwrap_or_default(),
                    error["message"].as_str().unwrap_or_default(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    /// Later events repeat earlier counts and add the output, so each field keeps its latest value.
    fn read_usage(&mut self, usage: &Value) {
        if !usage.is_object() {
            return;
        }
        let current = self.usage.get_or_insert_with(Usage::default);
        let set = |field: &mut u64, key: &str| {
            if let Some(count) = usage[key].as_u64() {
                *field = count;
            }
        };
        set(&mut current.input, "input_tokens");
        set(&mut current.output, "output_tokens");
        set(&mut current.cache_read, "cache_read_input_tokens");
        set(&mut current.cache_write, "cache_creation_input_tokens");
    }

    pub fn finish(self) -> Result<Response, LlmError> {
        let Some(stop) = self.stop else {
            return Err(LlmError::new(ErrorKind::Network, "The provider's reply ended before it was complete."));
        };
        let mut content = Vec::new();
        for partial in self.blocks.into_iter().flatten() {
            match partial {
                Partial::Text(text) if text.is_empty() => {}
                Partial::Text(text) => content.push(Block::Text { text }),
                Partial::Thinking { text, signature } => {
                    if !text.is_empty() || signature.is_some() {
                        content.push(Block::Thinking { text, signature });
                    }
                }
                Partial::ToolUse { id, name, json } => {
                    content.push(Block::ToolUse { id, name, input: tool_input(&json) });
                }
                Partial::Other { mut value, json } => {
                    if !json.is_empty() {
                        value["input"] = tool_input(&json);
                    }
                    content.push(Block::Opaque { format: WireFormat::AnthropicMessages, value });
                }
            }
        }
        let stop = match stop.as_str() {
            "end_turn" | "stop_sequence" => StopReason::EndTurn,
            "tool_use" => StopReason::ToolUse,
            "max_tokens" => StopReason::MaxTokens,
            "refusal" => StopReason::Refusal,
            other => StopReason::Other(other.to_string()),
        };
        Ok(Response { message: Message { role: Role::Assistant, content }, stop, usage: self.usage })
    }
}

/// Parses a tool's streamed input. With eager streaming the provider no longer checks it, so a
/// reply cut short leaves broken JSON: that comes back as the raw string for the caller to refuse.
pub(crate) fn tool_input(json: &str) -> Value {
    if json.trim().is_empty() {
        return json!({});
    }
    match serde_json::from_str::<Value>(json) {
        Ok(value) if value.is_object() => value,
        _ => Value::String(json.to_string()),
    }
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SystemPart, ToolDef, testing};

    fn request() -> Request {
        Request {
            model: "claude-opus-5-5".into(),
            system: vec![SystemPart { text: "Be brief.".into() }],
            messages: vec![Message::user("hi")],
            tools: vec![
                ToolDef { name: "read".into(), description: "Reads".into(), input_schema: json!({"type": "object"}), deferred: false },
                ToolDef { name: "browser".into(), description: "Browses".into(), input_schema: json!({"type": "object"}), deferred: true },
            ],
            max_output_tokens: None,
            effort: Some(Effort::High),
        }
    }

    #[test]
    fn body_defers_tools_only_with_native_search() {
        let native = body(&request(), true);
        assert_eq!(native["tools"][1]["defer_loading"], true);
        assert_eq!(native["tools"][2]["type"], SEARCH_TOOL);
        assert_eq!(native["thinking"]["type"], "adaptive");
        assert_eq!(native["output_config"]["effort"], "high");
        assert!(native.get("temperature").is_none());

        assert_eq!(native["tools"][0]["eager_input_streaming"], true);
        assert_eq!(native["cache_control"]["type"], "ephemeral");
        assert!(native["system"][0].get("cache_control").is_none());

        // Another host gets the long-standing form: explicit cache marks and no newer fields.
        let plain = body(&request(), false);
        assert!(plain["tools"][1].get("defer_loading").is_none());
        assert!(plain["tools"][0].get("eager_input_streaming").is_none());
        assert_eq!(plain["tools"].as_array().unwrap().len(), 2);
        assert!(plain.get("cache_control").is_none());
        assert_eq!(plain["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(plain["messages"][0]["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn unsigned_thinking_and_foreign_blocks_are_not_replayed() {
        let mut request = request();
        request.messages.push(Message {
            role: Role::Assistant,
            content: vec![
                Block::Thinking { text: "hm".into(), signature: None },
                Block::Opaque { format: WireFormat::OpenaiResponses, value: json!({"type": "reasoning"}) },
                Block::ToolUse { id: "t1".into(), name: "read".into(), input: Value::String("{\"pa".into()) },
            ],
        });
        let body = body(&request, false);
        let content = body["messages"][1]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["input"], json!({}));
    }

    #[test]
    fn tool_result_images_go_inside_the_result() {
        let mut request = request();
        let image = crate::Image::new("image/png", "AAAA");
        request.messages.push(Message {
            role: Role::User,
            content: vec![
                Block::ToolResult { tool_use_id: "t1".into(), content: "Shot.".into(), is_error: false, images: vec![image.clone()] },
                Block::ToolResult { tool_use_id: "t2".into(), content: "Plain.".into(), is_error: false, images: vec![] },
                Block::ToolResult { tool_use_id: "t3".into(), content: String::new(), is_error: false, images: vec![image] },
            ],
        });
        let body = body(&request, true);
        let content = &body["messages"][1]["content"];
        assert_eq!(content[0]["content"][0], json!({"type": "text", "text": "Shot."}));
        assert_eq!(content[0]["content"][1]["type"], "image");
        assert_eq!(content[0]["content"][1]["source"], json!({"type": "base64", "media_type": "image/png", "data": "AAAA"}));
        // Without pictures the result is the plain string it always was.
        assert_eq!(content[1]["content"], "Plain.");
        // An empty text sends no empty text block.
        assert_eq!(content[2]["content"].as_array().unwrap().len(), 1);
        assert_eq!(content[2]["content"][0]["type"], "image");
    }

    const STREAM: &str = r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":12,"cache_read_input_tokens":300,"cache_creation_input_tokens":40,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Look first."}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Reading."}}

event: content_block_start
data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"read","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}

event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\"a.rs\"}"}}

event: content_block_start
data: {"type":"content_block_start","index":3,"content_block":{"type":"server_tool_use","id":"srv_1","name":"tool_search_tool_bm25","input":{}}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":25}}

event: message_stop
data: {"type":"message_stop"}

"#;

    #[test]
    fn decodes_a_reply_with_thinking_text_and_a_tool_call() {
        let collected = testing::Collected::default();
        let mut decoder = Decoder::default();
        for event in testing::events(STREAM) {
            decoder.event(&event, &|delta| collected.push(delta)).unwrap();
        }
        let response = decoder.finish().unwrap();
        assert_eq!(response.stop, StopReason::ToolUse);
        assert_eq!(response.usage, Some(Usage { input: 12, output: 25, cache_read: 300, cache_write: 40 }));
        assert_eq!(collected.text(), "Reading.");
        let content = &response.message.content;
        assert_eq!(content[0], Block::Thinking { text: "Look first.".into(), signature: Some("sig".into()) });
        assert_eq!(content[2], Block::ToolUse { id: "toolu_1".into(), name: "read".into(), input: json!({"path": "a.rs"}) });
        assert!(matches!(&content[3], Block::Opaque { format: WireFormat::AnthropicMessages, .. }));
    }

    #[test]
    fn a_stream_cut_short_is_a_network_error_and_broken_input_stays_raw() {
        assert_eq!(Decoder::default().finish().unwrap_err().kind, ErrorKind::Network);
        assert_eq!(tool_input("{\"path\": \"a"), Value::String("{\"path\": \"a".into()));
        assert_eq!(tool_input(""), json!({}));
    }

    #[test]
    fn an_error_event_is_sorted_by_its_type() {
        let event = json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}});
        let error = Decoder::default().event(&event, &|_| {}).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unavailable);
    }
}
