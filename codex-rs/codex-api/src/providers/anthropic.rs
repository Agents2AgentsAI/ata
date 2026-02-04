//! Anthropic provider adapter implementation.
//!
//! This adapter handles the Anthropic Messages API, converting between the
//! internal ResponseItem format and Anthropic's message format.

use http::{HeaderMap, HeaderValue};
use serde_json::{json, Value};

use crate::common::ResponseEvent;
use crate::error::ApiError;
use crate::provider_adapter::{ProviderAdapter, RequestOptions};
use crate::sse::anthropic::parse_anthropic_event;
use crate::tools::AnthropicToolFormatter;

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

    /// Converts internal ResponseItem format to Anthropic messages format.
    fn convert_input_to_messages(&self, input: &[Value]) -> Vec<Value> {
        input
            .iter()
            .filter_map(|item| {
                let item_type = item.get("type")?.as_str()?;
                match item_type {
                    "message" => {
                        let role = item.get("role")?.as_str()?;
                        let content = item.get("content")?;
                        let anthropic_role = if role == "assistant" {
                            "assistant"
                        } else {
                            "user"
                        };

                        let anthropic_content: Vec<Value> = content
                            .as_array()?
                            .iter()
                            .filter_map(|c| self.convert_content_block(c))
                            .collect();

                        if anthropic_content.is_empty() {
                            return None;
                        }

                        Some(json!({
                            "role": anthropic_role,
                            "content": anthropic_content
                        }))
                    }
                    "function_call" => {
                        let name = item.get("name")?.as_str()?;
                        let arguments = item.get("arguments")?.as_str()?;
                        let call_id = item.get("call_id")?.as_str()?;
                        let input: Value =
                            serde_json::from_str(arguments).unwrap_or_else(|_| json!({}));

                        Some(json!({
                            "role": "assistant",
                            "content": [{
                                "type": "tool_use",
                                "id": call_id,
                                "name": name,
                                "input": input
                            }]
                        }))
                    }
                    "function_call_output" => {
                        let call_id = item.get("call_id")?.as_str()?;
                        let output = item.get("output")?;
                        let content = output.get("content")?.as_str()?;

                        Some(json!({
                            "role": "user",
                            "content": [{
                                "type": "tool_result",
                                "tool_use_id": call_id,
                                "content": content
                            }]
                        }))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    fn convert_content_block(&self, block: &Value) -> Option<Value> {
        let block_type = block.get("type")?.as_str()?;
        match block_type {
            "output_text" | "input_text" => {
                let text = block.get("text")?.as_str()?;
                Some(json!({
                    "type": "text",
                    "text": text
                }))
            }
            _ => None,
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
        use crate::tools::ToolFormatter;
        self.tool_formatter.format_tools(tools)
    }

    fn build_request_body(
        &self,
        model: &str,
        instructions: &str,
        input: &[Value],
        tools: &[Value],
        _options: &RequestOptions,
    ) -> Result<Value, ApiError> {
        let messages = self.convert_input_to_messages(input);
        let formatted_tools = self.format_tools(tools)?;

        let mut body = json!({
            "model": model,
            "max_tokens": 8192,
            "stream": true
        });

        // Add system prompt if provided
        if !instructions.is_empty() {
            body["system"] = json!(instructions);
        }

        // Add messages
        body["messages"] = json!(messages);

        // Add tools if any
        if !formatted_tools.is_empty() {
            body["tools"] = json!(formatted_tools);
        }

        Ok(body)
    }

    fn parse_sse_event(
        &self,
        event_type: &str,
        data: &str,
    ) -> Result<Option<ResponseEvent>, ApiError> {
        parse_anthropic_event(event_type, data)
    }

    fn streaming_endpoint(&self, _model: &str) -> String {
        "/messages".to_string()
    }

    fn extra_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static("2023-06-01"),
        );
        headers
    }

    fn is_completion_event(&self, event_type: &str) -> bool {
        event_type == "message_stop"
    }

    fn auth_header_name(&self) -> &str {
        "x-api-key"
    }

    fn format_auth_header(&self, api_key: &str) -> String {
        // Anthropic uses plain API key, not Bearer token
        api_key.to_string()
    }
}
