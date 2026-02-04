//! Gemini provider adapter implementation.
//!
//! This adapter handles the Google Gemini GenerateContent API, converting between
//! the internal ResponseItem format and Gemini's content format.

use serde_json::{json, Value};

use crate::common::ResponseEvent;
use crate::error::ApiError;
use crate::provider_adapter::{ProviderAdapter, RequestOptions};
use crate::sse::gemini::parse_gemini_event;
use crate::tools::GeminiToolFormatter;

/// Google Gemini GenerateContent API adapter.
pub struct GeminiAdapter {
    tool_formatter: GeminiToolFormatter,
}

impl GeminiAdapter {
    pub fn new() -> Self {
        Self {
            tool_formatter: GeminiToolFormatter::new(),
        }
    }

    /// Converts internal ResponseItem format to Gemini contents format.
    fn convert_input_to_contents(&self, input: &[Value]) -> Vec<Value> {
        input
            .iter()
            .filter_map(|item| {
                let item_type = item.get("type")?.as_str()?;
                match item_type {
                    "message" => {
                        let role = item.get("role")?.as_str()?;
                        let content = item.get("content")?;
                        let gemini_role = if role == "assistant" { "model" } else { "user" };

                        let parts: Vec<Value> = content
                            .as_array()?
                            .iter()
                            .filter_map(|c| self.convert_part(c))
                            .collect();

                        if parts.is_empty() {
                            return None;
                        }

                        Some(json!({
                            "role": gemini_role,
                            "parts": parts
                        }))
                    }
                    "function_call" => {
                        let name = item.get("name")?.as_str()?;
                        let arguments = item.get("arguments")?.as_str()?;
                        let args: Value =
                            serde_json::from_str(arguments).unwrap_or_else(|_| json!({}));

                        Some(json!({
                            "role": "model",
                            "parts": [{
                                "functionCall": {
                                    "name": name,
                                    "args": args
                                }
                            }]
                        }))
                    }
                    "function_call_output" => {
                        let output = item.get("output")?;
                        let content_str = output.get("content")?.as_str()?;
                        let response: Value = serde_json::from_str(content_str)
                            .unwrap_or_else(|_| json!({"result": content_str}));

                        // Try to get the function name from the call_id or context
                        // Gemini requires the function name in the response
                        let name = item
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("function");

                        Some(json!({
                            "role": "user",
                            "parts": [{
                                "functionResponse": {
                                    "name": name,
                                    "response": response
                                }
                            }]
                        }))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    fn convert_part(&self, block: &Value) -> Option<Value> {
        let block_type = block.get("type")?.as_str()?;
        match block_type {
            "output_text" | "input_text" => {
                let text = block.get("text")?.as_str()?;
                Some(json!({"text": text}))
            }
            _ => None,
        }
    }
}

impl Default for GeminiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderAdapter for GeminiAdapter {
    fn provider_id(&self) -> &str {
        "gemini"
    }

    fn format_tools(&self, tools: &[Value]) -> Result<Vec<Value>, ApiError> {
        use crate::tools::ToolFormatter;
        self.tool_formatter.format_tools(tools)
    }

    fn build_request_body(
        &self,
        _model: &str,
        instructions: &str,
        input: &[Value],
        tools: &[Value],
        _options: &RequestOptions,
    ) -> Result<Value, ApiError> {
        let contents = self.convert_input_to_contents(input);
        let formatted_tools = self.format_tools(tools)?;

        let mut body = json!({
            "generationConfig": {
                "temperature": 1.0,
                "maxOutputTokens": 8192
            }
        });

        // Add system instruction if provided
        if !instructions.is_empty() {
            body["system_instruction"] = json!({
                "parts": [{"text": instructions}]
            });
        }

        // Add contents (conversation history)
        body["contents"] = json!(contents);

        // Add tools if any
        if !formatted_tools.is_empty() {
            body["tools"] = json!(formatted_tools);
        }

        Ok(body)
    }

    fn parse_sse_event(
        &self,
        _event_type: &str,
        data: &str,
    ) -> Result<Option<ResponseEvent>, ApiError> {
        // Gemini doesn't use standard SSE event types, just JSON lines
        parse_gemini_event(data)
    }

    fn streaming_endpoint(&self, model: &str) -> String {
        // Gemini uses model name in the URL path
        // The API key is added as a query parameter by the caller
        format!("/models/{}:streamGenerateContent", model)
    }

    fn is_completion_event(&self, _event_type: &str) -> bool {
        // Gemini completion is detected from response content (finishReason)
        // not from event type
        false
    }

    fn auth_header_name(&self) -> &str {
        // Gemini uses query parameter auth, not header
        // But we'll return a dummy header name and handle it specially
        "x-goog-api-key"
    }

    fn format_auth_header(&self, api_key: &str) -> String {
        // Gemini typically uses query params, but also supports this header
        api_key.to_string()
    }
}
