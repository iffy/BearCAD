//! Turning a conversation into an HTTP request, and a stream of bytes back into text
//! (#1596).
//!
//! Two wire shapes cover every backend BearCAD talks to:
//!
//! - **Anthropic** `POST /v1/messages`, keyed by `x-api-key`.
//! - **OpenAI chat completions** `POST /chat/completions`, keyed by `Authorization: Bearer`
//!   — which xAI, Ollama, LM Studio, vLLM and every gateway also implement.
//!
//! Everything here is pure: a request in, a request out; an SSE payload in, deltas out. The
//! socket work lives in [`super::transport`], which keeps this module testable without a
//! network.

use serde_json::{json, Value};

use super::backends::{Backend, Provider};

/// Who said a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    fn wire_name(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// One turn of the conversation, as sent to the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub text: String,
}

#[allow(dead_code)] // Constructors used by tests and by the MCP tool surface (#1605).
impl ChatMessage {
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, text: text.into() }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self { role: Role::Assistant, text: text.into() }
    }
}

/// Token counts for one exchange. Providers report these differently and not all report
/// every field; a zero means "not reported", never "free".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Tokens served from the provider's prompt cache, billed at a lower rate.
    pub cache_read_tokens: u64,
    /// Tokens written into the prompt cache, billed at a higher rate.
    pub cache_write_tokens: u64,
}

impl Usage {
    /// Fold in a later report. Providers send input counts at the start of a stream and
    /// output counts at the end, so the two arrive separately; a non-zero value never gets
    /// overwritten by a zero.
    pub fn merge(&mut self, other: Usage) {
        for (slot, value) in [
            (&mut self.input_tokens, other.input_tokens),
            (&mut self.output_tokens, other.output_tokens),
            (&mut self.cache_read_tokens, other.cache_read_tokens),
            (&mut self.cache_write_tokens, other.cache_write_tokens),
        ] {
            if value > 0 {
                *slot = value;
            }
        }
    }

    /// Total tokens, cached ones included.
    #[allow(dead_code)] // The cost readout (#1599) shows this.
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

/// A request ready to put on the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    pub url: String,
    /// Header name/value pairs. The key header is included when the backend has a key.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpRequest {
    /// The request with every credential replaced by `<redacted>` — for diagnostics and
    /// error messages, which must never carry the key (#1595).
    #[allow(dead_code)] // Exercised by tests; the privacy audit (#1609) wires it into diag.
    pub fn redacted(&self) -> HttpRequest {
        let headers = self
            .headers
            .iter()
            .map(|(name, value)| {
                let redact = name.eq_ignore_ascii_case("x-api-key")
                    || name.eq_ignore_ascii_case("authorization");
                (
                    name.clone(),
                    if redact { "<redacted>".to_string() } else { value.clone() },
                )
            })
            .collect();
        HttpRequest { url: self.url.clone(), headers, body: self.body.clone() }
    }
}

/// One piece of a streamed reply.
#[derive(Clone, Debug, PartialEq)]
pub enum Delta {
    /// Text to append to the reply.
    Text(String),
    /// Token counts reported mid-stream.
    Usage(Usage),
    /// The provider says the reply is complete.
    Done,
}

/// How many tokens a reply may run to. Anthropic requires the field; the OpenAI shape
/// leaves it to the server when omitted, which is what a local model wants.
const ANTHROPIC_MAX_TOKENS: u32 = 8192;

/// Build the HTTP request for one exchange.
///
/// `system` is the instruction block (the document context lives there, #1597) and
/// `messages` is the conversation so far, oldest first.
pub fn build_request(backend: &Backend, system: &str, messages: &[ChatMessage]) -> HttpRequest {
    match backend.provider {
        Provider::Anthropic => anthropic_request(backend, system, messages),
        _ => openai_request(backend, system, messages),
    }
}

fn anthropic_request(backend: &Backend, system: &str, messages: &[ChatMessage]) -> HttpRequest {
    let mut headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("anthropic-version".to_string(), "2023-06-01".to_string()),
    ];
    if let Some(key) = backend.resolve_key() {
        headers.push(("x-api-key".to_string(), key));
    }
    let mut body = json!({
        "model": backend.model,
        "max_tokens": ANTHROPIC_MAX_TOKENS,
        "stream": true,
        "messages": messages
            .iter()
            .map(|m| json!({ "role": m.role.wire_name(), "content": m.text }))
            .collect::<Vec<_>>(),
    });
    // `thinking` is deliberately absent: current Claude models think adaptively when it is
    // omitted, and the reasoning is not something the pane renders.
    if !system.trim().is_empty() {
        body["system"] = Value::String(system.to_string());
    }
    HttpRequest {
        url: format!("{}/v1/messages", backend.effective_base_url()),
        headers,
        body: body.to_string(),
    }
}

