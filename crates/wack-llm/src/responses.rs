//! OpenAI's Responses API, stateless: `store` is off and the whole conversation is sent each time.
//!
//! Reasoning items come back encrypted and are replayed as opaque blocks, which is what keeps a
//! reasoning model coherent across tool calls without the provider storing the conversation.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::{
    Block, Delta, DeltaSink, Effort, LlmError, Message, Request, Response, Role, StopReason, Usage, WireFormat,
    anthropic::tool_input, error::ErrorKind,
};

pub fn body(request: &Request) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert("stream".into(), json!(true));
    body.insert("store".into(), json!(false));
    body.insert("include".into(), json!(["reasoning.encrypted_content"]));
    if let Some(max) = request.max_output_tokens {
        body.insert("max_output_tokens".into(), json!(max));
    }
    if let Some(effort) = request.effort {
        let level = match effort {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh | Effort::Max => "xhigh",
        };
        body.insert("reasoning".into(), json!({ "effort": level, "summary": "auto" }));
    }
    let system: Vec<&str> = request.system.iter().map(|part| part.text.as_str()).collect();
    if !system.is_empty() {
        body.insert("instructions".into(), json!(system.join("\n\n")));
    }

    let mut input = Vec::new();
    for message in &request.messages {
        push_message(message, &mut input);
    }
    body.insert("input".into(), Value::Array(input));

    let tools: Vec<Value> = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
                "strict": false,
            })
        })
        .collect();
    if !tools.is_empty() {
        body.insert("tools".into(), Value::Array(tools));
    }
    Value::Object(body)
}

fn push_message(message: &Message, out: &mut Vec<Value>) {
    let mut parts = Vec::new();
    let flush = |parts: &mut Vec<Value>, out: &mut Vec<Value>| {
        if !parts.is_empty() {
            let role = if message.role == Role::User { "user" } else { "assistant" };
            out.push(json!({ "role": role, "content": std::mem::take(parts) }));
        }
    };
    for block in &message.content {
        match (block, message.role) {
            (Block::Text { text }, _) if text.is_empty() => {}
            (Block::Text { text }, Role::User) => parts.push(json!({ "type": "input_text", "text": text })),
            (Block::Text { text }, Role::Assistant) => parts.push(json!({ "type": "output_text", "text": text })),
            (Block::Image { media_type, data }, Role::User) => parts.push(json!({
                "type": "input_image",
                "image_url": format!("data:{media_type};base64,{data}"),
            })),
            (Block::ToolUse { id, name, input }, _) => {
                flush(&mut parts, out);
                let arguments = if input.is_object() { input.to_string() } else { "{}".to_string() };
                out.push(json!({ "type": "function_call", "call_id": id, "name": name, "arguments": arguments }));
            }
            (Block::ToolResult { tool_use_id, content, .. }, _) => {
                flush(&mut parts, out);
                out.push(json!({ "type": "function_call_output", "call_id": tool_use_id, "output": content }));
            }
            (Block::Opaque { format: WireFormat::OpenaiResponses, value }, _) => {
                flush(&mut parts, out);
                out.push(value.clone());
            }
            _ => {}
        }
    }
    flush(&mut parts, out);
}

#[derive(Default)]
pub struct Decoder {
    items: Vec<Value>,
    /// The stream names a call by its item id; everything else uses its call id.
    call_ids: HashMap<String, String>,
    finished: Option<Value>,
}

