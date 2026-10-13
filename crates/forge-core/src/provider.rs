//! Provider wire formats. One trait, three implementations:
//! OpenAI-compatible (DeepSeek, OpenRouter, Gemini's OpenAI endpoint, OpenAI, xAI, Groq, Ollama, LM Studio),
//! Anthropic Messages, and a deterministic Mock for tests and dry runs.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Value};

use crate::config::{ProviderDef, Wire};
use crate::types::{ChatReply, ChatRequest, LlmError};

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    async fn complete(&self, req: &ChatRequest) -> Result<ChatReply, LlmError>;
}

/// Build a provider from its definition. Fails with `MissingKey` when the named env var is unset.
pub fn build(name: &str, def: &ProviderDef, client: reqwest::Client) -> Result<Arc<dyn Provider>, LlmError> {
    let key = match (&def.key, def.no_key) {
        (_, true) | (None, _) => None,
        (Some(var), false) => match std::env::var(var) {
            Ok(k) if !k.trim().is_empty() => Some(k.trim().to_string()),
            _ => return Err(LlmError::MissingKey(var.clone())),
        },
    };
    let headers = extra_headers(&def.headers);
    Ok(match def.wire() {
        Wire::Mock => Arc::new(MockProvider::echo(name)),
        Wire::Openai => Arc::new(OpenAiCompat { name: name.into(), url: def.url(), key, headers, client }),
        Wire::Anthropic => Arc::new(Anthropic { name: name.into(), url: def.url(), key, headers, client }),
    })
}

fn extra_headers(map: &BTreeMap<String, String>) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in map {
        if let (Ok(k), Ok(v)) = (HeaderName::try_from(k.as_str()), HeaderValue::from_str(v)) {
            h.insert(k, v);
        }
    }
    h
}

fn map_send_error(e: reqwest::Error) -> LlmError {
    if e.is_timeout() {
        LlmError::Timeout
    } else {
        LlmError::Connect(e.without_url().to_string())
    }
}

async fn map_status(resp: reqwest::Response) -> Result<Value, LlmError> {
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(Duration::from_secs_f64);
    let body = resp.text().await.map_err(map_send_error)?;
    let short: String = body.chars().take(500).collect();
    match status {
        200..=299 => serde_json::from_str(&body).map_err(|e| LlmError::InvalidReply(format!("{e}: {short}"))),
        429 => Err(LlmError::RateLimited { retry_after, body: short }),
        401 | 403 => Err(LlmError::Auth { status, body: short }),
        408 | 500..=599 => Err(LlmError::Server { status, body: short }),
        _ => Err(LlmError::BadRequest { status, body: short }),
    }
}

fn u32_at(v: &Value, path: &[&str]) -> u32 {
    let mut cur = v;
    for p in path {
        cur = &cur[*p];
    }
    cur.as_u64().unwrap_or(0) as u32
}

pub struct OpenAiCompat {
    name: String,
    url: String,
    key: Option<String>,
    headers: HeaderMap,
    client: reqwest::Client,
}

impl OpenAiCompat {
    pub fn body(req: &ChatRequest) -> Value {
        let mut messages: Vec<Value> = req.messages.iter().map(|m| json!({"role": m.role, "content": m.content})).collect();
        if req.json_mode && !req.messages.iter().any(|m| m.content.to_lowercase().contains("json")) {
            messages.insert(0, json!({"role": "system", "content": "Reply with a single JSON object."}));
        }
        let mut body = json!({
            "model": req.model,
            "messages": messages,
            "temperature": req.temperature,
            "max_tokens": req.max_tokens,
        });
        if req.json_mode {
            body["response_format"] = json!({"type": "json_object"});
        }
        body
    }

    pub fn parse(v: &Value) -> Result<ChatReply, LlmError> {
        let choice = &v["choices"][0];
        if choice.is_null() {
            return Err(LlmError::InvalidReply(format!("no choices in reply: {}", truncate(&v.to_string(), 300))));
        }
        let finish = choice["finish_reason"].as_str().map(str::to_string);
        let completion_tokens = u32_at(v, &["usage", "completion_tokens"]);
        if finish.as_deref() == Some("length") {
            return Err(LlmError::Truncated { completion_tokens });
        }
        let text = choice["message"]["content"].as_str().unwrap_or_default().to_string();
        if text.trim().is_empty() {
            return Err(LlmError::InvalidReply("empty content".into()));
        }
        Ok(ChatReply {
            text,
            prompt_tokens: u32_at(v, &["usage", "prompt_tokens"]),
            completion_tokens,
            finish_reason: finish,
            model: v["model"].as_str().unwrap_or_default().to_string(),
        })
    }
}

#[async_trait]
impl Provider for OpenAiCompat {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, req: &ChatRequest) -> Result<ChatReply, LlmError> {
        let mut rb = self.client.post(&self.url).headers(self.headers.clone()).json(&Self::body(req));
        if let Some(k) = &self.key {
            rb = rb.bearer_auth(k);
        }
        let v = map_status(rb.send().await.map_err(map_send_error)?).await?;
        let mut reply = Self::parse(&v)?;
        if reply.model.is_empty() {
            reply.model = req.model.clone();
        }
        Ok(reply)
    }
}

pub struct Anthropic {
    name: String,
    url: String,
    key: Option<String>,
    headers: HeaderMap,
    client: reqwest::Client,
}

