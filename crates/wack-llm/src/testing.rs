//! A provider that plays back a script, for tests of everything above this crate.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use serde_json::Value;

use crate::{Block, Delta, DeltaSink, LlmError, Message, Provider, Request, Response, Role, StopReason, Usage};

/// One scripted reply to one request.
#[derive(Clone, Debug)]
pub enum Step {
    Reply(Response),
    Fail(LlmError),
    /// Streams `text` as a delta, then never finishes: for tests that stop a run mid-reply.
    Hang { text: String },
}

/// Answers each request with the next step of its script and remembers every request it was sent.
#[derive(Clone, Default)]
pub struct ScriptedProvider {
    steps: Arc<Mutex<VecDeque<Step>>>,
    requests: Arc<Mutex<Vec<Request>>>,
    native_tool_search: bool,
}

impl ScriptedProvider {
    pub fn new(steps: impl IntoIterator<Item = Step>) -> Self {
        Self { steps: Arc::new(Mutex::new(steps.into_iter().collect())), ..Self::default() }
    }

    pub fn with_native_tool_search(mut self) -> Self {
        self.native_tool_search = true;
        self
    }

    /// Every request received so far, oldest first.
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    async fn stream(&self, request: &Request, deltas: DeltaSink<'_>) -> Result<Response, LlmError> {
        self.requests.lock().unwrap().push(request.clone());
        let step = self.steps.lock().unwrap().pop_front();
        match step {
            None => panic!("the script ran out: the agent made a request the test did not expect"),
            Some(Step::Fail(error)) => Err(error),
            Some(Step::Hang { text }) => {
                deltas(Delta::Text(text));
                loop {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                }
            }
            Some(Step::Reply(response)) => {
                for block in &response.message.content {
                    match block {
                        Block::Text { text } => deltas(Delta::Text(text.clone())),
                        Block::Thinking { text, .. } => deltas(Delta::Thinking(text.clone())),
                        Block::ToolUse { id, name, input } => {
                            deltas(Delta::ToolStart { id: id.clone(), name: name.clone() });
                            deltas(Delta::ToolInput { id: id.clone(), json: input.to_string() });
                        }
                        _ => {}
                    }
                }
                Ok(response)
            }
        }
    }

    fn native_tool_search(&self) -> bool {
        self.native_tool_search
    }
}

/// A reply of plain text that ends the turn.
pub fn say(text: &str) -> Step {
    Step::Reply(Response {
        message: Message { role: Role::Assistant, content: vec![Block::Text { text: text.to_string() }] },
        stop: StopReason::EndTurn,
        usage: Some(Usage { input: 100, output: 10, cache_read: 0, cache_write: 0 }),
    })
}

/// A reply that calls tools: each entry is `(call id, tool name, input)`.
pub fn call(calls: &[(&str, &str, Value)]) -> Step {
    let content = calls
        .iter()
        .map(|(id, name, input)| Block::ToolUse { id: id.to_string(), name: name.to_string(), input: input.clone() })
        .collect();
    Step::Reply(Response {
        message: Message { role: Role::Assistant, content },
        stop: StopReason::ToolUse,
        usage: Some(Usage { input: 100, output: 10, cache_read: 0, cache_write: 0 }),
    })
}

/// Runs a recorded event stream through a decoder: for the wire format tests.
#[cfg(test)]
pub(crate) fn events(stream: &str) -> Vec<Value> {
    crate::sse::SseParser::default()
        .push(stream.as_bytes())
        .into_iter()
        .filter(|event| event.data != "[DONE]")
        .map(|event| serde_json::from_str(&event.data).expect("the test stream holds valid JSON"))
        .collect()
}

/// A sink that keeps every delta it is given.
#[derive(Default)]
pub struct Collected(pub Mutex<Vec<Delta>>);

impl Collected {
    pub fn push(&self, delta: Delta) {
        self.0.lock().unwrap().push(delta);
    }

    pub fn text(&self) -> String {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|delta| match delta {
                Delta::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }
}
