use crate::Plugin;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Supported AI providers.
#[derive(Debug, Clone)]
pub enum AiProvider {
    Anthropic { api_key: String, model: String },
    OpenAI { api_key: String, model: String },
    Custom { base_url: String, api_key: String, model: Option<String> },
}

/// A single message in a conversation.
#[derive(Debug, Clone)]
pub struct AiMessage {
    pub role: String,
    pub content: String,
}

impl AiMessage {
    pub fn system(content: &str) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }

    pub fn user(content: &str) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }

    pub fn assistant(content: &str) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// AI proxy plugin
// ---------------------------------------------------------------------------

/// Proxies requests to LLM providers with streaming support.
///
/// Supports Anthropic, OpenAI, and any OpenAI-compatible custom endpoint
/// (Ollama, Together, Groq, etc.). HTTPS endpoints require a TLS-terminating
/// reverse proxy; plain HTTP endpoints (e.g. local Ollama) work directly.
pub struct AiProxyPlugin {
    provider: AiProvider,
}

impl AiProxyPlugin {
    pub fn anthropic(api_key: &str, model: &str) -> Self {
        Self {
            provider: AiProvider::Anthropic {
                api_key: api_key.to_string(),
                model: model.to_string(),
            },
        }
    }

    pub fn openai(api_key: &str, model: &str) -> Self {
        Self {
            provider: AiProvider::OpenAI {
                api_key: api_key.to_string(),
                model: model.to_string(),
            },
        }
    }

    pub fn custom(base_url: &str, api_key: &str) -> Self {
        Self {
            provider: AiProvider::Custom {
                base_url: base_url.to_string(),
                api_key: api_key.to_string(),
                model: None,
            },
        }
    }