impl Anthropic {
    /// Current Claude models reject non-default sampling parameters, so `temperature` is not sent.
    pub fn body(req: &ChatRequest) -> Value {
        let system: Vec<&str> = req.messages.iter().filter(|m| m.role == "system").map(|m| m.content.as_str()).collect();
        let messages: Vec<Value> = req
            .messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| json!({"role": if m.role == "assistant" { "assistant" } else { "user" }, "content": m.content}))
            .collect();
        let mut body = json!({ "model": req.model, "max_tokens": req.max_tokens, "messages": messages });
        let mut sys = system.join("\n\n");
        if req.json_mode {
            if !sys.is_empty() {
                sys.push_str("\n\n");
            }
            sys.push_str("Reply with a single JSON value and nothing else.");
        }
        if !sys.is_empty() {
            body["system"] = json!(sys);
        }
        body
    }

    pub fn parse(v: &Value) -> Result<ChatReply, LlmError> {
        let stop = v["stop_reason"].as_str().map(str::to_string);
        let completion_tokens = u32_at(v, &["usage", "output_tokens"]);
        match stop.as_deref() {
            Some("max_tokens") => return Err(LlmError::Truncated { completion_tokens }),
            Some("refusal") => {
                return Err(LlmError::InvalidReply(format!("model declined (refusal): {}", v["stop_details"])));
            }
            _ => {}
        }
        // Thinking blocks may precede the answer: join every text block.
        let text: String = v["content"]
            .as_array()
            .map(|blocks| blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join(""))
            .unwrap_or_default();
        if text.trim().is_empty() {
            return Err(LlmError::InvalidReply("no text content".into()));
        }
        Ok(ChatReply {
            text,
            prompt_tokens: u32_at(v, &["usage", "input_tokens"]),
            completion_tokens,
            finish_reason: stop,
            model: v["model"].as_str().unwrap_or_default().to_string(),
        })
    }
}

#[async_trait]
impl Provider for Anthropic {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, req: &ChatRequest) -> Result<ChatReply, LlmError> {
        let mut rb =
            self.client.post(&self.url).headers(self.headers.clone()).header("anthropic-version", "2023-06-01").json(&Self::body(req));
        if let Some(k) = &self.key {
            rb = rb.header("x-api-key", k);
        }
        let v = map_status(rb.send().await.map_err(map_send_error)?).await?;
        let mut reply = Self::parse(&v)?;
        if reply.model.is_empty() {
            reply.model = req.model.clone();
        }
        Ok(reply)
    }
}

/// No network. Replays scripted outcomes, then echoes deterministically.
pub struct MockProvider {
    name: String,
    script: Mutex<VecDeque<Result<ChatReply, LlmError>>>,
    pub calls: std::sync::atomic::AtomicUsize,
}

impl MockProvider {
    pub fn echo(name: &str) -> Self {
        Self::scripted(name, vec![])
    }

    pub fn scripted(name: &str, steps: Vec<Result<ChatReply, LlmError>>) -> Self {
        Self { name: name.into(), script: Mutex::new(steps.into()), calls: Default::default() }
    }

    pub fn ok(text: &str) -> Result<ChatReply, LlmError> {
        Ok(ChatReply {
            text: text.into(),
            prompt_tokens: 10,
            completion_tokens: 5,
            finish_reason: Some("stop".into()),
            model: "mock-1".into(),
        })
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, req: &ChatRequest) -> Result<ChatReply, LlmError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(step) = self.script.lock().unwrap().pop_front() {
            return step;
        }
        let user = req.messages.iter().rev().find(|m| m.role == "user").map(|m| m.content.as_str()).unwrap_or("");
        let chars = req.messages.iter().map(|m| m.content.len()).sum::<usize>();
        let text = if req.json_mode {
            json!({"mock": true, "model": req.model, "input_chars": chars}).to_string()
        } else {
            format!("[mock:{}] {} chars in. First line: {}", req.model, chars, user.lines().next().unwrap_or("").trim())
        };
        Ok(ChatReply {
            text,
            prompt_tokens: crate::types::estimate_tokens(chars),
            completion_tokens: 12,
            finish_reason: Some("stop".into()),
            model: req.model.clone(),
        })
    }
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;

    fn req(json_mode: bool) -> ChatRequest {
        ChatRequest {
            model: "m".into(),
            messages: vec![Message::system("be brief"), Message::user("hello")],
            temperature: 0.3,
            max_tokens: 100,
            json_mode,
        }
    }

    #[test]
    fn openai_json_mode_adds_instruction_and_format() {
        let b = OpenAiCompat::body(&req(true));
        assert_eq!(b["response_format"]["type"], "json_object");
        assert_eq!(b["messages"][0]["content"], "Reply with a single JSON object.");
    }

    #[test]
    fn openai_length_is_truncation_error() {
        let v = json!({"choices":[{"message":{"content":"half"},"finish_reason":"length"}],"usage":{"completion_tokens":100}});
        assert!(matches!(OpenAiCompat::parse(&v), Err(LlmError::Truncated { completion_tokens: 100 })));
    }

    #[test]
    fn anthropic_body_moves_system_and_omits_temperature() {
        let b = Anthropic::body(&req(false));
        assert_eq!(b["system"], "be brief");
        assert_eq!(b["messages"].as_array().unwrap().len(), 1);
        assert!(b.get("temperature").is_none());
    }

    #[test]
    fn anthropic_parse_joins_text_blocks_and_flags_max_tokens() {
        let v = json!({"content":[{"type":"thinking","thinking":""},{"type":"text","text":"a"},{"type":"text","text":"b"}],
                       "stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2},"model":"x"});
        assert_eq!(Anthropic::parse(&v).unwrap().text, "ab");
        let cut = json!({"content":[{"type":"text","text":"a"}],"stop_reason":"max_tokens","usage":{"output_tokens":9}});
        assert!(matches!(Anthropic::parse(&cut), Err(LlmError::Truncated { .. })));
    }
}
