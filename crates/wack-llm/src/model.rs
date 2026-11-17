//! The provider-neutral conversation model.
//!
//! A [`Block::Opaque`] carries a provider's own item that we do not model (a signed reasoning item,
//! a server tool's result). It is replayed untouched to the format that produced it and dropped
//! for any other, which is what lets signatures round-trip without per-provider patches.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WireFormat {
    /// Anthropic's Messages API.
    AnthropicMessages,
    /// OpenAI's Responses API.
    OpenaiResponses,
    /// Chat Completions, as spoken by OpenAI and most compatible servers.
    OpenaiChat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, content: vec![Block::Text { text: text.into() }] }
    }

    /// The message's plain text, blocks joined in order.
    pub fn text(&self) -> String {
        let mut text = String::new();
        for block in &self.content {
            if let Block::Text { text: part } = block {
                text.push_str(part);
            }
        }
        text
    }

    pub fn tool_uses(&self) -> impl Iterator<Item = (&str, &str, &Value)> {
        self.content.iter().filter_map(|block| match block {
            Block::ToolUse { id, name, input } => Some((id.as_str(), name.as_str(), input)),
            _ => None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    /// The model's reasoning as shown to the user. `signature` is the provider's proof that lets
    /// the block be sent back; without one it is display-only and never replayed.
    Thinking {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// `input` is an object. When the model's JSON could not be parsed it is the raw string
    /// instead, so the caller can answer with an error rather than run the tool.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// `images` are a tool's pictures (a screenshot) beside its text. Older session lines have
    /// none, and a result without any is written exactly as before, so the field is optional both
    /// ways. Every format can carry them; whether a model is sent them is the agent's call.
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<Image>,
    },
    Image {
        media_type: String,
        /// Base64.
        data: String,
    },
    Opaque {
        format: WireFormat,
        value: Value,
    },
}

/// A picture a tool returned, as the providers take it inline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    /// `image/png`, `image/jpeg`, `image/gif` or `image/webp`: what every format accepts.
    pub media_type: String,
    /// Base64.
    pub data: String,
}

impl Image {
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.data)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "minimal" => Self::Minimal,
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" => Self::Xhigh,
            "max" => Self::Max,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemPart {
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Declared but not loaded into the model's context until it searches for it. Only sent this
    /// way to a provider with native tool search.
    pub deferred: bool,
}

/// Order matters for prompt caching: providers render tools, then system, then messages, and a
/// change anywhere invalidates everything after it. Keep the stable parts first.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub model: String,
    pub system: Vec<SystemPart>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    /// `None` leaves the provider's default in place.
    pub max_output_tokens: Option<u32>,
    /// `None` sends no reasoning settings at all, for models without reasoning.
    pub effort: Option<Effort>,
}

/// What arrives while a response streams. The finished blocks come back in the [`Response`].
#[derive(Clone, Debug, PartialEq)]
pub enum Delta {
    Text(String),
    Thinking(String),
    /// A tool call has begun; its input follows as [`Delta::ToolInput`].
    ToolStart { id: String, name: String },
    ToolInput { id: String, json: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Refusal,
    Other(String),
}

/// Token counts as the usage ledger wants them: the four categories are additive, `input` excludes
/// both cache categories, and `output` includes reasoning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Usage {
    /// Everything the model read for this request: the size of the context it was given.
    pub fn context_tokens(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub message: Message,
    pub stop: StopReason,
    /// `None` when the provider reported no usable counts. Never an estimate.
    pub usage: Option<Usage>,
}
