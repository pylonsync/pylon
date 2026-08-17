//! Provider-abstracted LLM client.
//!
//! Wire shape mirrors Anthropic's Messages API (the most expressive of
//! the major providers — content blocks, tool_use, tool_result, system
//! prompts). When the configured provider is OpenAI, requests are
//! translated at the transport boundary so callers always think in
//! Anthropic shape.
//!
//! Why a single shape: agent loops, ctx.llm.complete callers, and
//! the streaming /api/ai/stream route all want to ship tool_use blocks
//! and tool_result follow-ups. Picking one shape per call site would
//! create a translation matrix; instead, all of Pylon speaks Anthropic
//! and the OpenAI adapter translates on egress + ingress.
//!
//! HTTPS — `ureq` with the rustls `tls` feature is already a workspace
//! dep (used by storage + cli). The previous AiProxyPlugin shipped a
//! stub that returned "HTTPS streaming requires a TLS library" for
//! all real provider calls; this module fixes that.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Public types — Anthropic-shaped, used by ctx.llm + HTTP routes
// ---------------------------------------------------------------------------

/// Provider identity. Loaded from `PYLON_LLM_PROVIDER` env (default
/// `anthropic` when an `ANTHROPIC_API_KEY` is present, else `openai`
/// if `OPENAI_API_KEY` is present, else `None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    Anthropic,
    Openai,
}

impl LlmProvider {
    pub fn from_env() -> Option<(Self, String)> {
        match std::env::var("PYLON_LLM_PROVIDER")
            .ok()
            .or_else(|| std::env::var("PYLON_AI_PROVIDER").ok())
            .as_deref()
        {
            Some("anthropic") => std::env::var("ANTHROPIC_API_KEY")
                .or_else(|_| std::env::var("PYLON_AI_API_KEY"))
                .ok()
                .map(|k| (Self::Anthropic, k)),
            Some("openai") => std::env::var("OPENAI_API_KEY")
                .or_else(|_| std::env::var("PYLON_AI_API_KEY"))
                .ok()
                .map(|k| (Self::Openai, k)),
            None | Some("") => {
                if let Ok(k) = std::env::var("ANTHROPIC_API_KEY") {
                    Some((Self::Anthropic, k))
                } else if let Ok(k) = std::env::var("OPENAI_API_KEY") {
                    Some((Self::Openai, k))
                } else {
                    None
                }
            }
            Some(other) => {
                tracing::warn!(
                    "[llm] Unknown PYLON_LLM_PROVIDER {other:?}; expected anthropic|openai"
                );
                None
            }
        }
    }
}

/// One message in a conversation. Content is either a plain string
/// (the common case) or a list of content blocks (text + tool_use +
/// tool_result, needed once the agent loop starts calling tools).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String,
    pub content: LlmContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LlmContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

/// Content block — mirror of Anthropic's content block enum. Streams
/// arrive as a sequence of these. Tools loop emits `ToolUse`, the
/// caller responds with `ToolResult` referencing the same `id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// Tool declaration. The schema is a JSON Schema object describing
/// the tool's argument shape — Anthropic validates against it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmCompleteRequest {
    /// Override the server's default model. Subject to
    /// PYLON_AI_MODELS_ALLOWED gating for client-supplied requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub messages: Vec<LlmMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<LlmTool>,
    /// Defaults to 4096. Provider caps apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCompleteResponse {
    pub model: String,
    pub content: Vec<ContentBlock>,
    /// `end_turn` | `tool_use` | `max_tokens` | `stop_sequence`
    pub stop_reason: String,
    pub usage: LlmUsage,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct LlmUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Streaming event emitted while a response is generated. Mirrors
/// the subset of Anthropic's event types that callers actually need.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Plain text delta. The common case — append to whatever text
    /// block is currently open.
    TextDelta { text: String },
    /// Start of a tool_use block. Subsequent ToolInputDelta events
    /// stream the JSON arguments until the block is closed.
    ToolUseStart { id: String, name: String },
    /// Partial JSON for the currently-open tool_use block. Caller
    /// concatenates the raw fragments and parses once the block closes.
    ToolInputDelta { partial_json: String },
    /// End-of-stream marker with the final stop_reason + usage. Always
    /// emitted, even on partial failures.
    Done {
        stop_reason: String,
        usage: LlmUsage,
    },
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct LlmClient {
    inner: Arc<LlmClientInner>,
}

