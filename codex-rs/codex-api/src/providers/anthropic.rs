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

use crate::error::ApiError;
use crate::provider_adapter::ProviderAdapter;
use crate::provider_adapter::RequestOptions;
use crate::tools::ToolFormatter;
use crate::tools::anthropic::AnthropicToolFormatter;

/// Current Anthropic API version.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic Messages API adapter.
pub struct AnthropicAdapter {
    tool_formatter: AnthropicToolFormatter,
}

impl AnthropicAdapter {
    pub fn new() -> Self {
        Self {
            tool_formatter: AnthropicToolFormatter::new(),
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
        let is_opus_4_6 = model.to_lowercase().contains("claude-opus-4-6");
        if let Some(reasoning) = &options.reasoning {
            if let Some(effort) = reasoning.get("effort").and_then(|e| e.as_str()) {
                match effort {
                    "none" => { /* skip — no thinking requested */ }
                    _ => {
                        if is_opus_4_6 {
                            // Opus 4.6 uses adaptive thinking (budget_tokens is deprecated)
                            body["thinking"] = json!({"type": "adaptive"});
                        } else {
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
        }

        Ok(body)
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

        headers
    }

    fn auth_header_name(&self) -> &str {
        "x-api-key"
    }

    fn format_auth_header(&self, api_key: &str) -> String {
        // Anthropic uses the API key directly, not as Bearer token
        api_key.to_string()
    }
}

/// Reorders input so tool outputs immediately follow their corresponding calls.
///
/// This prevents role-change flushes in the grouping algorithm from separating
/// `tool_use` / `tool_result` pairs. For example, `[call_A, user_msg, output_A]`
/// becomes `[call_A, output_A, user_msg]`.
fn reorder_tool_outputs(input: &[Value]) -> Vec<Value> {
    use std::collections::HashMap;
    use std::collections::HashSet;

    // Map call_id -> original index for every tool output item.
    let mut output_index: HashMap<&str, usize> = HashMap::new();
    for (i, item) in input.iter().enumerate() {
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
        if item_type == "function_call_output" || item_type == "custom_tool_call_output" {
            if let Some(call_id) = item.get("call_id").and_then(Value::as_str) {
                output_index.insert(call_id, i);
            }
        }
    }

    let mut result = Vec::with_capacity(input.len());
    let mut consumed: HashSet<usize> = HashSet::new();

    for (i, item) in input.iter().enumerate() {
        if consumed.contains(&i) {
            continue;
        }

        result.push(item.clone());

        // After each tool call, insert its matching output if it isn't already next.
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
        if item_type == "function_call" || item_type == "custom_tool_call" {
            if let Some(call_id) = item.get("call_id").and_then(Value::as_str) {
                if let Some(&out_idx) = output_index.get(call_id) {
                    if out_idx != i + 1 {
                        result.push(input[out_idx].clone());
                        consumed.insert(out_idx);
                    }
                }
            }
        }
    }

    result
}

/// Converts input ResponseItems to Anthropic messages format.
///
/// System messages are NOT included in the messages array - they
/// should be passed separately via the `system` field.
fn build_anthropic_messages(input: &[Value]) -> Result<Vec<Value>, ApiError> {
    let input = reorder_tool_outputs(input);

    fn flush_current(
        messages: &mut Vec<Value>,
        current_role: &mut Option<&'static str>,
        current_content: &mut Vec<Value>,
        current_user_has_non_tool_results: &mut bool,
    ) {
        if let Some(role) = current_role.take()
            && !current_content.is_empty()
        {
            messages.push(json!({
                "role": role,
                "content": std::mem::take(current_content),
            }));
        } else {
            current_content.clear();
        }
        *current_user_has_non_tool_results = false;
    }

    fn validate_tool_result_sequence(messages: &[Value]) -> Result<(), ApiError> {
        for (idx, message) in messages.iter().enumerate() {
            let role = message.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if role != "assistant" {
                continue;
            }

            let tool_use_ids = message
                .get("content")
                .and_then(Value::as_array)
                .map(|content| {
                    content
                        .iter()
                        .filter(|block| {
                            block.get("type").and_then(Value::as_str) == Some("tool_use")
                        })
                        .filter_map(|block| block.get("id").and_then(Value::as_str))
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            if tool_use_ids.is_empty() {
                continue;
            }

            let next = messages.get(idx + 1).ok_or_else(|| ApiError::InvalidRequest {
                message: format!(
                    "Anthropic tool_use at messages.{idx} missing required tool_result next message"
                ),
            })?;

            if next.get("role").and_then(Value::as_str) != Some("user") {
                return Err(ApiError::InvalidRequest {
                    message: format!(
                        "Anthropic tool_use at messages.{idx} must be followed by a user message containing tool_result blocks"
                    ),
                });
            }

            let mut tool_result_ids = std::collections::HashSet::new();
            let mut saw_non_tool_result = false;
            if let Some(content) = next.get("content").and_then(Value::as_array) {
                for block in content {
                    let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
                    if block_type == "tool_result" {
                        if saw_non_tool_result {
                            return Err(ApiError::InvalidRequest {
                                message: format!(
                                    "Anthropic tool_result blocks must come first in messages.{} content",
                                    idx + 1
                                ),
                            });
                        }
                        if let Some(tool_use_id) = block.get("tool_use_id").and_then(Value::as_str)
                        {
                            tool_result_ids.insert(tool_use_id.to_string());
                        }
                    } else {
                        saw_non_tool_result = true;
                    }
                }
            }

            let missing = tool_use_ids
                .iter()
                .filter(|id| !tool_result_ids.contains(*id))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(ApiError::InvalidRequest {
                    message: format!(
                        "Anthropic tool_use ids were found without tool_result blocks in messages.{}: {}",
                        idx + 1,
                        missing.join(", ")
                    ),
                });
            }
        }

        Ok(())
    }

    let mut messages = Vec::new();
    let mut current_role: Option<&'static str> = None;
    let mut current_content = Vec::new();
    let mut current_user_has_non_tool_results = false;

    for item in input {
        let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");

        let mut role: Option<&'static str> = None;
        let mut blocks = Vec::new();
        let mut contains_tool_result = false;

        match item_type {
            "message" => {
                let original_role = item.get("role").and_then(|r| r.as_str()).unwrap_or("user");

                // Skip system messages - they go in the system field
                if original_role == "system" {
                    continue;
                }

                // Anthropic only allows "user" or "assistant" roles;
                // map "developer" instructions to "user".
                role = Some(if original_role == "assistant" {
                    "assistant"
                } else {
                    "user"
                });

                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for block in content {
                        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match block_type {
                            "input_text" | "output_text" => {
                                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                    blocks.push(json!({
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
                                            blocks.push(json!({
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
                                        blocks.push(json!({
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

                role = Some("assistant");
                blocks.push(json!({
                    "type": "tool_use",
                    "id": call_id,
                    "name": name,
                    "input": input
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

                role = Some("user");
                contains_tool_result = true;
                blocks.push(json!({
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": content
                }));
            }

            "reasoning" => {
                // Convert to Anthropic thinking/redacted_thinking content blocks
                let summary = item.get("summary").and_then(Value::as_array);
                let encrypted_content = item.get("encrypted_content").and_then(Value::as_str);

                let has_summary_text = summary
                    .as_ref()
                    .is_some_and(|arr| arr.iter().any(|s| s.get("text").is_some()));

                role = Some("assistant");
                if has_summary_text {
                    // Normal thinking block
                    let thinking_text = summary
                        .unwrap()
                        .iter()
                        .filter_map(|s| s.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n");

                    let mut block = json!({
                        "type": "thinking",
                        "thinking": thinking_text
                    });
                    if let Some(sig) = encrypted_content {
                        block["signature"] = json!(sig);
                    }
                    blocks.push(block);
                } else if let Some(data) = encrypted_content {
                    // Redacted thinking block
                    blocks.push(json!({
                        "type": "redacted_thinking",
                        "data": data
                    }));
                } else {
                    role = None;
                }
                // If neither, skip (no useful data to pass back)
            }

            "custom_tool_call" => {
                let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let input_str = item.get("input").and_then(|a| a.as_str()).unwrap_or("{}");
                let input: Value = serde_json::from_str(input_str).unwrap_or(json!({}));

                role = Some("assistant");
                blocks.push(json!({
                    "type": "tool_use",
                    "id": call_id,
                    "name": name,
                    "input": input
                }));
            }

            "custom_tool_call_output" => {
                let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                let output = item.get("output").and_then(|o| o.as_str()).unwrap_or("");

                role = Some("user");
                contains_tool_result = true;
                blocks.push(json!({
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": output
                }));
            }

            _ => {
                // Skip unsupported item types
            }
        }

        let Some(role) = role else {
            continue;
        };
        if blocks.is_empty() {
            continue;
        }

        let should_flush = match current_role {
            Some(current) if current != role => true,
            Some("user") if contains_tool_result && current_user_has_non_tool_results => true,
            _ => false,
        };

        if should_flush {
            flush_current(
                &mut messages,
                &mut current_role,
                &mut current_content,
                &mut current_user_has_non_tool_results,
            );
        }

        if current_role.is_none() {
            current_role = Some(role);
        }

        if role == "user" && !contains_tool_result {
            current_user_has_non_tool_results = true;
        }

        current_content.extend(blocks);
    }

    flush_current(
        &mut messages,
        &mut current_role,
        &mut current_content,
        &mut current_user_has_non_tool_results,
    );

    let mut result = messages;

    // Ensure the first message has role "user" as required by the Anthropic API
    if result
        .first()
        .and_then(|m| m.get("role").and_then(|r| r.as_str()))
        != Some("user")
    {
        result.insert(
            0,
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "Continue."}]
            }),
        );
    }

    validate_tool_result_sequence(&result)?;

    Ok(result)
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

    // 128K models — check most specific first
    if model_lower.contains("claude-opus-4-6") {
        128000
    // 64K models
    } else if model_lower.contains("claude-3-7")
        || model_lower.contains("claude-3.7")
        || model_lower.contains("claude-sonnet-4")
        || model_lower.contains("claude-opus-4-5")
        || model_lower.contains("claude-opus-4")
        || model_lower.contains("claude-haiku-4-5")
        || model_lower.contains("claude-haiku-4.5")
    {
        64000
    // 8K models
    } else if model_lower.contains("claude-3.5") || model_lower.contains("claude-3-5") {
        8192
    // 4K models
    } else if model_lower.contains("claude-3-haiku") || model_lower.contains("claude-3-opus") {
        4096
    } else {
        // Safe default for unknown models
        4096
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
        assert!(
            !headers.contains_key("anthropic-beta"),
            "obsolete anthropic-beta header should not be sent"
        );
    }

    #[test]
    fn test_default_max_tokens() {
        // 4K models
        assert_eq!(default_max_tokens("claude-3-haiku-20240307"), 4096);
        assert_eq!(default_max_tokens("claude-3-opus-20240229"), 4096);
        // 8K models
        assert_eq!(default_max_tokens("claude-3.5-sonnet-20240620"), 8192);
        assert_eq!(default_max_tokens("claude-3-5-sonnet-20241022"), 8192);
        // 64K models
        assert_eq!(default_max_tokens("claude-3-7-sonnet-20250219"), 64000);
        assert_eq!(default_max_tokens("claude-sonnet-4-20250514"), 64000);
        assert_eq!(default_max_tokens("claude-opus-4-20250514"), 64000);
        assert_eq!(default_max_tokens("claude-opus-4-5-20251101"), 64000);
        assert_eq!(default_max_tokens("claude-haiku-4-5-20251001"), 64000);
        // 128K models
        assert_eq!(default_max_tokens("claude-opus-4-6-20260101"), 128000);
        // Unknown
        assert_eq!(default_max_tokens("unknown-model"), 4096);
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
    fn test_build_request_body_opus_4_6_adaptive_thinking() {
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
                "claude-opus-4-6-20260101",
                "Be helpful",
                &input,
                &[],
                &options,
            )
            .unwrap();
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert!(
            body["thinking"].get("budget_tokens").is_none(),
            "Opus 4.6 should use adaptive thinking without budget_tokens"
        );
    }

    #[test]
    fn test_build_request_body_opus_4_6_thinking_none() {
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
            .build_request_body("claude-opus-4-6-20260101", "", &input, &[], &options)
            .unwrap();
        assert!(
            body.get("thinking").is_none(),
            "effort=none should not add thinking even for Opus 4.6"
        );
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

        assert_eq!(messages.len(), 3);
        // First message is the synthetic "Continue." prefix
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["text"], "Continue.");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(messages[1]["content"][0]["id"], "toolu_123");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "toolu_123");
    }

    #[test]
    fn test_thinking_block_included_in_messages() {
        let input = vec![
            json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "Think about this"}]
            }),
            json!({
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": "Let me think..."}],
                "encrypted_content": "sig_abc123"
            }),
            json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Here's my answer."}]
            }),
        ];

        let messages = build_anthropic_messages(&input).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        // Thinking block
        assert_eq!(messages[1]["content"][0]["type"], "thinking");
        assert_eq!(messages[1]["content"][0]["thinking"], "Let me think...");
        assert_eq!(messages[1]["content"][0]["signature"], "sig_abc123");
        // Text block (same assistant message)
        assert_eq!(messages[1]["content"][1]["type"], "text");
        assert_eq!(messages[1]["content"][1]["text"], "Here's my answer.");
    }

    #[test]
    fn test_redacted_thinking_block_included() {
        let input = vec![
            json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "Think about this"}]
            }),
            json!({
                "type": "reasoning",
                "summary": [],
                "encrypted_content": "encrypted_data_blob"
            }),
            json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Here's my answer."}]
            }),
        ];

        let messages = build_anthropic_messages(&input).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "redacted_thinking");
        assert_eq!(messages[1]["content"][0]["data"], "encrypted_data_blob");
        assert_eq!(messages[1]["content"][1]["type"], "text");
    }

    #[test]
    fn test_thinking_plus_tool_use_produces_correct_messages() {
        // reasoning + function_call + function_call_output
        let input = vec![
            json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "Do something"}]
            }),
            json!({
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": "I should use the tool"}],
                "encrypted_content": "sig_xyz"
            }),
            json!({
                "type": "function_call",
                "call_id": "toolu_think",
                "name": "run_cmd",
                "arguments": "{\"cmd\": \"ls\"}"
            }),
            json!({
                "type": "function_call_output",
                "call_id": "toolu_think",
                "output": "file1.txt"
            }),
        ];