impl Decoder {
    pub fn event(&mut self, data: &Value, deltas: DeltaSink<'_>) -> Result<(), LlmError> {
        match data["type"].as_str().unwrap_or_default() {
            "response.output_text.delta" => deltas(Delta::Text(text(&data["delta"]))),
            "response.reasoning_summary_text.delta" => deltas(Delta::Thinking(text(&data["delta"]))),
            "response.output_item.added" => {
                let item = &data["item"];
                if item["type"] == "function_call" {
                    let call_id = text(&item["call_id"]);
                    self.call_ids.insert(text(&item["id"]), call_id.clone());
                    deltas(Delta::ToolStart { id: call_id, name: text(&item["name"]) });
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(call_id) = self.call_ids.get(data["item_id"].as_str().unwrap_or_default()) {
                    deltas(Delta::ToolInput { id: call_id.clone(), json: text(&data["delta"]) });
                }
            }
            "response.output_item.done" => self.items.push(data["item"].clone()),
            "response.completed" | "response.incomplete" => self.finished = Some(data["response"].clone()),
            "response.failed" => {
                let error = &data["response"]["error"];
                return Err(LlmError::from_stream(
                    error["code"].as_str().unwrap_or_default(),
                    error["message"].as_str().unwrap_or_default(),
                ));
            }
            "error" => {
                return Err(LlmError::from_stream(
                    data["code"].as_str().unwrap_or_default(),
                    data["message"].as_str().unwrap_or_default(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    pub fn finish(self) -> Result<Response, LlmError> {
        let Some(finished) = self.finished else {
            return Err(LlmError::new(ErrorKind::Network, "The provider's reply ended before it was complete."));
        };
        let mut content = Vec::new();
        let mut called = false;
        for item in self.items {
            match item["type"].as_str().unwrap_or_default() {
                "message" => {
                    for part in item["content"].as_array().into_iter().flatten() {
                        let piece = part["text"].as_str().or(part["refusal"].as_str()).unwrap_or_default();
                        if !piece.is_empty() {
                            content.push(Block::Text { text: piece.to_string() });
                        }
                    }
                }
                "function_call" => {
                    called = true;
                    content.push(Block::ToolUse {
                        id: text(&item["call_id"]),
                        name: text(&item["name"]),
                        input: tool_input(item["arguments"].as_str().unwrap_or_default()),
                    });
                }
                kind => {
                    if kind == "reasoning" {
                        let summary: Vec<&str> = item["summary"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|part| part["text"].as_str())
                            .collect();
                        if !summary.is_empty() {
                            content.push(Block::Thinking { text: summary.join("\n\n"), signature: None });
                        }
                    }
                    content.push(Block::Opaque { format: WireFormat::OpenaiResponses, value: item });
                }
            }
        }
        let stop = if called {
            StopReason::ToolUse
        } else if finished["status"] == "incomplete" {
            match finished["incomplete_details"]["reason"].as_str().unwrap_or_default() {
                "max_output_tokens" => StopReason::MaxTokens,
                "content_filter" => StopReason::Refusal,
                other => StopReason::Other(other.to_string()),
            }
        } else {
            StopReason::EndTurn
        };
        let usage = finished.get("usage").filter(|usage| usage.is_object()).map(|usage| {
            let input = usage["input_tokens"].as_u64().unwrap_or(0);
            let cached = usage["input_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0);
            Usage {
                input: input.saturating_sub(cached),
                output: usage["output_tokens"].as_u64().unwrap_or(0),
                cache_read: cached,
                cache_write: 0,
            }
        });
        Ok(Response { message: Message { role: Role::Assistant, content }, stop, usage })
    }
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[test]
    fn body_replays_reasoning_and_calls_as_items() {
        let request = Request {
            model: "m".into(),
            system: vec![],
            messages: vec![
                Message::user("go"),
                Message {
                    role: Role::Assistant,
                    content: vec![
                        Block::Opaque { format: WireFormat::OpenaiResponses, value: json!({"type": "reasoning", "id": "rs_1"}) },
                        Block::Opaque { format: WireFormat::AnthropicMessages, value: json!({"type": "server_tool_use"}) },
                        Block::Text { text: "Looking.".into() },
                        Block::ToolUse { id: "call_1".into(), name: "ls".into(), input: json!({}) },
                    ],
                },
                Message {
                    role: Role::User,
                    content: vec![Block::ToolResult { tool_use_id: "call_1".into(), content: "a.rs".into(), is_error: false }],
                },
            ],
            tools: vec![],
            max_output_tokens: None,
            effort: Some(Effort::Max),
        };
        let body = body(&request);
        let kinds: Vec<_> =
            body["input"].as_array().unwrap().iter().map(|item| item["type"].as_str().unwrap_or("message")).collect();
        assert_eq!(kinds, ["message", "reasoning", "message", "function_call", "function_call_output"]);
        assert_eq!(body["store"], false);
        assert_eq!(body["reasoning"]["effort"], "xhigh");
    }

    const STREAM: &str = r#"data: {"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read","arguments":""}}

data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"path\":\"a.rs\"}"}

data: {"type":"response.output_item.done","item":{"type":"reasoning","id":"rs_1","encrypted_content":"abc","summary":[{"type":"summary_text","text":"Plan."}]}}

data: {"type":"response.output_text.delta","delta":"Reading."}

data: {"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"Reading."}]}}

data: {"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read","arguments":"{\"path\":\"a.rs\"}"}}

data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":50,"output_tokens":9,"input_tokens_details":{"cached_tokens":30}}}}

"#;

    #[test]
    fn decodes_items_and_names_calls_by_call_id() {
        let collected = testing::Collected::default();
        let mut decoder = Decoder::default();
        for event in testing::events(STREAM) {
            decoder.event(&event, &|delta| collected.push(delta)).unwrap();
        }
        let response = decoder.finish().unwrap();
        assert_eq!(response.stop, StopReason::ToolUse);
        assert_eq!(response.usage, Some(Usage { input: 20, output: 9, cache_read: 30, cache_write: 0 }));
        assert!(collected.0.lock().unwrap().contains(&Delta::ToolInput { id: "call_1".into(), json: "{\"path\":\"a.rs\"}".into() }));
        let content = &response.message.content;
        assert_eq!(content[0], Block::Thinking { text: "Plan.".into(), signature: None });
        assert!(matches!(&content[1], Block::Opaque { format: WireFormat::OpenaiResponses, .. }));
        assert_eq!(content[3], Block::ToolUse { id: "call_1".into(), name: "read".into(), input: json!({"path": "a.rs"}) });
    }
}