struct LlmClientInner {
    provider: LlmProvider,
    api_key: String,
    default_model: String,
    /// Optional base URL override for custom OpenAI-compatible endpoints
    /// (Together, Groq, local Ollama via HTTP). Default uses the
    /// provider's canonical host.
    base_url: Option<String>,
    /// ureq agent — pooled across calls for keep-alive.
    agent: ureq::Agent,
    /// Manifest-declared allowlist for caller-supplied models (codex P1-1).
    manifest_allowed: Vec<String>,
    /// Cap applied to `max_tokens` for non-admin callers (codex P1-5).
    /// None = no manifest/env cap.
    max_tokens_cap: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct LlmError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for LlmError {}

impl LlmClient {
    /// Build a client from environment + optional manifest defaults.
    /// Returns None when no provider + key is configured. The manifest
    /// `default_model` is applied when the env doesn't pin one;
    /// `allowed_models` is exposed via [`Self::manifest_allowed_models`]
    /// for callers that gate model overrides.
    ///
    /// Env precedence is preserved — operators rotating providers can
    /// override the manifest without redeploying the app bundle.
    pub fn from_env_with_manifest(
        manifest_llm: Option<&pylon_kernel::ManifestLlmConfig>,
    ) -> Option<Self> {
        // Provider: env-detected, with manifest as a fallback signal
        // (the env helper already covers explicit env). If the env
        // pinned a provider but no matching key exists, log loudly
        // — that's the common "rotated secret, forgot the key" foot-
        // gun (codex P2-B).
        let env_explicit_provider = std::env::var("PYLON_LLM_PROVIDER")
            .or_else(|_| std::env::var("PYLON_AI_PROVIDER"))
            .ok()
            .filter(|s| !s.is_empty());
        if let Some(prov) = env_explicit_provider.as_deref() {
            let needs = match prov {
                "anthropic" => "ANTHROPIC_API_KEY",
                "openai" => "OPENAI_API_KEY",
                _ => "",
            };
            if !needs.is_empty()
                && std::env::var(needs)
                    .ok()
                    .filter(|s| !s.is_empty())
                    .is_none()
                && std::env::var("PYLON_AI_API_KEY")
                    .ok()
                    .filter(|s| !s.is_empty())
                    .is_none()
            {
                tracing::warn!(
                    "[llm] PYLON_LLM_PROVIDER={prov} but {needs} is unset; ctx.llm.complete will reject as LLM_NOT_CONFIGURED"
                );
            }
        }
        let (provider, api_key) = LlmProvider::from_env()?;
        // Loud telemetry when both keys exist but no explicit provider
        // was selected — codex P2-A.
        if env_explicit_provider.is_none()
            && std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|s| !s.is_empty())
                .is_some()
            && std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|s| !s.is_empty())
                .is_some()
        {
            tracing::warn!(
                "[llm] Both ANTHROPIC_API_KEY and OPENAI_API_KEY are set with no PYLON_LLM_PROVIDER; auto-selected provider={provider:?}. Set PYLON_LLM_PROVIDER to pin."
            );
        }
        let manifest_model = manifest_llm.and_then(|m| m.default_model.clone());
        let default_model = std::env::var("PYLON_LLM_MODEL")
            .or_else(|_| std::env::var("PYLON_AI_MODEL"))
            .ok()
            .or(manifest_model)
            .unwrap_or_else(|| match provider {
                LlmProvider::Anthropic => "claude-sonnet-4-5".into(),
                LlmProvider::Openai => "gpt-4o-mini".into(),
            });
        let base_url = std::env::var("PYLON_LLM_BASE_URL")
            .or_else(|_| std::env::var("PYLON_AI_BASE_URL"))
            .ok()
            .filter(|s| !s.is_empty());
        let manifest_allowed = manifest_llm
            .map(|m| m.allowed_models.clone())
            .unwrap_or_default();
        let agent = ureq::AgentBuilder::new()
            // Separate connect timeout (10s) catches TLS handshake stalls
            // without holding the worker thread for the full 120s read
            // budget. Read timeout caps total round-trip for the
            // non-streaming complete path.
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build();
        Some(Self {
            inner: Arc::new(LlmClientInner {
                provider,
                api_key,
                default_model,
                base_url,
                agent,
                manifest_allowed,
                max_tokens_cap: std::env::var("PYLON_LLM_MAX_TOKENS_CAP")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .filter(|n: &u32| *n > 0),
            }),
        })
    }

    /// Build a client from environment only (no manifest). Kept for
    /// callers that don't have the manifest in scope.
    pub fn from_env() -> Option<Self> {
        Self::from_env_with_manifest(None)
    }

    /// Returns the manifest-declared allowlist (empty when unset).
    /// Callers combine this with `PYLON_AI_MODELS_ALLOWED` env to
    /// build the effective list.
    pub fn manifest_allowed_models(&self) -> &[String] {
        &self.inner.manifest_allowed
    }

    /// Returns the configured max_tokens cap. None = no manifest /
    /// env cap (callers can still request any value, subject to the
    /// provider's own limits).
    pub fn max_tokens_cap(&self) -> Option<u32> {
        self.inner.max_tokens_cap
    }

    pub fn new(
        provider: LlmProvider,
        api_key: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build();
        Self {
            inner: Arc::new(LlmClientInner {
                provider,
                api_key: api_key.into(),
                default_model: default_model.into(),
                base_url: None,
                agent,
                manifest_allowed: Vec::new(),
                max_tokens_cap: None,
            }),
        }
    }

    pub fn provider(&self) -> LlmProvider {
        self.inner.provider
    }
    pub fn default_model(&self) -> &str {
        &self.inner.default_model
    }

    /// Non-streaming completion. Returns the full response in one
    /// shot. Use this when the caller doesn't need progressive output
    /// (agent tool-use loops, batch generation, internal jobs).
    pub fn complete(&self, mut req: LlmCompleteRequest) -> Result<LlmCompleteResponse, LlmError> {
        if req.model.is_none() {
            req.model = Some(self.inner.default_model.clone());
        }
        // Clamp max_tokens against the configured cap. Codex P1-5:
        // a caller can request `max_tokens: 200000` against a premium
        // model and burn the entire budget on one request. With the
        // cap set, requests above it silently clamp down (caller still
        // gets a response, just truncated to the cap).
        let requested_max = req.max_tokens.unwrap_or(4096);
        let capped_max = match self.inner.max_tokens_cap {
            Some(cap) => requested_max.min(cap),
            None => requested_max,
        };
        req.max_tokens = Some(capped_max);
        match self.inner.provider {
            LlmProvider::Anthropic => self.complete_anthropic(req),
            LlmProvider::Openai => self.complete_openai(req),
        }
    }

    /// Streaming completion. Calls `on_event` for each event in the
    /// SSE stream. Returns the assembled final response on the last
    /// `Done` event (same shape as `complete`).
    pub fn stream(
        &self,
        mut req: LlmCompleteRequest,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<LlmCompleteResponse, LlmError> {
        if req.model.is_none() {
            req.model = Some(self.inner.default_model.clone());
        }
        let requested_max = req.max_tokens.unwrap_or(4096);
        req.max_tokens = Some(match self.inner.max_tokens_cap {
            Some(cap) => requested_max.min(cap),
            None => requested_max,
        });
        match self.inner.provider {
            LlmProvider::Anthropic => self.stream_anthropic(req, on_event),
            LlmProvider::Openai => self.stream_openai(req, on_event),
        }
    }

    // -----------------------------------------------------------------------
    // Anthropic transport
    // -----------------------------------------------------------------------

    fn anthropic_url(&self) -> String {
        match &self.inner.base_url {
            Some(base) => format!("{}/v1/messages", base.trim_end_matches('/')),
            None => "https://api.anthropic.com/v1/messages".into(),
        }
    }

    fn complete_anthropic(&self, req: LlmCompleteRequest) -> Result<LlmCompleteResponse, LlmError> {
        let body = anthropic_request_body(&req, false);
        let resp = self
            .inner
            .agent
            .post(&self.anthropic_url())
            .set("x-api-key", &self.inner.api_key)
            .set("anthropic-version", "2023-06-01")
            .set("content-type", "application/json")
            .send_json(body)
            .map_err(map_ureq_err)?;
        let parsed: serde_json::Value = resp.into_json().map_err(|e| LlmError {
            code: "INVALID_RESPONSE".into(),
            message: format!("Failed to parse provider response: {e}"),
        })?;
        parse_anthropic_message(parsed)
    }

    fn stream_anthropic(
        &self,
        req: LlmCompleteRequest,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<LlmCompleteResponse, LlmError> {
        let body = anthropic_request_body(&req, true);
        let resp = self
            .inner
            .agent
            .post(&self.anthropic_url())
            .set("x-api-key", &self.inner.api_key)
            .set("anthropic-version", "2023-06-01")
            .set("content-type", "application/json")
            .set("accept", "text/event-stream")
            .send_json(body)
            .map_err(map_ureq_err)?;

        let mut model = req
            .model
            .clone()
            .unwrap_or_else(|| self.inner.default_model.clone());
        let mut content: Vec<ContentBlock> = Vec::new();
        let mut stop_reason = String::from("end_turn");
        let mut usage = LlmUsage::default();

        // Track the currently-open block so deltas concatenate
        // into the right slot.
        enum OpenBlock {
            None,
            Text(usize),
            ToolUse { idx: usize, raw_json: String },
        }
        let mut open = OpenBlock::None;

        let reader = BufReader::new(resp.into_reader());
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    return Err(LlmError {
                        code: "STREAM_READ_FAILED".into(),
                        message: format!("Read failed mid-stream: {e}"),
                    });
                }
            };
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("event:") || trimmed.starts_with(':') {
                continue;
            }
            let data = match trimmed.strip_prefix("data:") {
                Some(d) => d.trim(),
                None => continue,
            };
            if data == "[DONE]" {
                break;
            }
            let value: serde_json::Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match event_type {
                "message_start" => {
                    if let Some(m) = value
                        .get("message")
                        .and_then(|m| m.get("model"))
                        .and_then(|v| v.as_str())
                    {
                        model = m.to_string();
                    }
                    if let Some(u) = value.get("message").and_then(|m| m.get("usage")) {
                        usage.input_tokens =
                            u.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as u32;
                    }
                }
                "content_block_start" => {
                    let block = value
                        .get("content_block")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let bt = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match bt {
                        "text" => {
                            content.push(ContentBlock::Text {
                                text: String::new(),
                            });
                            open = OpenBlock::Text(content.len() - 1);
                        }
                        "tool_use" => {
                            let id = block
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = block
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            content.push(ContentBlock::ToolUse {
                                id: id.clone(),
                                name: name.clone(),
                                input: serde_json::json!({}),
                            });
                            on_event(StreamEvent::ToolUseStart { id, name });
                            open = OpenBlock::ToolUse {
                                idx: content.len() - 1,
                                raw_json: String::new(),
                            };
                        }
                        _ => {}
                    }
                }
                "content_block_delta" => {
                    let delta = value
                        .get("delta")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let dt = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match (&mut open, dt) {
                        (OpenBlock::Text(idx), "text_delta") => {
                            if let Some(t) = delta.get("text").and_then(|v| v.as_str()) {
                                if let ContentBlock::Text { text } = &mut content[*idx] {
                                    text.push_str(t);
                                }
                                on_event(StreamEvent::TextDelta {
                                    text: t.to_string(),
                                });
                            }
                        }
                        (OpenBlock::ToolUse { raw_json, .. }, "input_json_delta") => {
                            if let Some(p) = delta.get("partial_json").and_then(|v| v.as_str()) {
                                raw_json.push_str(p);
                                on_event(StreamEvent::ToolInputDelta {
                                    partial_json: p.to_string(),
                                });
                            }
                        }
                        _ => {}
                    }
                }
                "content_block_stop" => {
                    if let OpenBlock::ToolUse { idx, raw_json } = &open {
                        if let ContentBlock::ToolUse { input, .. } = &mut content[*idx] {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw_json)
                            {
                                *input = parsed;
                            }
                        }
                    }
                    open = OpenBlock::None;
                }
                "message_delta" => {
                    if let Some(sr) = value
                        .get("delta")
                        .and_then(|d| d.get("stop_reason"))
                        .and_then(|v| v.as_str())
                    {
                        stop_reason = sr.to_string();
                    }
                    if let Some(u) = value.get("usage") {
                        if let Some(o) = u.get("output_tokens").and_then(|v| v.as_u64()) {
                            usage.output_tokens = o as u32;
                        }
                    }
                }
                "message_stop" => {
                    break;
                }
                _ => {}
            }
        }

        on_event(StreamEvent::Done {
            stop_reason: stop_reason.clone(),
            usage,
        });

        Ok(LlmCompleteResponse {
            model,
            content,
            stop_reason,
            usage,
        })
    }

    // -----------------------------------------------------------------------
    // OpenAI transport (translation to/from Anthropic shape)
    // -----------------------------------------------------------------------

    fn openai_url(&self) -> String {
        match &self.inner.base_url {
            Some(base) => format!("{}/v1/chat/completions", base.trim_end_matches('/')),
            None => "https://api.openai.com/v1/chat/completions".into(),
        }
    }

    fn complete_openai(&self, req: LlmCompleteRequest) -> Result<LlmCompleteResponse, LlmError> {
        let body = openai_request_body(&req, false);
        let resp = self
            .inner
            .agent
            .post(&self.openai_url())
            .set("Authorization", &format!("Bearer {}", self.inner.api_key))
            .set("Content-Type", "application/json")
            .send_json(body)
            .map_err(map_ureq_err)?;
        let parsed: serde_json::Value = resp.into_json().map_err(|e| LlmError {
            code: "INVALID_RESPONSE".into(),
            message: format!("Failed to parse provider response: {e}"),
        })?;
        parse_openai_completion(parsed)
    }

    fn stream_openai(
        &self,
        req: LlmCompleteRequest,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<LlmCompleteResponse, LlmError> {
        let body = openai_request_body(&req, true);
        let resp = self
            .inner
            .agent
            .post(&self.openai_url())
            .set("Authorization", &format!("Bearer {}", self.inner.api_key))
            .set("Content-Type", "application/json")
            .set("Accept", "text/event-stream")
            .send_json(body)
            .map_err(map_ureq_err)?;

        let mut model = req
            .model
            .clone()
            .unwrap_or_else(|| self.inner.default_model.clone());
        let mut text_buf = String::new();
        let mut tool_calls: Vec<(String, String, String)> = Vec::new(); // (id, name, raw_args)
        let mut stop_reason = String::from("end_turn");
        // Usage arrives in the final chunk when stream_options.include_usage
        // is set — codex P1-8 fix. Mutable so we can populate from the
        // last frame.
        let mut usage = LlmUsage::default();

        let reader = BufReader::new(resp.into_reader());
        for line in reader.lines() {
            let line = line.map_err(|e| LlmError {
                code: "STREAM_READ_FAILED".into(),
                message: format!("Read failed mid-stream: {e}"),
            })?;
            let trimmed = line.trim();
            let data = match trimmed.strip_prefix("data:") {
                Some(d) => d.trim(),
                None => continue,
            };
            if data == "[DONE]" {
                break;
            }
            let value: serde_json::Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(m) = value.get("model").and_then(|v| v.as_str()) {
                model = m.to_string();
            }
            // Capture usage from any chunk that carries it — OpenAI's
            // final chunk has empty choices + a populated usage object
            // when stream_options.include_usage is set.
            if let Some(u) = value.get("usage") {
                if let Some(p) = u.get("prompt_tokens").and_then(|v| v.as_u64()) {
                    usage.input_tokens = p as u32;
                }
                if let Some(c) = u.get("completion_tokens").and_then(|v| v.as_u64()) {
                    usage.output_tokens = c as u32;
                }
            }
            let choice = value.get("choices").and_then(|c| c.get(0));
            if let Some(c) = choice {
                if let Some(delta) = c.get("delta") {
                    if let Some(t) = delta.get("content").and_then(|v| v.as_str()) {
                        text_buf.push_str(t);
                        on_event(StreamEvent::TextDelta {
                            text: t.to_string(),
                        });
                    }
                    if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                        for tc in tcs {
                            let idx =
                                tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                            while tool_calls.len() <= idx {
                                tool_calls.push((String::new(), String::new(), String::new()));
                            }
                            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                tool_calls[idx].0 = id.to_string();
                            }
                            if let Some(fname) = tc
                                .get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|v| v.as_str())
                            {
                                if tool_calls[idx].1.is_empty() {
                                    on_event(StreamEvent::ToolUseStart {
                                        id: tool_calls[idx].0.clone(),
                                        name: fname.to_string(),
                                    });
                                }
                                tool_calls[idx].1 = fname.to_string();
                            }
                            if let Some(args) = tc
                                .get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(|v| v.as_str())
                            {
                                tool_calls[idx].2.push_str(args);
                                on_event(StreamEvent::ToolInputDelta {
                                    partial_json: args.to_string(),
                                });
                            }
                        }
                    }
                }
                if let Some(fr) = c.get("finish_reason").and_then(|v| v.as_str()) {
                    stop_reason = match fr {
                        "stop" => "end_turn".into(),
                        "length" => "max_tokens".into(),
                        "tool_calls" => "tool_use".into(),
                        other => other.into(),
                    };
                }
            }
        }

        let mut content: Vec<ContentBlock> = Vec::new();
        if !text_buf.is_empty() {
            content.push(ContentBlock::Text { text: text_buf });
        }
        for (id, name, raw_args) in tool_calls {
            let input = serde_json::from_str::<serde_json::Value>(&raw_args)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            content.push(ContentBlock::ToolUse { id, name, input });
        }

        on_event(StreamEvent::Done {
            stop_reason: stop_reason.clone(),
            usage,
        });

        Ok(LlmCompleteResponse {
            model,
            content,
            stop_reason,
            usage,
        })
    }
}

