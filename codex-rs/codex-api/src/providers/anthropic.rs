//! Anthropic provider adapter implementation.
//!
//! This adapter handles Anthropic Claude API-specific request building and
//! response parsing, including system message extraction, tool choice mapping,
//! and max_tokens defaults.

use http::HeaderMap;
use http::HeaderName;
use http::HeaderValue;
use serde_json::Value;
use serde_json::json;

use crate::common::ResponseEvent;
use crate::error::ApiError;
use crate::provider_adapter::ProviderAdapter;
use crate::provider_adapter::RequestOptions;
use crate::sse::anthropic::AnthropicStreamState;
use crate::sse::anthropic::is_completion_event;
use crate::sse::anthropic::parse_anthropic_event;
use crate::tools::ToolFormatter;
use crate::tools::anthropic::AnthropicToolFormatter;

/// Current Anthropic API version.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic Messages API adapter.
pub struct AnthropicAdapter {
    tool_formatter: AnthropicToolFormatter,
    /// Stream state for parsing SSE events.
    /// Note: This needs to be managed externally for proper stateful parsing.
    stream_state: std::sync::Mutex<AnthropicStreamState>,
}

impl AnthropicAdapter {
    pub fn new() -> Self {
        Self {
            tool_formatter: AnthropicToolFormatter::new(),
            stream_state: std::sync::Mutex::new(AnthropicStreamState::new()),
        }
    }

    /// Resets the stream state. Call this before starting a new stream.
    pub fn reset_stream_state(&self) {
        if let Ok(mut state) = self.stream_state.lock() {
            *state = AnthropicStreamState::new();
        }
    }
}

impl Default for AnthropicAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderAdapter for AnthropicAdapter {
    fn provider_id(&self) -> &str {
        "anthropic"
    }

    fn format_tools(&self, tools: &[Value]) -> Result<Vec<Value>, ApiError> {
        self.tool_formatter.format_tools(tools)
    }

    fn build_request_body(
        &self,
        model: &str,
        instructions: &str,
        input: &[Value],
        tools: &[Value],
        options: &RequestOptions,
    ) -> Result<Value, ApiError> {
        // Build messages array, extracting system messages
        let messages = build_anthropic_messages(input)?;

        // Determine max_tokens
        let max_tokens = default_max_tokens(model);

        // Build the request body
        let mut body = json!({
            "model": model,
            "messages": messages,
            "max_tokens": max_tokens,
            "stream": true
        });

        // Add system instruction if provided
        if !instructions.is_empty() {
            body["system"] = json!(instructions);
        }

        // Add tools if any
        if !tools.is_empty() {
            let formatted_tools = self.format_tools(tools)?;
            body["tools"] = json!(formatted_tools);

            // Tool choice configuration
            body["tool_choice"] = json!({"type": "auto"});
        }

        // Add thinking config if reasoning is requested
        if let Some(reasoning) = &options.reasoning {
            if let Some(effort) = reasoning.get("effort").and_then(|e| e.as_str()) {
                match effort {
                    "none" => { /* skip — no thinking requested */ }
                    _ => {
                        let budget = match effort {
                            "minimal" | "low" => 1024_u32,
                            "medium" => 10_000,
                            "high" => 32_000,
                            _ => max_tokens.saturating_sub(1).max(1024), // xhigh or unknown
                        };
                        let budget = budget.max(1024).min(max_tokens.saturating_sub(1));
                        body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
                    }
                }
            }
        }

        Ok(body)
    }

    fn parse_sse_event(
        &self,
        event_type: &str,
        data: &str,
    ) -> Result<Vec<ResponseEvent>, ApiError> {
        let mut state = self
            .stream_state
            .lock()
            .map_err(|_| ApiError::Stream("Failed to acquire stream state lock".to_string()))?;

        let events = parse_anthropic_event(event_type, data, &mut state)?;

        Ok(events)
    }

    fn streaming_endpoint(&self, _model: &str) -> String {
        "/messages".to_string()
    }

    fn extra_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();

        // Required anthropic-version header
        if let Ok(value) = HeaderValue::from_str(ANTHROPIC_VERSION) {
            headers.insert(HeaderName::from_static("anthropic-version"), value);
        }

        // Enable beta features for streaming
        if let Ok(value) = HeaderValue::from_str("messages-2023-12-15") {
            headers.insert(HeaderName::from_static("anthropic-beta"), value);
        }

        headers
    }

    fn is_completion_event(&self, event_type: &str) -> bool {
        is_completion_event(event_type)
    }

    fn auth_header_name(&self) -> &str {
        "x-api-key"
    }

    fn format_auth_header(&self, api_key: &str) -> String {
        // Anthropic uses the API key directly, not as Bearer token
        api_key.to_string()
    }
}