fn openai_request(backend: &Backend, system: &str, messages: &[ChatMessage]) -> HttpRequest {
    let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
    if let Some(key) = backend.resolve_key() {
        headers.push(("authorization".to_string(), format!("Bearer {key}")));
    }
    // The system prompt is the first message in this shape, not a field of its own.
    let mut wire: Vec<Value> = Vec::with_capacity(messages.len() + 1);
    if !system.trim().is_empty() {
        wire.push(json!({ "role": "system", "content": system }));
    }
    wire.extend(
        messages
            .iter()
            .map(|m| json!({ "role": m.role.wire_name(), "content": m.text })),
    );
    // No max_tokens: the field was renamed to `max_completion_tokens` on newer OpenAI
    // models and local servers have their own ceilings, so leaving it out is the one
    // choice that works everywhere.
    let body = json!({
        "model": backend.model,
        "stream": true,
        // Without this, the OpenAI shape reports no usage at all when streaming, and the
        // cost readout (#1599) would have nothing to show.
        "stream_options": { "include_usage": true },
        "messages": wire,
    });
    HttpRequest {
        url: format!("{}/chat/completions", backend.effective_base_url()),
        headers,
        body: body.to_string(),
    }
}

/// Parse one SSE `data:` payload into deltas. Unknown events yield nothing — providers add
/// event types over time, and an unrecognised one is not an error.
pub fn parse_delta(provider: Provider, data: &str) -> Vec<Delta> {
    let data = data.trim();
    if data.is_empty() {
        return Vec::new();
    }
    // The OpenAI shape ends its stream with a literal sentinel rather than JSON.
    if data == "[DONE]" {
        return vec![Delta::Done];
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    match provider {
        Provider::Anthropic => anthropic_delta(&value),
        _ => openai_delta(&value),
    }
}

fn anthropic_delta(value: &Value) -> Vec<Delta> {
    let mut out = Vec::new();
    match value.get("type").and_then(Value::as_str) {
        Some("content_block_delta") => {
            // Only text: `thinking_delta` and tool input deltas are not shown in the pane.
            if let Some(text) = value
                .get("delta")
                .filter(|d| d.get("type").and_then(Value::as_str) == Some("text_delta"))
                .and_then(|d| d.get("text"))
                .and_then(Value::as_str)
            {
                out.push(Delta::Text(text.to_string()));
            }
        }
        Some("message_start") => {
            if let Some(usage) = value.get("message").and_then(|m| m.get("usage")) {
                out.push(Delta::Usage(anthropic_usage(usage)));
            }
        }
        Some("message_delta") => {
            if let Some(usage) = value.get("usage") {
                out.push(Delta::Usage(anthropic_usage(usage)));
            }
        }
        Some("message_stop") => out.push(Delta::Done),
        Some("error") => {
            // An error mid-stream arrives as an event, not an HTTP status.
            let message = value
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("the provider reported an error");
            out.push(Delta::Text(format!("\n\n[{message}]")));
            out.push(Delta::Done);
        }
        _ => {}
    }
    out
}

fn anthropic_usage(usage: &Value) -> Usage {
    Usage {
        input_tokens: field(usage, "input_tokens"),
        output_tokens: field(usage, "output_tokens"),
        cache_read_tokens: field(usage, "cache_read_input_tokens"),
        cache_write_tokens: field(usage, "cache_creation_input_tokens"),
    }
}

fn openai_delta(value: &Value) -> Vec<Delta> {
    let mut out = Vec::new();
    if let Some(text) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("content"))
        .and_then(Value::as_str)
    {
        if !text.is_empty() {
            out.push(Delta::Text(text.to_string()));
        }
    }
    if let Some(usage) = value.get("usage").filter(|u| !u.is_null()) {
        out.push(Delta::Usage(Usage {
            input_tokens: field(usage, "prompt_tokens"),
            output_tokens: field(usage, "completion_tokens"),
            // Reported (when at all) nested under the prompt-token details.
            cache_read_tokens: usage
                .get("prompt_tokens_details")
                .map(|d| field(d, "cached_tokens"))
                .unwrap_or(0),
            cache_write_tokens: 0,
        }));
    }
    out
}