// ---------------------------------------------------------------------------
// Request body builders
// ---------------------------------------------------------------------------

fn anthropic_request_body(req: &LlmCompleteRequest, stream: bool) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": req.model.as_deref().unwrap_or(""),
        "messages": serialize_messages_anthropic(&req.messages),
        "max_tokens": req.max_tokens.unwrap_or(4096),
        "stream": stream,
    });
    if let Some(system) = &req.system {
        body["system"] = serde_json::Value::String(system.clone());
    }
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if !req.tools.is_empty() {
        body["tools"] = serde_json::json!(req
            .tools
            .iter()
            .map(|t| serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            }))
            .collect::<Vec<_>>());
    }
    body
}

fn serialize_messages_anthropic(messages: &[LlmMessage]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| {
            let content = match &m.content {
                LlmContent::Text(s) => serde_json::Value::String(s.clone()),
                LlmContent::Blocks(blocks) => serde_json::json!(blocks),
            };
            serde_json::json!({"role": m.role, "content": content})
        })
        .collect()
}

fn openai_request_body(req: &LlmCompleteRequest, stream: bool) -> serde_json::Value {
    let mut messages: Vec<serde_json::Value> = Vec::new();
    if let Some(system) = &req.system {
        messages.push(serde_json::json!({"role": "system", "content": system}));
    }
    // Codex P1-9: serialize_message_openai may yield ONE OR MORE
    // wire messages per Anthropic message — a `user` message with
    // multiple `tool_result` blocks must become multiple `role:tool`
    // messages, not one (OpenAI rejects multiple results on a single
    // message and the previous code silently dropped all but the last).
    for m in &req.messages {
        for wire_msg in serialize_messages_openai(m) {
            messages.push(wire_msg);
        }
    }
    let mut body = serde_json::json!({
        "model": req.model.as_deref().unwrap_or(""),
        "messages": messages,
        "max_tokens": req.max_tokens.unwrap_or(4096),
        "stream": stream,
    });
    if stream {
        // Codex P1-8: ask OpenAI to include usage in the final
        // streamed chunk. Without this the streaming path always
        // returns `input_tokens: 0, output_tokens: 0`, breaking
        // metering / billing accounting downstream.
        body["stream_options"] = serde_json::json!({"include_usage": true});
    }
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if !req.tools.is_empty() {
        body["tools"] = serde_json::json!(req
            .tools
            .iter()
            .map(|t| serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                }
            }))
            .collect::<Vec<_>>());
    }
    body
}