/// Converts input ResponseItems to Anthropic messages format.
///
/// System messages are NOT included in the messages array - they
/// should be passed separately via the `system` field.
fn build_anthropic_messages(input: &[Value]) -> Result<Vec<Value>, ApiError> {
    let mut messages = Vec::new();

    for item in input {
        let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match item_type {
            "message" => {
                let role = item.get("role").and_then(|r| r.as_str()).unwrap_or("user");

                // Skip system messages - they go in the system field
                if role == "system" {
                    continue;
                }

                let mut content_blocks = Vec::new();
                if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                    for block in content {
                        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match block_type {
                            "input_text" | "output_text" => {
                                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                    content_blocks.push(json!({
                                        "type": "text",
                                        "text": text
                                    }));
                                }
                            }
                            "input_image" => {
                                // Handle image content
                                if let Some(url) = block.get("image_url").and_then(|u| u.as_str()) {
                                    if url.starts_with("data:") {
                                        // Base64 data URL
                                        if let Some((media_type, data)) = parse_data_url(url) {
                                            content_blocks.push(json!({
                                                "type": "image",
                                                "source": {
                                                    "type": "base64",
                                                    "media_type": media_type,
                                                    "data": data
                                                }
                                            }));
                                        }
                                    } else {
                                        // URL reference
                                        content_blocks.push(json!({
                                            "type": "image",
                                            "source": {
                                                "type": "url",
                                                "url": url
                                            }
                                        }));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                if !content_blocks.is_empty() {
                    messages.push(json!({
                        "role": role,
                        "content": content_blocks
                    }));
                }
            }

            "function_call" => {
                // Anthropic tool_use blocks
                let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let arguments = item
                    .get("arguments")
                    .and_then(|a| a.as_str())
                    .unwrap_or("{}");

                let input: Value = serde_json::from_str(arguments).unwrap_or(json!({}));

                messages.push(json!({
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": call_id,
                        "name": name,
                        "input": input
                    }]
                }));
            }

            "function_call_output" => {
                // Anthropic tool_result blocks
                let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                let output = item.get("output").cloned().unwrap_or(json!({}));

                // Convert output to string content
                let content = match &output {
                    Value::String(s) => s.clone(),
                    _ => output.to_string(),
                };

                messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": content
                    }]
                }));
            }

            _ => {
                // Skip unsupported item types
            }
        }
    }

    Ok(messages)
}

/// Parses a data URL into (media_type, base64_data).
fn parse_data_url(url: &str) -> Option<(String, String)> {
    // Format: data:mime/type;base64,<data>
    let url = url.strip_prefix("data:")?;
    let (header, data) = url.split_once(',')?;
    let mime = header.strip_suffix(";base64")?;
    Some((mime.to_string(), data.to_string()))
}

/// Default max_tokens by model family.
///
/// Anthropic API requires max_tokens, so we provide sensible defaults
/// based on each model's capabilities.
pub fn default_max_tokens(model: &str) -> u32 {
    let model_lower = model.to_lowercase();

    if model_lower.contains("claude-3-haiku") || model_lower.contains("claude-3-opus") {
        4096
    } else if model_lower.contains("claude-3.5") || model_lower.contains("claude-3-5") {
        8192
    } else if model_lower.contains("claude-3.7")
        || model_lower.contains("claude-sonnet-4")
        || model_lower.contains("claude-opus-4.5")
        || model_lower.contains("claude-haiku-4.5")
    {
        64000
    } else if model_lower.contains("claude-opus-4") {
        32000
    } else {
        // Safe default for unknown models
        4096
    }
}

