//! Chat Completions, as spoken by OpenAI and most compatible servers.
//!
//! This is the lowest common denominator, so the request stays conservative: the `system` role
//! (strict servers reject `developer`), no reasoning replay, and a token cap only when asked for.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::{
    Block, Delta, DeltaSink, Effort, LlmError, Message, Request, Response, Role, StopReason, Usage,
    anthropic::tool_input, error::ErrorKind,
};

pub fn body(request: &Request, is_openai: bool) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert("stream".into(), json!(true));
    body.insert("stream_options".into(), json!({ "include_usage": true }));
    if let Some(max) = request.max_output_tokens {
        // OpenAI's own reasoning models reject the older name; most other servers know only it.
        let key = if is_openai { "max_completion_tokens" } else { "max_tokens" };
        body.insert(key.into(), json!(max));
    }
    if let Some(effort) = request.effort {
        let level = match effort {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High | Effort::Xhigh | Effort::Max => "high",
        };
        body.insert("reasoning_effort".into(), json!(level));
    }

    let mut messages = Vec::new();
    let system: Vec<&str> = request.system.iter().map(|part| part.text.as_str()).collect();
    if !system.is_empty() {
        messages.push(json!({ "role": "system", "content": system.join("\n\n") }));
    }
    for message in &request.messages {
        push_message(message, &mut messages);
    }
    body.insert("messages".into(), Value::Array(messages));

    let tools: Vec<Value> = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": { "name": tool.name, "description": tool.description, "parameters": tool.input_schema },
            })
        })
        .collect();
    if !tools.is_empty() {
        body.insert("tools".into(), Value::Array(tools));
    }
    Value::Object(body)
}

fn push_message(message: &Message, out: &mut Vec<Value>) {
    match message.role {
        Role::Assistant => {
            let text = message.text();
            let calls: Vec<Value> = message
                .tool_uses()
                .map(|(id, name, input)| {
                    let arguments = if input.is_object() { input.to_string() } else { "{}".to_string() };
                    json!({ "id": id, "type": "function", "function": { "name": name, "arguments": arguments } })
                })
                .collect();
            if text.is_empty() && calls.is_empty() {
                return;
            }
            let mut entry = json!({ "role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) } });
            if !calls.is_empty() {
                entry["tool_calls"] = Value::Array(calls);
            }
            out.push(entry);
        }
        Role::User => {
            // Tool results are their own messages here and must come straight after the call.
            let mut parts = Vec::new();
            for block in &message.content {
                match block {
                    Block::ToolResult { tool_use_id, content, .. } => {
                        out.push(json!({ "role": "tool", "tool_call_id": tool_use_id, "content": content }));
                    }
                    Block::Text { text } if !text.is_empty() => parts.push(json!({ "type": "text", "text": text })),
                    Block::Image { media_type, data } => parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:{media_type};base64,{data}") },
                    })),
                    _ => {}
                }
            }
            match parts.as_slice() {
                [] => {}
                [only] if only["type"] == "text" => out.push(json!({ "role": "user", "content": only["text"] })),
                _ => out.push(json!({ "role": "user", "content": parts })),
            }
        }
    }
}

#[derive(Default)]
struct Call {
    id: String,
    name: String,
    arguments: String,
    announced: bool,
}

#[derive(Default)]
pub struct Decoder {
    text: String,
    thinking: String,
    calls: BTreeMap<u64, Call>,
    finish: Option<String>,
    usage: Option<Usage>,
}

impl Decoder {
    pub fn event(&mut self, data: &Value, deltas: DeltaSink<'_>) -> Result<(), LlmError> {
        if let Some(error) = data.get("error").filter(|error| !error.is_null()) {
            return Err(LlmError::from_stream(
                error["type"].as_str().or(error["code"].as_str()).unwrap_or_default(),
                error["message"].as_str().unwrap_or_default(),
            ));
        }
        if let Some(usage) = data.get("usage").filter(|usage| usage.is_object()) {
            let prompt = usage["prompt_tokens"].as_u64().unwrap_or(0);
            let cached = usage["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0);
            self.usage = Some(Usage {
                input: prompt.saturating_sub(cached),
                output: usage["completion_tokens"].as_u64().unwrap_or(0),
                cache_read: cached,
                cache_write: 0,
            });
        }
        let choice = &data["choices"][0];
        if let Some(finish) = choice["finish_reason"].as_str() {
            self.finish = Some(finish.to_string());
        }
        let delta = &choice["delta"];
        if let Some(piece) = delta["content"].as_str().filter(|piece| !piece.is_empty()) {
            self.text.push_str(piece);
            deltas(Delta::Text(piece.to_string()));
        }
        // Servers disagree on the name of the reasoning field.
        for key in ["reasoning_content", "reasoning"] {
            if let Some(piece) = delta[key].as_str().filter(|piece| !piece.is_empty()) {
                self.thinking.push_str(piece);
                deltas(Delta::Thinking(piece.to_string()));
            }
        }
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let entry = self.calls.entry(call["index"].as_u64().unwrap_or(0)).or_default();
            if let Some(id) = call["id"].as_str().filter(|id| !id.is_empty()) {
                entry.id = id.to_string();
            }
            if let Some(name) = call["function"]["name"].as_str().filter(|name| !name.is_empty()) {
                entry.name.push_str(name);
            }
            if !entry.announced && !entry.id.is_empty() && !entry.name.is_empty() {
                entry.announced = true;
                deltas(Delta::ToolStart { id: entry.id.clone(), name: entry.name.clone() });
            }
            if let Some(piece) = call["function"]["arguments"].as_str().filter(|piece| !piece.is_empty()) {
                entry.arguments.push_str(piece);
                if entry.announced {
                    deltas(Delta::ToolInput { id: entry.id.clone(), json: piece.to_string() });
                }
            }
        }
        Ok(())
    }