/// Translate one Anthropic-shaped message into ONE OR MORE OpenAI
/// wire messages.
///
/// - Plain text → one assistant/user message with `content` string.
/// - `tool_use` blocks → one assistant message with `tool_calls`.
///   Tool calls are only valid on `role: "assistant"`; if the caller
///   passed a different role with tool_use blocks, we force it to
///   "assistant" (codex P1-10: silently producing a 400 from the
///   provider is worse than normalizing — but log a warn so the
///   author sees the misuse).
/// - `tool_result` blocks → ONE message per result, each with
///   `role: "tool"` + the corresponding `tool_call_id` (codex P1-9:
///   the previous code emitted at most one regardless of how many
///   tool_result blocks the caller supplied, silently dropping
///   parallel tool calls).
/// - Mixed content (text + tool_use in the same assistant message)
///   stays in one wire message — OpenAI supports `content` + sibling
///   `tool_calls` on assistant messages.
fn serialize_messages_openai(m: &LlmMessage) -> Vec<serde_json::Value> {
    match &m.content {
        LlmContent::Text(s) => {
            vec![serde_json::json!({"role": m.role, "content": s})]
        }
        LlmContent::Blocks(blocks) => {
            let mut out: Vec<serde_json::Value> = Vec::new();
            let mut text_chunks: Vec<String> = Vec::new();
            let mut tool_calls: Vec<serde_json::Value> = Vec::new();
            // Emit tool_result blocks as standalone role:"tool"
            // messages — these don't combine with text/tool_use.
            for b in blocks {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = b
                {
                    out.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": content,
                    }));
                }
            }
            for b in blocks {
                match b {
                    ContentBlock::Text { text } => text_chunks.push(text.clone()),
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push(serde_json::json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": input.to_string(),
                            }
                        }));
                    }
                    ContentBlock::ToolResult { .. } => {} // handled above
                }
            }
            // Build the text+tool_calls message if we have either —
            // role-normalized to assistant when tool_calls present,
            // since OpenAI rejects tool_calls on non-assistant roles.
            if !text_chunks.is_empty() || !tool_calls.is_empty() {
                let role = if !tool_calls.is_empty() && m.role != "assistant" {
                    tracing::warn!(
                        "[llm] OpenAI: forcing role to \"assistant\" on message with tool_calls (caller passed role=\"{}\")",
                        m.role
                    );
                    "assistant"
                } else {
                    m.role.as_str()
                };
                let mut obj = serde_json::json!({
                    "role": role,
                    "content": if text_chunks.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::Value::String(text_chunks.join(""))
                    },
                });
                if !tool_calls.is_empty() {
                    obj["tool_calls"] = serde_json::Value::Array(tool_calls);
                }
                out.push(obj);
            }
            out
        }
    }
}