/// Maps tool choice to Anthropic format.
///
/// Anthropic supports:
/// - auto: Model decides whether to use tools
/// - any: Model must use at least one tool
/// - tool: Model must use a specific tool
pub fn map_tool_choice(choice: Option<&str>) -> Value {
    match choice {
        None | Some("auto") => json!({"type": "auto"}),
        Some("required") => json!({"type": "any"}),
        Some("none") => {
            // Anthropic doesn't have a "none" mode - just don't send tools
            json!({"type": "auto"})
        }
        Some(specific) => json!({
            "type": "tool",
            "name": specific
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_streaming_endpoint() {
        let adapter = AnthropicAdapter::new();
        assert_eq!(
            adapter.streaming_endpoint("claude-sonnet-4-20250514"),
            "/messages"
        );
    }

    #[test]
    fn test_auth_header() {
        let adapter = AnthropicAdapter::new();
        assert_eq!(adapter.auth_header_name(), "x-api-key");
        assert_eq!(
            adapter.format_auth_header("sk-ant-api03-xxx"),
            "sk-ant-api03-xxx"
        );
    }

    #[test]
    fn test_extra_headers() {
        let adapter = AnthropicAdapter::new();
        let headers = adapter.extra_headers();

        assert!(headers.contains_key("anthropic-version"));
    }

    #[test]
    fn test_default_max_tokens() {
        assert_eq!(default_max_tokens("claude-3-haiku-20240307"), 4096);
        assert_eq!(default_max_tokens("claude-3-opus-20240229"), 4096);
        assert_eq!(default_max_tokens("claude-3.5-sonnet-20240620"), 8192);
        assert_eq!(default_max_tokens("claude-3-5-sonnet-20241022"), 8192);
        assert_eq!(default_max_tokens("claude-sonnet-4-20250514"), 64000);
        assert_eq!(default_max_tokens("claude-opus-4-20250514"), 32000);
        assert_eq!(default_max_tokens("unknown-model"), 4096);
    }

    #[test]
    fn test_map_tool_choice() {
        assert_eq!(map_tool_choice(None), json!({"type": "auto"}));
        assert_eq!(map_tool_choice(Some("auto")), json!({"type": "auto"}));
        assert_eq!(map_tool_choice(Some("required")), json!({"type": "any"}));
        assert_eq!(
            map_tool_choice(Some("get_weather")),
            json!({"type": "tool", "name": "get_weather"})
        );
    }

    #[test]
    fn test_build_request_body() {
        let adapter = AnthropicAdapter::new();

        let input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Hello"}]
        })];

        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "test",
                "description": "A test function",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })];

        let options = RequestOptions::default();

        let body = adapter
            .build_request_body(
                "claude-sonnet-4-20250514",
                "You are helpful",
                &input,
                &tools,
                &options,
            )
            .unwrap();

        assert_eq!(body["model"], "claude-sonnet-4-20250514");
        assert!(body.get("messages").is_some());
        assert!(body.get("system").is_some());
        assert!(body.get("tools").is_some());
        assert_eq!(body["max_tokens"], 64000);
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn test_build_anthropic_messages() {
        let input = vec![
            json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "Hi"}]
            }),
            json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }),
        ];

        let messages = build_anthropic_messages(&input).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["type"], "text");
        assert_eq!(messages[0]["content"][0]["text"], "Hi");
        assert_eq!(messages[1]["role"], "assistant");
    }

    #[test]
    fn test_build_request_body_with_thinking_high() {
        let adapter = AnthropicAdapter::new();
        let input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Hello"}]
        })];
        let options = RequestOptions {
            reasoning: Some(json!({"effort": "high"})),
            ..Default::default()
        };
        let body = adapter
            .build_request_body(
                "claude-sonnet-4-20250514",
                "Be helpful",
                &input,
                &[],
                &options,
            )
            .unwrap();
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 32_000);
    }

    #[test]
    fn test_build_request_body_with_thinking_medium() {
        let adapter = AnthropicAdapter::new();
        let input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Hello"}]
        })];
        let options = RequestOptions {
            reasoning: Some(json!({"effort": "medium"})),
            ..Default::default()
        };
        let body = adapter
            .build_request_body("claude-sonnet-4-20250514", "", &input, &[], &options)
            .unwrap();
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 10_000);
    }

    #[test]
    fn test_build_request_body_with_thinking_low() {
        let adapter = AnthropicAdapter::new();
        let input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Hello"}]
        })];
        let options = RequestOptions {
            reasoning: Some(json!({"effort": "low"})),
            ..Default::default()
        };
        let body = adapter
            .build_request_body("claude-sonnet-4-20250514", "", &input, &[], &options)
            .unwrap();
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 1024);
    }

    #[test]
    fn test_build_request_body_with_thinking_none() {
        let adapter = AnthropicAdapter::new();
        let input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Hello"}]
        })];
        let options = RequestOptions {
            reasoning: Some(json!({"effort": "none"})),
            ..Default::default()
        };
        let body = adapter
            .build_request_body("claude-sonnet-4-20250514", "", &input, &[], &options)
            .unwrap();
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn test_build_request_body_no_reasoning() {
        let adapter = AnthropicAdapter::new();
        let input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Hello"}]
        })];
        let options = RequestOptions::default();
        let body = adapter
            .build_request_body("claude-sonnet-4-20250514", "", &input, &[], &options)
            .unwrap();
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn test_build_anthropic_messages_with_tool_use() {
        let input = vec![
            json!({
                "type": "function_call",
                "call_id": "toolu_123",
                "name": "get_weather",
                "arguments": "{\"location\": \"SF\"}"
            }),
            json!({
                "type": "function_call_output",
                "call_id": "toolu_123",
                "output": "Sunny, 72F"
            }),
        ];

        let messages = build_anthropic_messages(&input).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["content"][0]["type"], "tool_use");
        assert_eq!(messages[0]["content"][0]["id"], "toolu_123");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"][0]["type"], "tool_result");
        assert_eq!(messages[1]["content"][0]["tool_use_id"], "toolu_123");
    }
}