    pub fn finish(self) -> Result<Response, LlmError> {
        let Some(finish) = self.finish else {
            return Err(LlmError::new(ErrorKind::Network, "The provider's reply ended before it was complete."));
        };
        let mut content = Vec::new();
        if !self.thinking.is_empty() {
            content.push(Block::Thinking { text: self.thinking, signature: None });
        }
        if !self.text.is_empty() {
            content.push(Block::Text { text: self.text });
        }
        let mut called = false;
        for (index, call) in self.calls {
            called = true;
            // Some servers send no call id; the pairing with its result still needs one.
            let id = if call.id.is_empty() { format!("call_{index}") } else { call.id };
            content.push(Block::ToolUse { id, name: call.name, input: tool_input(&call.arguments) });
        }
        let stop = match finish.as_str() {
            _ if called => StopReason::ToolUse,
            "stop" => StopReason::EndTurn,
            "length" => StopReason::MaxTokens,
            "content_filter" => StopReason::Refusal,
            other => StopReason::Other(other.to_string()),
        };
        Ok(Response { message: Message { role: Role::Assistant, content }, stop, usage: self.usage })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SystemPart, ToolDef, testing};

    #[test]
    fn body_orders_tool_results_before_user_text() {
        let request = Request {
            model: "m".into(),
            system: vec![SystemPart { text: "A".into() }, SystemPart { text: "B".into() }],
            messages: vec![
                Message::user("go"),
                Message {
                    role: Role::Assistant,
                    content: vec![Block::ToolUse { id: "c1".into(), name: "ls".into(), input: json!({"path": "."}) }],
                },
                Message {
                    role: Role::User,
                    content: vec![
                        Block::ToolResult { tool_use_id: "c1".into(), content: "a.rs".into(), is_error: false },
                        Block::Text { text: "also check b".into() },
                    ],
                },
            ],
            tools: vec![ToolDef { name: "ls".into(), description: "Lists".into(), input_schema: json!({}), deferred: false }],
            max_output_tokens: Some(100),
            effort: None,
        };
        let body = body(&request, false);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["content"], "A\n\nB");
        assert_eq!(messages[2]["tool_calls"][0]["function"]["arguments"], "{\"path\":\".\"}");
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[4]["content"], "also check b");
        assert_eq!(body["max_tokens"], 100);
        assert!(super::body(&request, true).get("max_completion_tokens").is_some());
    }

    const STREAM: &str = r#"data: {"choices":[{"delta":{"role":"assistant","reasoning_content":"Think."}}]}

data: {"choices":[{"delta":{"content":"On it."}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":""}}]}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"a.rs\"}"}},{"index":1,"id":"call_b","function":{"name":"ls","arguments":"{}"}}]}}]}

data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}

data: {"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":60}}}

data: [DONE]

"#;

    #[test]
    fn decodes_parallel_tool_calls_and_usage() {
        let collected = testing::Collected::default();
        let mut decoder = Decoder::default();
        for event in testing::events(STREAM) {
            decoder.event(&event, &|delta| collected.push(delta)).unwrap();
        }
        let response = decoder.finish().unwrap();
        assert_eq!(response.stop, StopReason::ToolUse);
        assert_eq!(response.usage, Some(Usage { input: 40, output: 20, cache_read: 60, cache_write: 0 }));
        let calls: Vec<_> = response.message.tool_uses().collect();
        assert_eq!(calls, vec![("call_a", "read", &json!({"path": "a.rs"})), ("call_b", "ls", &json!({}))]);
        assert_eq!(collected.text(), "On it.");
    }
}