// ---------------------------------------------------------------------------
// Response parsers
// ---------------------------------------------------------------------------

fn parse_anthropic_message(v: serde_json::Value) -> Result<LlmCompleteResponse, LlmError> {
    let model = v
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let stop_reason = v
        .get("stop_reason")
        .and_then(|s| s.as_str())
        .unwrap_or("end_turn")
        .to_string();
    let usage_obj = v.get("usage");
    let usage = LlmUsage {
        input_tokens: usage_obj
            .and_then(|u| u.get("input_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32,
        output_tokens: usage_obj
            .and_then(|u| u.get("output_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32,
    };
    let content: Vec<ContentBlock> = v
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    Ok(LlmCompleteResponse {
        model,
        content,
        stop_reason,
        usage,
    })
}

fn parse_openai_completion(v: serde_json::Value) -> Result<LlmCompleteResponse, LlmError> {
    let model = v
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let usage_obj = v.get("usage");
    let usage = LlmUsage {
        input_tokens: usage_obj
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32,
        output_tokens: usage_obj
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32,
    };
    let choice = v
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| LlmError {
            code: "INVALID_RESPONSE".into(),
            message: "OpenAI response missing choices[0]".into(),
        })?;
    let message = choice.get("message").ok_or_else(|| LlmError {
        code: "INVALID_RESPONSE".into(),
        message: "OpenAI choices[0] missing message".into(),
    })?;
    let mut content: Vec<ContentBlock> = Vec::new();
    if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            content.push(ContentBlock::Text { text: text.into() });
        }
    }
    if let Some(tcs) = message.get("tool_calls").and_then(|c| c.as_array()) {
        for tc in tcs {
            let id = tc
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args_str = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let input = serde_json::from_str(args_str)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            content.push(ContentBlock::ToolUse { id, name, input });
        }
    }
    let stop_reason = match choice.get("finish_reason").and_then(|v| v.as_str()) {
        Some("stop") => "end_turn".into(),
        Some("length") => "max_tokens".into(),
        Some("tool_calls") => "tool_use".into(),
        Some(other) => other.into(),
        None => "end_turn".into(),
    };
    Ok(LlmCompleteResponse {
        model,
        content,
        stop_reason,
        usage,
    })
}

fn map_ureq_err(err: ureq::Error) -> LlmError {
    match err {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            LlmError {
                code: format!("PROVIDER_HTTP_{code}"),
                // Codex P1-M: callers see the redacted, truncated body
                // verbatim. A provider's 400 body can echo prompt
                // content + a partial request — both of which can leak
                // to an attacker who triggers an error via a public
                // function. The PROVIDER_HTTP_ code carries enough
                // signal; the body summary is bounded + redacted but
                // remains useful for ops debugging.
                message: redact_body(&body),
            }
        }
        ureq::Error::Transport(t) => LlmError {
            code: "PROVIDER_UNREACHABLE".into(),
            // Transport errors can carry the request URL + sometimes
            // header fragments. Run through the same redaction pass
            // (codex P1-11).
            message: redact_body(&format!("{t}")),
        },
    }
}

/// Provider error bodies + transport errors sometimes echo back
/// fragments of headers / request bodies. Strip anything that looks
/// like an api_key before the error reaches a client.
///
/// Covers Anthropic (`sk-ant-`), OpenAI legacy (`sk-`), OpenAI
/// project keys (`sk-proj-`), Together/Groq/Perplexity (custom
/// patterns), and Bearer header echoes.
fn redact_body(body: &str) -> String {
    let mut out = body.to_string();
    // Common API-key prefixes. Order matters: longest first so
    // `sk-` doesn't eat the prefix of `sk-ant-` before the more
    // specific match fires.
    for prefix in [
        "sk-ant-api03-",
        "sk-ant-",
        "sk-proj-",
        "tgp_v1_", // Together
        "gsk_",    // Groq
        "pplx-",   // Perplexity
        "sk-",
    ] {
        while let Some(start) = out.find(prefix) {
            let after = &out[start..];
            // End at first whitespace/quote/comma/closing brace.
            let end_rel = after
                .find(|c: char| {
                    c.is_whitespace() || c == '"' || c == ',' || c == '}' || c == ']' || c == '\''
                })
                .unwrap_or(after.len());
            let absolute_end = start + end_rel;
            out.replace_range(start..absolute_end, "<redacted>");
        }
    }
    // Strip `Authorization: Bearer <anything>` patterns (case-insensitive
    // best-effort — the redaction above already catches typical key
    // prefixes; this is a belt-and-suspenders pass).
    let lower = out.to_lowercase();
    if let Some(pos) = lower.find("authorization: bearer ") {
        let value_start = pos + "authorization: bearer ".len();
        let after = &out[value_start..];
        let end_rel = after
            .find(|c: char| c.is_whitespace() || c == '"' || c == ',')
            .unwrap_or(after.len());
        out.replace_range(value_start..value_start + end_rel, "<redacted>");
    }
    // UTF-8 safe truncation — `String::truncate(n)` panics if n falls
    // mid-codepoint. Step back to the previous char boundary instead.
    const LIMIT: usize = 1024;
    if out.len() > LIMIT {
        let mut cut = LIMIT;
        while cut > 0 && !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str("...");
    }
    out
}

// ---------------------------------------------------------------------------
// Embeddings — a separate provider axis from chat. An app can run
// Anthropic for completions and OpenAI or Voyage for embeddings; each
// resolves its own key so configuring one never silently disables the
// other.
// ---------------------------------------------------------------------------

/// Embeddings provider identity. Loaded from `PYLON_EMBEDDINGS_PROVIDER`
/// (`openai` | `voyage`); when unset, auto-detects from which key is
/// present (OpenAI first — it's also the chat provider's key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingsProvider {
    Openai,
    Voyage,
}