        let messages = build_anthropic_messages(&input).unwrap();

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");

        // Thinking + tool use blocks are merged into a single assistant message
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "thinking");
        assert_eq!(
            messages[1]["content"][0]["thinking"],
            "I should use the tool"
        );
        assert_eq!(messages[1]["content"][0]["signature"], "sig_xyz");

        assert_eq!(messages[1]["content"][1]["type"], "tool_use");
        assert_eq!(messages[1]["content"][1]["id"], "toolu_think");

        // Tool result
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "toolu_think");
    }

    #[test]
    fn test_multiple_tool_use_blocks_grouped_with_matching_tool_results() {
        let input = vec![
            json!({
                "type": "function_call",
                "call_id": "toolu_a",
                "name": "a",
                "arguments": "{}"
            }),
            json!({
                "type": "function_call",
                "call_id": "toolu_b",
                "name": "b",
                "arguments": "{}"
            }),
            json!({
                "type": "function_call_output",
                "call_id": "toolu_a",
                "output": "out a"
            }),
            json!({
                "type": "function_call_output",
                "call_id": "toolu_b",
                "output": "out b"
            }),
        ];

        let messages = build_anthropic_messages(&input).unwrap();

        // After reordering: [call_A, output_A, call_B, output_B]
        // Produces: Continue + assistant(call_A) + user(output_A) + assistant(call_B) + user(output_B)
        assert_eq!(messages.len(), 5);
        // First message is the synthetic "Continue." prefix
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["text"], "Continue.");

        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(messages[1]["content"][0]["id"], "toolu_a");

        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "toolu_a");

        assert_eq!(messages[3]["role"], "assistant");
        assert_eq!(messages[3]["content"][0]["type"], "tool_use");
        assert_eq!(messages[3]["content"][0]["id"], "toolu_b");

        assert_eq!(messages[4]["role"], "user");
        assert_eq!(messages[4]["content"][0]["type"], "tool_result");
        assert_eq!(messages[4]["content"][0]["tool_use_id"], "toolu_b");
    }

    #[test]
    fn test_custom_tool_call_converted() {
        let input = vec![
            json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "Do it"}]
            }),
            json!({
                "type": "custom_tool_call",
                "call_id": "ct_123",
                "name": "my_tool",
                "input": "{\"key\": \"value\"}"
            }),
            json!({
                "type": "custom_tool_call_output",
                "call_id": "ct_123",
                "output": "result here"
            }),
        ];

        let messages = build_anthropic_messages(&input).unwrap();

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(messages[1]["content"][0]["id"], "ct_123");
        assert_eq!(messages[1]["content"][0]["name"], "my_tool");
        assert_eq!(messages[1]["content"][0]["input"]["key"], "value");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "ct_123");
        assert_eq!(messages[2]["content"][0]["content"], "result here");
    }

    #[test]
    fn test_tool_output_separated_by_user_message() {
        // Reproduces the bug: [call, user_msg, output] should NOT produce
        // an assistant message with tool_use followed by a user message without tool_result
        let input = vec![
            json!({
                "type": "function_call",
                "call_id": "toolu_1",
                "name": "t",
                "arguments": "{}"
            }),
            json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "hello"}]
            }),
            json!({
                "type": "function_call_output",
                "call_id": "toolu_1",
                "output": "result"
            }),
        ];

        let messages = build_anthropic_messages(&input).unwrap();

        // After reordering: [call, output, user_msg]
        // Verify every assistant message with tool_use has matching tool_result in next message
        for (idx, msg) in messages.iter().enumerate() {
            if msg.get("role").and_then(Value::as_str) != Some("assistant") {
                continue;
            }
            let has_tool_use = msg
                .get("content")
                .and_then(Value::as_array)
                .map(|c| {
                    c.iter()
                        .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                })
                .unwrap_or(false);
            if !has_tool_use {
                continue;
            }
            let next = &messages[idx + 1];
            assert_eq!(next.get("role").and_then(Value::as_str), Some("user"));
            let has_tool_result = next
                .get("content")
                .and_then(Value::as_array)
                .map(|c| {
                    c.iter()
                        .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
                })
                .unwrap_or(false);
            assert!(
                has_tool_result,
                "assistant message at index {idx} has tool_use but next message has no tool_result"
            );
        }
    }

    #[test]
    fn test_first_message_not_user_gets_prefix() {
        // If the first item is an assistant message, a user prefix should be added
        let input = vec![json!({
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "I'm here."}]
        })];

        let messages = build_anthropic_messages(&input).unwrap();

        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["type"], "text");
        assert_eq!(messages[0]["content"][0]["text"], "Continue.");
        assert_eq!(messages[1]["role"], "assistant");
    }
}
