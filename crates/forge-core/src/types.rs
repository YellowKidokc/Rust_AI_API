//! Wire-neutral request/reply types shared by every provider and front end.

use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into() }
    }
}

/// One call to one model. The router fills `model`; callers fill the rest.
#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub max_tokens: u32,
    pub json_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatReply {
    pub text: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub finish_reason: Option<String>,
    /// Model name as reported by the provider (may differ from the requested one).
    pub model: String,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum LlmError {
    #[error("rate limited (429): {body}")]
    RateLimited { retry_after: Option<Duration>, body: String },
    #[error("server error {status}: {body}")]
    Server { status: u16, body: String },
    #[error("auth refused {status}: {body}")]
    Auth { status: u16, body: String },
    #[error("bad request {status}: {body}")]
    BadRequest { status: u16, body: String },
    #[error("timed out")]
    Timeout,
    #[error("connection failed: {0}")]
    Connect(String),
    /// A reply cut off at the token cap is an error, never a result.
    #[error("reply truncated at the output token cap ({completion_tokens} tokens)")]
    Truncated { completion_tokens: u32 },
    #[error("missing API key: set environment variable {0}")]
    MissingKey(String),
    #[error("invalid reply: {0}")]
    InvalidReply(String),
}

impl LlmError {
    /// Worth retrying against the same provider/model after a back-off.
    pub fn retryable(&self) -> bool {
        matches!(self, LlmError::RateLimited { .. } | LlmError::Server { .. } | LlmError::Timeout | LlmError::Connect(_))
    }

    pub fn kind(&self) -> &'static str {
        match self {
            LlmError::RateLimited { .. } => "rate_limited",
            LlmError::Server { .. } => "server",
            LlmError::Auth { .. } => "auth",
            LlmError::BadRequest { .. } => "bad_request",
            LlmError::Timeout => "timeout",
            LlmError::Connect(_) => "connect",
            LlmError::Truncated { .. } => "truncated",
            LlmError::MissingKey(_) => "missing_key",
            LlmError::InvalidReply(_) => "invalid_reply",
        }
    }
}

/// Rough token estimate used for context-window checks and cost previews.
pub fn estimate_tokens(chars: usize) -> u32 {
    chars.div_ceil(4) as u32
}