fn field(value: &Value, name: &str) -> u64 {
    value.get(name).and_then(Value::as_u64).unwrap_or(0)
}

/// A readable one-line explanation of a failed request. Providers bury the useful part in
/// different places; a raw JSON blob in the chat thread helps nobody.
pub fn error_message(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            // Anthropic: {"error":{"message":…}}. OpenAI: the same. Ollama: {"error":"…"}.
            let error = value.get("error")?.clone();
            error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| error.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                "no details".to_string()
            } else {
                trimmed.chars().take(200).collect()
            }
        });
    match status {
        401 | 403 => format!("{status}: {detail} — check the backend's API key"),
        404 => format!("{status}: {detail} — check the model name and URL"),
        429 => format!("{status}: {detail} — rate limited, try again shortly"),
        _ => format!("{status}: {detail}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::backends::KeySource;

    fn anthropic() -> Backend {
        Backend {
            key: KeySource::Stored("sk-test-key".into()),
            ..Backend::preset(Provider::Anthropic)
        }
    }

    fn openai() -> Backend {
        Backend {
            key: KeySource::Stored("sk-openai-key".into()),
            ..Backend::preset(Provider::OpenAi)
        }
    }

    fn body_of(request: &HttpRequest) -> Value {
        serde_json::from_str(&request.body).expect("request body is JSON")
    }

    fn header<'a>(request: &'a HttpRequest, name: &str) -> Option<&'a str> {
        request
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn the_anthropic_request_targets_the_messages_endpoint_and_streams() {
        let request = build_request(
            &anthropic(),
            "You are looking at a CAD document.",
            &[ChatMessage::user("How wide is it?")],
        );
        assert_eq!(request.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(header(&request, "x-api-key"), Some("sk-test-key"));
        assert_eq!(header(&request, "anthropic-version"), Some("2023-06-01"));

        let body = body_of(&request);
        assert_eq!(body["model"], "claude-opus-5");
        assert_eq!(body["stream"], true);
        assert!(body["max_tokens"].is_number(), "Anthropic requires max_tokens");
        // The system prompt is its own field here, not a message.
        assert_eq!(body["system"], "You are looking at a CAD document.");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "How wide is it?");
        // Adaptive thinking is what current models do when the field is absent.
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn the_openai_request_carries_the_system_prompt_as_the_first_message() {
        let request = build_request(
            &openai(),
            "System text",
            &[
                ChatMessage::user("first"),
                ChatMessage::assistant("reply"),
                ChatMessage::user("second"),
            ],
        );
        assert_eq!(request.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(header(&request, "authorization"), Some("Bearer sk-openai-key"));

        let body = body_of(&request);
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true, "usage drives the cost readout");
        // max_tokens is deliberately absent — the field name differs across servers.
        assert!(body.get("max_tokens").is_none());
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[3]["content"], "second");
    }

    #[test]
    fn a_keyless_local_backend_sends_no_authorization_header() {
        let request = build_request(
            &Backend::preset(Provider::OpenAiCompatible),
            "",
            &[ChatMessage::user("hi")],
        );
        assert_eq!(request.url, "http://localhost:11434/v1/chat/completions");
        assert!(header(&request, "authorization").is_none());
        // An empty system prompt adds no message at all.
        assert_eq!(body_of(&request)["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_custom_base_url_is_honoured_without_doubling_slashes() {
        let backend = Backend {
            base_url: "https://gateway.example.com/v1/".into(),
            ..Backend::preset(Provider::XAi)
        };
        let request = build_request(&backend, "", &[ChatMessage::user("hi")]);
        assert_eq!(request.url, "https://gateway.example.com/v1/chat/completions");
    }

    #[test]
    fn a_redacted_request_keeps_everything_but_the_credentials() {
        let request = build_request(&anthropic(), "sys", &[ChatMessage::user("hi")]);
        let redacted = request.redacted();
        assert_eq!(redacted.url, request.url);
        assert_eq!(redacted.body, request.body);
        assert_eq!(header(&redacted, "x-api-key"), Some("<redacted>"));
        assert!(!format!("{redacted:?}").contains("sk-test-key"));

        let openai = build_request(&openai(), "sys", &[ChatMessage::user("hi")]).redacted();
        assert_eq!(header(&openai, "authorization"), Some("<redacted>"));
    }

    #[test]
    fn anthropic_stream_events_become_text_and_usage() {
        let events = [
            r#"{"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":1200,"output_tokens":1,"cache_read_input_tokens":800,"cache_creation_input_tokens":40}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"The box "}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"is 80 mm wide."}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#,
            r#"{"type":"message_stop"}"#,
        ];
        let mut text = String::new();
        let mut usage = Usage::default();
        let mut done = false;
        for event in events {
            for delta in parse_delta(Provider::Anthropic, event) {
                match delta {
                    Delta::Text(t) => text.push_str(&t),
                    Delta::Usage(u) => usage.merge(u),
                    Delta::Done => done = true,
                }
            }
        }
        assert_eq!(text, "The box is 80 mm wide.", "thinking deltas are not shown");
        assert_eq!(usage.input_tokens, 1200);
        assert_eq!(usage.output_tokens, 42, "the final count replaces the initial 1");
        assert_eq!(usage.cache_read_tokens, 800);
        assert_eq!(usage.cache_write_tokens, 40);
        assert!(done);
    }

    #[test]
    fn openai_stream_chunks_become_text_and_usage() {
        let events = [
            r#"{"choices":[{"delta":{"role":"assistant","content":""}}]}"#,
            r#"{"choices":[{"delta":{"content":"Two "}}]}"#,
            r#"{"choices":[{"delta":{"content":"bodies."}}],"usage":null}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":900,"completion_tokens":12,"prompt_tokens_details":{"cached_tokens":128}}}"#,
            "[DONE]",
        ];
        let mut text = String::new();
        let mut usage = Usage::default();
        let mut done = false;
        for event in events {
            for delta in parse_delta(Provider::OpenAi, event) {
                match delta {
                    Delta::Text(t) => text.push_str(&t),
                    Delta::Usage(u) => usage.merge(u),
                    Delta::Done => done = true,
                }
            }
        }
        assert_eq!(text, "Two bodies.");
        assert_eq!(usage.input_tokens, 900);
        assert_eq!(usage.output_tokens, 12);
        assert_eq!(usage.cache_read_tokens, 128);
        assert!(done, "[DONE] ends an OpenAI-shaped stream");
    }

    #[test]
    fn an_unknown_or_malformed_event_is_ignored_rather_than_fatal() {
        assert!(parse_delta(Provider::Anthropic, r#"{"type":"ping"}"#).is_empty());
        assert!(parse_delta(Provider::Anthropic, "not json at all").is_empty());
        assert!(parse_delta(Provider::OpenAi, "").is_empty());
        assert!(parse_delta(Provider::OpenAi, r#"{"choices":[{"delta":{}}]}"#).is_empty());
    }

    #[test]
    fn a_mid_stream_error_event_shows_up_in_the_reply_and_ends_it() {
        let deltas = parse_delta(
            Provider::Anthropic,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        );
        assert_eq!(
            deltas,
            vec![Delta::Text("\n\n[Overloaded]".into()), Delta::Done]
        );
    }

    #[test]
    fn error_bodies_turn_into_one_readable_line() {
        assert_eq!(
            error_message(401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#),
            "401: invalid x-api-key — check the backend's API key"
        );
        assert_eq!(
            error_message(404, r#"{"error":{"message":"model not found","type":"invalid_request_error"}}"#),
            "404: model not found — check the model name and URL"
        );
        // Ollama's shape: a bare string.
        assert_eq!(error_message(400, r#"{"error":"model 'llama9' not found"}"#), "400: model 'llama9' not found");
        // Anything else still says something useful.
        assert_eq!(error_message(500, "<html>Gateway error</html>"), "500: <html>Gateway error</html>");
        assert_eq!(error_message(502, ""), "502: no details");
    }

    #[test]
    fn usage_merging_never_replaces_a_count_with_a_missing_one() {
        let mut usage = Usage { input_tokens: 10, output_tokens: 5, ..Usage::default() };
        usage.merge(Usage { output_tokens: 20, ..Usage::default() });
        assert_eq!(usage.input_tokens, 10, "a later report without input keeps the first");
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.total(), 30);
    }
}