    /// Create a custom provider with an explicit model name included in
    /// the request body sent to the upstream endpoint.
    pub fn custom_with_model(base_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            provider: AiProvider::Custom {
                base_url: base_url.to_string(),
                api_key: api_key.to_string(),
                model: if model.is_empty() { None } else { Some(model.to_string()) },
            },
        }
    }

    /// Returns a reference to the configured provider.
    pub fn provider(&self) -> &AiProvider {
        &self.provider
    }

    /// Stream a completion request to the configured provider.
    ///
    /// Calls `on_chunk` for each text token received from the provider.
    /// Returns the full accumulated response text on success.
    pub fn stream_completion(
        &self,
        messages: &[AiMessage],
        on_chunk: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        match &self.provider {
            AiProvider::Anthropic { api_key, model } => {
                self.stream_anthropic(api_key, model, messages, on_chunk)
            }
            AiProvider::OpenAI { api_key, model } => {
                self.stream_openai(api_key, model, messages, on_chunk)
            }
            AiProvider::Custom { base_url, api_key, model } => {
                self.stream_custom(base_url, api_key, model.as_deref(), messages, on_chunk)
            }
        }
    }

    /// Non-streaming convenience wrapper. Waits for the full response.
    pub fn completion(&self, messages: &[AiMessage]) -> Result<String, String> {
        let mut full = String::new();
        self.stream_completion(messages, &mut |chunk| {
            full.push_str(chunk);
        })?;
        Ok(full)
    }

    // -----------------------------------------------------------------------
    // Provider-specific streaming
    // -----------------------------------------------------------------------

    fn stream_anthropic(
        &self,
        api_key: &str,
        model: &str,
        messages: &[AiMessage],
        on_chunk: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::json!({"role": m.role, "content": m.content}))
            .collect();

        let body = serde_json::json!({
            "model": model,
            "max_tokens": 4096,
            "stream": true,
            "messages": msgs,
        })
        .to_string();

        self.stream_https_request(
            "api.anthropic.com",
            443,
            "/v1/messages",
            &[
                ("x-api-key", api_key),
                ("anthropic-version", "2023-06-01"),
                ("content-type", "application/json"),
            ],
            &body,
            on_chunk,
            parse_anthropic_sse,
        )
    }

    fn stream_openai(
        &self,
        api_key: &str,
        model: &str,
        messages: &[AiMessage],
        on_chunk: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::json!({"role": m.role, "content": m.content}))
            .collect();

        let body = serde_json::json!({
            "model": model,
            "stream": true,
            "max_tokens": 4096,
            "messages": msgs,
        })
        .to_string();

        self.stream_https_request(
            "api.openai.com",
            443,
            "/v1/chat/completions",
            &[
                ("Authorization", &format!("Bearer {api_key}")),
                ("Content-Type", "application/json"),
            ],
            &body,
            on_chunk,
            parse_openai_sse,
        )
    }

    fn stream_custom(
        &self,
        base_url: &str,
        api_key: &str,
        model: Option<&str>,
        messages: &[AiMessage],
        on_chunk: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        let is_https = base_url.starts_with("https://");
        let url = base_url
            .strip_prefix("https://")
            .or_else(|| base_url.strip_prefix("http://"))
            .unwrap_or(base_url);

        let (host, path) = match url.find('/') {
            Some(i) => (&url[..i], &url[i..]),
            None => (url, "/v1/chat/completions"),
        };

        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::json!({"role": m.role, "content": m.content}))
            .collect();

        let mut body_value = serde_json::json!({
            "stream": true,
            "messages": msgs,
        });

        // Include model in the request body when configured.
        if let Some(m) = model {
            body_value["model"] = serde_json::json!(m);
        }

        let body = body_value.to_string();

        if is_https {
            let port = 443;
            return self.stream_https_request(
                host,
                port,
                path,
                &[
                    ("Authorization", &format!("Bearer {api_key}")),
                    ("Content-Type", "application/json"),
                ],
                &body,
                on_chunk,
                parse_openai_sse,
            );
        }

        self.stream_http_request(host, 80, path, api_key, &body, on_chunk)
    }

    // -----------------------------------------------------------------------
    // Transport
    // -----------------------------------------------------------------------

    /// HTTPS transport stub. Real HTTPS requires a TLS library (rustls or
    /// native-tls) which we deliberately avoid to keep the dependency tree
    /// minimal. Users who need HTTPS should either:
    ///   - Use a local TLS-terminating proxy (nginx, caddy, stunnel).
    ///   - Use a plain-HTTP custom endpoint (e.g. local Ollama on port 11434).
    fn stream_https_request(
        &self,
        _host: &str,
        _port: u16,
        _path: &str,
        _headers: &[(&str, &str)],
        _body: &str,
        _on_chunk: &mut dyn FnMut(&str),
        _parse_chunk: fn(&str) -> Option<String>,
    ) -> Result<String, String> {
        Err(
            "HTTPS streaming requires a TLS library. Configure a TLS-terminating \
             reverse proxy or use a plain-HTTP custom endpoint (e.g. Ollama)."
                .into(),
        )
    }

    /// Plain HTTP streaming for local/custom endpoints (Ollama, vLLM, etc.).
    fn stream_http_request(
        &self,
        host: &str,
        port: u16,
        path: &str,
        api_key: &str,
        body: &str,
        on_chunk: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        let addr = format!("{host}:{port}");
        let mut stream =
            TcpStream::connect(&addr).map_err(|e| format!("Connection failed: {e}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(120))).ok();

        // Build raw HTTP/1.1 request.
        let mut req = format!(
            "POST {path} HTTP/1.1\r\n\
             Host: {host}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: keep-alive\r\n",
            body.len()
        );
        if !api_key.is_empty() {
            req.push_str(&format!("Authorization: Bearer {api_key}\r\n"));
        }
        req.push_str("\r\n");
        req.push_str(body);

        stream
            .write_all(req.as_bytes())
            .map_err(|e| format!("Write failed: {e}"))?;

        // Consume response headers.
        let mut reader = BufReader::new(stream);
        let mut header_line = String::new();
        let mut status_code: u16 = 0;
        let mut first_line = true;
        loop {
            header_line.clear();
            reader
                .read_line(&mut header_line)
                .map_err(|e| format!("Read failed: {e}"))?;
            if first_line {
                // Parse "HTTP/1.1 200 OK"
                status_code = header_line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                first_line = false;
            }
            if header_line.trim().is_empty() {
                break;
            }
        }

        if status_code != 200 {
            // Read the error body (up to 4 KB).
            let mut err_body = vec![0u8; 4096];
            let n = reader.read(&mut err_body).unwrap_or(0);
            let err_text = String::from_utf8_lossy(&err_body[..n]);
            return Err(format!(
                "Provider returned HTTP {status_code}: {err_text}"
            ));
        }

        // Read SSE data lines.
        let mut full_response = String::new();
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Some(text) = parse_openai_sse(trimmed) {
                        full_response.push_str(&text);
                        on_chunk(&text);
                    }
                    // Check for [DONE] sentinel.
                    if trimmed == "data: [DONE]" {
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        Ok(full_response)
    }
}

impl Plugin for AiProxyPlugin {
    fn name(&self) -> &str {
        "ai-proxy"
    }
}

// ---------------------------------------------------------------------------
// SSE parsers — free functions so they can be passed as fn pointers
// ---------------------------------------------------------------------------

/// Extract text from an Anthropic SSE data line.
///
/// Anthropic sends `content_block_delta` events with
/// `{"delta":{"type":"text_delta","text":"..."}}`.
fn parse_anthropic_sse(line: &str) -> Option<String> {
    let data = line.strip_prefix("data: ")?;
    let parsed: serde_json::Value = serde_json::from_str(data).ok()?;
    if parsed.get("type").and_then(|t| t.as_str()) != Some("content_block_delta") {
        return None;
    }
    let delta = parsed.get("delta")?;
    // Only extract text from text_delta events; ignore tool_use or other delta types.
    if delta.get("type").and_then(|t| t.as_str()) != Some("text_delta") {
        return None;
    }
    delta
        .get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
}

/// Extract text from an OpenAI-compatible SSE data line.
///
/// OpenAI (and compatible APIs like Ollama, Together, Groq) sends
/// `{"choices":[{"delta":{"content":"..."}}]}`.
fn parse_openai_sse(line: &str) -> Option<String> {
    let data = line.strip_prefix("data: ")?;
    if data == "[DONE]" {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(data).ok()?;
    parsed
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("delta"))
        .and_then(|d| d.get("content"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_anthropic_provider() {
        let plugin = AiProxyPlugin::anthropic("sk-ant-test", "claude-sonnet-4-20250514");
        match plugin.provider() {
            AiProvider::Anthropic { api_key, model } => {
                assert_eq!(api_key, "sk-ant-test");
                assert_eq!(model, "claude-sonnet-4-20250514");
            }
            _ => panic!("Expected Anthropic provider"),
        }
    }

    #[test]
    fn creates_openai_provider() {
        let plugin = AiProxyPlugin::openai("sk-test", "gpt-4");
        match plugin.provider() {
            AiProvider::OpenAI { api_key, model } => {
                assert_eq!(api_key, "sk-test");
                assert_eq!(model, "gpt-4");
            }
            _ => panic!("Expected OpenAI provider"),
        }
    }

    #[test]
    fn creates_custom_provider() {
        let plugin = AiProxyPlugin::custom("http://localhost:11434/v1/chat/completions", "key");
        match plugin.provider() {
            AiProvider::Custom { base_url, api_key, model } => {
                assert_eq!(base_url, "http://localhost:11434/v1/chat/completions");
                assert_eq!(api_key, "key");
                assert!(model.is_none());
            }
            _ => panic!("Expected Custom provider"),
        }
    }

    #[test]
    fn creates_custom_provider_with_model() {
        let plugin = AiProxyPlugin::custom_with_model("http://localhost:11434", "key", "llama3");
        match plugin.provider() {
            AiProvider::Custom { base_url, api_key, model } => {
                assert_eq!(base_url, "http://localhost:11434");
                assert_eq!(api_key, "key");
                assert_eq!(model.as_deref(), Some("llama3"));
            }
            _ => panic!("Expected Custom provider"),
        }
    }

    #[test]
    fn custom_with_empty_model_stores_none() {
        let plugin = AiProxyPlugin::custom_with_model("http://localhost:11434", "key", "");
        match plugin.provider() {
            AiProvider::Custom { model, .. } => {
                assert!(model.is_none());
            }
            _ => panic!("Expected Custom provider"),
        }
    }

    #[test]
    fn ai_message_constructors() {
        let sys = AiMessage::system("You are helpful.");
        assert_eq!(sys.role, "system");
        assert_eq!(sys.content, "You are helpful.");

        let user = AiMessage::user("Hello!");
        assert_eq!(user.role, "user");
        assert_eq!(user.content, "Hello!");

        let asst = AiMessage::assistant("Hi there.");
        assert_eq!(asst.role, "assistant");
        assert_eq!(asst.content, "Hi there.");
    }

    #[test]
    fn plugin_name() {
        let plugin = AiProxyPlugin::openai("key", "model");
        assert_eq!(plugin.name(), "ai-proxy");
    }

    #[test]
    fn completion_without_server_returns_error() {
        // Attempting to reach an unreachable host should return an error,
        // not panic.
        let plugin = AiProxyPlugin::custom("http://127.0.0.1:19999", "");
        let msgs = vec![AiMessage::user("hi")];
        let result = plugin.completion(&msgs);
        assert!(result.is_err());
    }

    #[test]
    fn parse_anthropic_sse_extracts_text() {
        let line = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        assert_eq!(parse_anthropic_sse(line), Some("Hello".to_string()));
    }

    #[test]
    fn parse_anthropic_sse_ignores_non_delta() {
        let line = r#"data: {"type":"message_start","message":{}}"#;
        assert_eq!(parse_anthropic_sse(line), None);
    }

    #[test]
    fn parse_anthropic_sse_ignores_non_text_delta() {
        // A content_block_delta with a non-text_delta type (e.g. tool_use)
        // should be ignored.
        let line = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"x\":1}"}}"#;
        assert_eq!(parse_anthropic_sse(line), None);
    }

    #[test]
    fn parse_openai_sse_extracts_content() {
        let line = r#"data: {"id":"x","choices":[{"index":0,"delta":{"content":" world"}}]}"#;
        assert_eq!(parse_openai_sse(line), Some(" world".to_string()));
    }

    #[test]
    fn parse_openai_sse_handles_done() {
        assert_eq!(parse_openai_sse("data: [DONE]"), None);
    }

    #[test]
    fn parse_openai_sse_ignores_non_data() {
        assert_eq!(parse_openai_sse("event: message"), None);
    }

    #[test]
    fn https_returns_informative_error() {
        let plugin = AiProxyPlugin::anthropic("key", "model");
        let msgs = vec![AiMessage::user("hi")];
        let result = plugin.completion(&msgs);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("TLS"), "Error should mention TLS: {err}");
    }
}