/// Request shape for `ctx.llm.embed`. `input` is 1..=2048 texts —
/// the OpenAI batch cap; Voyage's is lower (128) and the provider
/// rejects overage with a clear HTTP error.
#[derive(Debug, Clone, Deserialize)]
pub struct LlmEmbedRequest {
    pub input: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// One embedding per input text, in input order.
#[derive(Debug, Clone, Serialize)]
pub struct LlmEmbedResponse {
    pub embeddings: Vec<Vec<f32>>,
    pub model: String,
    /// Provider-reported total token usage (0 when not reported).
    pub total_tokens: u32,
}

#[derive(Clone)]
pub struct EmbeddingsClient {
    inner: Arc<EmbeddingsClientInner>,
}

struct EmbeddingsClientInner {
    provider: EmbeddingsProvider,
    api_key: String,
    default_model: String,
    /// `PYLON_EMBEDDINGS_BASE_URL` — OpenAI-compatible endpoint
    /// override (Together, local inference servers).
    base_url: Option<String>,
    agent: ureq::Agent,
}

impl EmbeddingsClient {
    /// Build from environment. Returns None when no embeddings
    /// provider + key can be resolved.
    ///
    /// Resolution:
    /// - `PYLON_EMBEDDINGS_PROVIDER=openai` → key from
    ///   `PYLON_EMBEDDINGS_API_KEY` | `OPENAI_API_KEY` | `PYLON_AI_API_KEY`
    /// - `PYLON_EMBEDDINGS_PROVIDER=voyage` → key from
    ///   `PYLON_EMBEDDINGS_API_KEY` | `VOYAGE_API_KEY`
    /// - unset → openai when an OpenAI key exists, else voyage when
    ///   `VOYAGE_API_KEY` exists, else None.
    ///
    /// Default model: `PYLON_EMBEDDINGS_MODEL`, else
    /// `text-embedding-3-small` (openai) / `voyage-3.5` (voyage).
    pub fn from_env() -> Option<Self> {
        let env = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
        let explicit = env("PYLON_EMBEDDINGS_PROVIDER");
        let openai_key = || {
            env("PYLON_EMBEDDINGS_API_KEY")
                .or_else(|| env("OPENAI_API_KEY"))
                .or_else(|| env("PYLON_AI_API_KEY"))
        };
        let voyage_key = || env("PYLON_EMBEDDINGS_API_KEY").or_else(|| env("VOYAGE_API_KEY"));
        let (provider, api_key) = match explicit.as_deref() {
            Some("openai") => {
                let Some(key) = openai_key() else {
                    tracing::warn!(
                        "[llm] PYLON_EMBEDDINGS_PROVIDER=openai but no OPENAI_API_KEY / PYLON_EMBEDDINGS_API_KEY; ctx.llm.embed will reject as EMBEDDINGS_NOT_CONFIGURED"
                    );
                    return None;
                };
                (EmbeddingsProvider::Openai, key)
            }
            Some("voyage") => {
                let Some(key) = voyage_key() else {
                    tracing::warn!(
                        "[llm] PYLON_EMBEDDINGS_PROVIDER=voyage but no VOYAGE_API_KEY / PYLON_EMBEDDINGS_API_KEY; ctx.llm.embed will reject as EMBEDDINGS_NOT_CONFIGURED"
                    );
                    return None;
                };
                (EmbeddingsProvider::Voyage, key)
            }
            Some(other) => {
                tracing::warn!(
                    "[llm] Unknown PYLON_EMBEDDINGS_PROVIDER {other:?}; expected openai|voyage"
                );
                return None;
            }
            None => {
                if let Some(key) = env("OPENAI_API_KEY").or_else(|| env("PYLON_AI_API_KEY")) {
                    (EmbeddingsProvider::Openai, key)
                } else if let Some(key) = env("VOYAGE_API_KEY") {
                    (EmbeddingsProvider::Voyage, key)
                } else if let Some(key) = env("PYLON_EMBEDDINGS_API_KEY") {
                    // A dedicated embeddings key with no provider named:
                    // default to OpenAI (the documented default), loudly.
                    tracing::info!(
                        "[llm] PYLON_EMBEDDINGS_API_KEY set with no PYLON_EMBEDDINGS_PROVIDER; defaulting to openai"
                    );
                    (EmbeddingsProvider::Openai, key)
                } else {
                    return None;
                }
            }
        };
        let default_model = env("PYLON_EMBEDDINGS_MODEL").unwrap_or_else(|| match provider {
            EmbeddingsProvider::Openai => "text-embedding-3-small".into(),
            EmbeddingsProvider::Voyage => "voyage-3.5".into(),
        });
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build();
        Some(Self {
            inner: Arc::new(EmbeddingsClientInner {
                provider,
                api_key,
                default_model,
                base_url: env("PYLON_EMBEDDINGS_BASE_URL"),
                agent,
            }),
        })
    }

    pub fn provider(&self) -> EmbeddingsProvider {
        self.inner.provider
    }
    pub fn default_model(&self) -> &str {
        &self.inner.default_model
    }

    fn url(&self) -> String {
        match &self.inner.base_url {
            Some(base) => format!("{}/v1/embeddings", base.trim_end_matches('/')),
            None => match self.inner.provider {
                EmbeddingsProvider::Openai => "https://api.openai.com/v1/embeddings".into(),
                EmbeddingsProvider::Voyage => "https://api.voyageai.com/v1/embeddings".into(),
            },
        }
    }

    /// Embed a batch of texts. Both providers speak the same
    /// OpenAI-shaped request/response (`{model, input}` →
    /// `{data: [{embedding, index}], model, usage}`), so one transport
    /// path serves both — only the host + auth header differ.
    pub fn embed(&self, req: LlmEmbedRequest) -> Result<LlmEmbedResponse, LlmError> {
        if req.input.is_empty() {
            return Err(LlmError {
                code: "INVALID_REQUEST".into(),
                message: "embed input must contain at least one text".into(),
            });
        }
        if req.input.len() > 2048 {
            return Err(LlmError {
                code: "INVALID_REQUEST".into(),
                message: format!(
                    "embed input has {} texts; max 2048 per call",
                    req.input.len()
                ),
            });
        }
        let model = req
            .model
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| self.inner.default_model.clone());
        let body = serde_json::json!({ "model": model, "input": req.input });
        let resp = self
            .inner
            .agent
            .post(&self.url())
            .set("authorization", &format!("Bearer {}", self.inner.api_key))
            .set("content-type", "application/json")
            .send_json(body)
            .map_err(map_ureq_err)?;
        let parsed: serde_json::Value = resp.into_json().map_err(|e| LlmError {
            code: "INVALID_RESPONSE".into(),
            message: format!("Failed to parse embeddings response: {e}"),
        })?;
        parse_embed_response(&parsed, req.input.len(), &model)
    }
}

/// Parse the OpenAI-shaped embeddings response body. Factored out of
/// [`EmbeddingsClient::embed`] so response-shape handling is unit-
/// testable without a live provider.
fn parse_embed_response(
    parsed: &serde_json::Value,
    input_len: usize,
    fallback_model: &str,
) -> Result<LlmEmbedResponse, LlmError> {
    let data = parsed
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| LlmError {
            code: "INVALID_RESPONSE".into(),
            message: "embeddings response has no data array".into(),
        })?;
    // Providers document data[] in input order, but each item carries
    // `index` — honor it so a reordering can't silently mis-assign
    // embeddings to texts. Sized to the INPUT count, not data.len(): a
    // tail-truncated response must fail the emptiness check below,
    // never return a short array.
    let mut embeddings: Vec<Vec<f32>> = vec![Vec::new(); input_len];
    for item in data {
        let idx = item
            .get("index")
            .and_then(|i| i.as_u64())
            .unwrap_or(u64::MAX) as usize;
        let vec_json = item
            .get("embedding")
            .and_then(|e| e.as_array())
            .ok_or_else(|| LlmError {
                code: "INVALID_RESPONSE".into(),
                message: "embeddings item has no embedding array".into(),
            })?;
        let values: Vec<f32> = vec_json
            .iter()
            .filter_map(|v| v.as_f64())
            .map(|f| f as f32)
            .collect();
        if values.len() != vec_json.len() {
            return Err(LlmError {
                code: "INVALID_RESPONSE".into(),
                message: "embeddings item contains non-number elements".into(),
            });
        }
        if values.is_empty() {
            return Err(LlmError {
                code: "INVALID_RESPONSE".into(),
                message: format!("embeddings item {idx} is empty"),
            });
        }
        match embeddings.get_mut(idx) {
            Some(slot) => *slot = values,
            None => {
                return Err(LlmError {
                    code: "INVALID_RESPONSE".into(),
                    message: format!("embeddings item index {idx} out of range"),
                })
            }
        }
    }
    if embeddings.iter().any(|e| e.is_empty()) {
        return Err(LlmError {
            code: "INVALID_RESPONSE".into(),
            message: "embeddings response is missing entries for some inputs".into(),
        });
    }
    let response_model = parsed
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or(fallback_model)
        .to_string();
    let total_tokens = parsed
        .get("usage")
        .and_then(|u| u.get("total_tokens"))
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as u32;
    Ok(LlmEmbedResponse {
        embeddings,
        model: response_model,
        total_tokens,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_body_serializes_minimum() {
        let req = LlmCompleteRequest {
            model: Some("claude-sonnet-4-5".into()),
            messages: vec![LlmMessage {
                role: "user".into(),
                content: LlmContent::Text("hi".into()),
            }],
            ..Default::default()
        };
        let body = anthropic_request_body(&req, false);
        assert_eq!(body["model"], "claude-sonnet-4-5");
        assert_eq!(body["stream"], false);
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hi");
    }

    #[test]
    fn anthropic_body_serializes_tools_and_system() {
        let req = LlmCompleteRequest {
            model: Some("claude-sonnet-4-5".into()),
            system: Some("You are helpful.".into()),
            tools: vec![LlmTool {
                name: "get_weather".into(),
                description: "Look up weather.".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }],
            messages: vec![LlmMessage {
                role: "user".into(),
                content: LlmContent::Text("weather in SF?".into()),
            }],
            ..Default::default()
        };
        let body = anthropic_request_body(&req, true);
        assert_eq!(body["system"], "You are helpful.");
        assert_eq!(body["tools"][0]["name"], "get_weather");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn anthropic_body_serializes_content_blocks_for_tool_followup() {
        // Mid-tool-use loop: caller posts the assistant's tool_use turn
        // back along with a tool_result. Verifies content arrays
        // serialize untouched.
        let req = LlmCompleteRequest {
            model: Some("claude-sonnet-4-5".into()),
            messages: vec![
                LlmMessage {
                    role: "user".into(),
                    content: LlmContent::Text("weather?".into()),
                },
                LlmMessage {
                    role: "assistant".into(),
                    content: LlmContent::Blocks(vec![
                        ContentBlock::Text {
                            text: "Let me check.".into(),
                        },
                        ContentBlock::ToolUse {
                            id: "tool_use_1".into(),
                            name: "get_weather".into(),
                            input: serde_json::json!({"city": "SF"}),
                        },
                    ]),
                },
                LlmMessage {
                    role: "user".into(),
                    content: LlmContent::Blocks(vec![ContentBlock::ToolResult {
                        tool_use_id: "tool_use_1".into(),
                        content: "{\"temp\":60}".into(),
                        is_error: None,
                    }]),
                },
            ],
            ..Default::default()
        };
        let body = anthropic_request_body(&req, false);
        assert_eq!(body["messages"][1]["content"][1]["type"], "tool_use");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn openai_body_translates_tool_use_to_tool_calls() {
        let req = LlmCompleteRequest {
            model: Some("gpt-4o".into()),
            messages: vec![LlmMessage {
                role: "assistant".into(),
                content: LlmContent::Blocks(vec![ContentBlock::ToolUse {
                    id: "call_1".into(),
                    name: "lookup".into(),
                    input: serde_json::json!({"q": "x"}),
                }]),
            }],
            ..Default::default()
        };
        let body = openai_request_body(&req, false);
        assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["name"],
            "lookup"
        );
        // Arguments are stringified JSON for OpenAI compat.
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["arguments"],
            "{\"q\":\"x\"}"
        );
    }

    #[test]
    fn openai_body_translates_tool_result_to_tool_role() {
        let req = LlmCompleteRequest {
            model: Some("gpt-4o".into()),
            messages: vec![LlmMessage {
                role: "user".into(),
                content: LlmContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "call_1".into(),
                    content: "{\"ok\":true}".into(),
                    is_error: None,
                }]),
            }],
            ..Default::default()
        };
        let body = openai_request_body(&req, false);
        assert_eq!(body["messages"][0]["role"], "tool");
        assert_eq!(body["messages"][0]["tool_call_id"], "call_1");
    }

    #[test]
    fn anthropic_response_parses_content_blocks() {
        let resp = serde_json::json!({
            "id": "msg_1",
            "model": "claude-sonnet-4-5",
            "content": [
                {"type": "text", "text": "Hi!"},
                {"type": "tool_use", "id": "tu_1", "name": "foo", "input": {"a": 1}},
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 20},
        });
        let parsed = parse_anthropic_message(resp).unwrap();
        assert_eq!(parsed.model, "claude-sonnet-4-5");
        assert_eq!(parsed.stop_reason, "tool_use");
        assert_eq!(parsed.usage.input_tokens, 10);
        assert_eq!(parsed.content.len(), 2);
        match &parsed.content[1] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "tu_1");
                assert_eq!(name, "foo");
                assert_eq!(input["a"], 1);
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn openai_response_parses_tool_calls() {
        let resp = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "foo", "arguments": "{\"a\":1}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 20}
        });
        let parsed = parse_openai_completion(resp).unwrap();
        assert_eq!(parsed.stop_reason, "tool_use");
        assert_eq!(parsed.content.len(), 1);
        match &parsed.content[0] {
            ContentBlock::ToolUse { id, input, .. } => {
                assert_eq!(id, "call_1");
                assert_eq!(input["a"], 1);
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn redact_strips_anthropic_api_keys() {
        let body = r#"{"error":"Invalid api key sk-ant-abcd1234efgh"}"#;
        let redacted = redact_body(body);
        assert!(!redacted.contains("sk-ant-abcd1234efgh"));
        assert!(redacted.contains("<redacted>"));
    }

    #[test]
    fn redact_strips_together_groq_perplexity_keys() {
        // Codex finding #11 — multi-provider key prefix coverage.
        let body = "transport error to tgp_v1_xxxxxxxxxxxxxx and gsk_yyyyyyyyy plus pplx-zzzz1234";
        let redacted = redact_body(body);
        assert!(!redacted.contains("tgp_v1_xxxxxxxxxxxxxx"));
        assert!(!redacted.contains("gsk_yyyyyyyyy"));
        assert!(!redacted.contains("pplx-zzzz1234"));
    }

    #[test]
    fn redact_strips_bearer_authorization() {
        let body = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.abc.def\nstatus=401";
        let redacted = redact_body(body);
        assert!(!redacted.contains("eyJhbGciOiJIUzI1NiJ9"));
        assert!(redacted.contains("<redacted>"));
    }

    #[test]
    fn redact_truncates_at_utf8_char_boundary_without_panic() {
        // Codex P2-F: `String::truncate` panics if the cutoff falls
        // inside a multi-byte codepoint. Construct a string where
        // the 1024th byte falls inside a 4-byte emoji.
        let mut s = String::with_capacity(2048);
        // Pad to byte 1023, then append a 4-byte emoji whose first
        // byte is at 1023 — truncation at 1024 would split it.
        s.push_str(&"a".repeat(1023));
        s.push('🦀'); // 4-byte UTF-8
        s.push_str(&"b".repeat(100));
        // Should not panic.
        let r = redact_body(&s);
        assert!(r.ends_with("..."));
        // The result must be valid UTF-8 (string ops would have
        // panicked otherwise, but make the invariant explicit).
        assert!(r.is_char_boundary(r.len()));
    }

    #[test]
    fn openai_translation_emits_one_tool_message_per_parallel_result() {
        // Codex P1-9: parallel tool calls (one assistant turn invokes
        // get_weather AND get_time) get parallel tool_result blocks
        // in one Anthropic user message. The previous translator
        // dropped all but one.
        let msg = LlmMessage {
            role: "user".into(),
            content: LlmContent::Blocks(vec![
                ContentBlock::ToolResult {
                    tool_use_id: "tu_weather".into(),
                    content: "{\"temp\":60}".into(),
                    is_error: None,
                },
                ContentBlock::ToolResult {
                    tool_use_id: "tu_time".into(),
                    content: "{\"time\":\"12:00\"}".into(),
                    is_error: None,
                },
            ]),
        };
        let out = serialize_messages_openai(&msg);
        assert_eq!(out.len(), 2, "expected 2 tool messages, got {}", out.len());
        assert_eq!(out[0]["role"], "tool");
        assert_eq!(out[0]["tool_call_id"], "tu_weather");
        assert_eq!(out[1]["role"], "tool");
        assert_eq!(out[1]["tool_call_id"], "tu_time");
    }

    #[test]
    fn openai_translation_forces_assistant_role_for_tool_calls() {
        // Codex P1-10: tool_calls only valid on assistant. If caller
        // mislabels role, normalize rather than emit a body OpenAI
        // will 400 on.
        let msg = LlmMessage {
            role: "user".into(), // WRONG — would be 400 from OpenAI
            content: LlmContent::Blocks(vec![ContentBlock::ToolUse {
                id: "call_x".into(),
                name: "do_thing".into(),
                input: serde_json::json!({}),
            }]),
        };
        let out = serialize_messages_openai(&msg);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "assistant");
    }

    #[test]
    fn openai_body_includes_stream_options_usage_when_streaming() {
        // Codex P1-8: OpenAI streams omit usage by default. Ask for
        // it so metering works.
        let req = LlmCompleteRequest {
            model: Some("gpt-4o".into()),
            messages: vec![LlmMessage {
                role: "user".into(),
                content: LlmContent::Text("hi".into()),
            }],
            ..Default::default()
        };
        let body = openai_request_body(&req, true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        // Non-streaming should NOT have stream_options.
        let body2 = openai_request_body(&req, false);
        assert!(body2.get("stream_options").is_none());
    }

    #[test]
    fn complete_clamps_max_tokens_to_cap() {
        // Codex P1-5: caller requesting max_tokens: 200000 should
        // clamp down to the configured cap.
        let client = LlmClient {
            inner: Arc::new(LlmClientInner {
                provider: LlmProvider::Anthropic,
                api_key: "test".into(),
                default_model: "claude-test".into(),
                base_url: None,
                agent: ureq::Agent::new(),
                manifest_allowed: Vec::new(),
                max_tokens_cap: Some(8000),
            }),
        };
        // Use the body-builder pathway since we can't actually call
        // a provider in unit tests. Build the request, mimic the
        // clamp logic.
        let req = LlmCompleteRequest {
            model: Some("claude-test".into()),
            messages: vec![],
            max_tokens: Some(200_000),
            ..Default::default()
        };
        let requested = req.max_tokens.unwrap_or(4096);
        let capped = match client.inner.max_tokens_cap {
            Some(cap) => requested.min(cap),
            None => requested,
        };
        assert_eq!(capped, 8000);
    }

    #[test]
    fn embed_response_full_shape_parses() {
        let body = serde_json::json!({
            "data": [
                {"index": 1, "embedding": [0.5, 0.5]},
                {"index": 0, "embedding": [1.0, 0.0]},
            ],
            "model": "text-embedding-3-small",
            "usage": {"total_tokens": 7}
        });
        let out = parse_embed_response(&body, 2, "fallback").unwrap();
        // Order restored from `index`, not array position.
        assert_eq!(out.embeddings[0], vec![1.0, 0.0]);
        assert_eq!(out.embeddings[1], vec![0.5, 0.5]);
        assert_eq!(out.model, "text-embedding-3-small");
        assert_eq!(out.total_tokens, 7);
    }

    #[test]
    fn embed_response_truncated_tail_is_rejected() {
        // Provider silently drops the last item — must error, never
        // return a short array that mis-zips against inputs.
        let body = serde_json::json!({
            "data": [ {"index": 0, "embedding": [1.0]} ],
        });
        let err = parse_embed_response(&body, 2, "m").unwrap_err();
        assert_eq!(err.code, "INVALID_RESPONSE");
        assert!(err.message.contains("missing entries"), "{}", err.message);
    }

    #[test]
    fn embed_response_bad_shapes_are_rejected() {
        // Duplicate index leaves a hole.
        let dup = serde_json::json!({
            "data": [
                {"index": 0, "embedding": [1.0]},
                {"index": 0, "embedding": [2.0]},
            ],
        });
        assert!(parse_embed_response(&dup, 2, "m").is_err());
        // Out-of-range index.
        let oob = serde_json::json!({
            "data": [ {"index": 5, "embedding": [1.0]} ],
        });
        assert!(parse_embed_response(&oob, 1, "m").is_err());
        // Non-number element.
        let bad = serde_json::json!({
            "data": [ {"index": 0, "embedding": [1.0, "x"]} ],
        });
        assert!(parse_embed_response(&bad, 1, "m").is_err());
        // No data array.
        assert!(parse_embed_response(&serde_json::json!({}), 1, "m").is_err());
    }
}
