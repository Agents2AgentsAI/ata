//! OpenAI provider adapter implementation.
//!
//! This adapter wraps the existing OpenAI Responses API implementation,
//! providing a consistent interface through the `ProviderAdapter` trait.

use serde_json::{json, Value};

use crate::common::ResponseEvent;
use crate::error::ApiError;
use crate::provider_adapter::{ProviderAdapter, RequestOptions};
use crate::sse::responses::{process_responses_event, ResponsesStreamEvent};

/// OpenAI Responses API adapter.
pub struct OpenAiAdapter;

impl OpenAiAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for OpenAiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderAdapter for OpenAiAdapter {
    fn provider_id(&self) -> &str {
        "openai"
    }

    fn format_tools(&self, tools: &[Value]) -> Result<Vec<Value>, ApiError> {
        // Tools are already in OpenAI format
        Ok(tools.to_vec())
    }

    fn build_request_body(
        &self,
        model: &str,
        instructions: &str,
        input: &[Value],
        tools: &[Value],
        options: &RequestOptions,
    ) -> Result<Value, ApiError> {
        let mut body = json!({
            "model": model,
            "instructions": instructions,
            "input": input,
            "tools": tools,
            "tool_choice": "auto",
            "parallel_tool_calls": options.parallel_tool_calls,
            "store": options.store,
            "stream": true,
        });

        if let Some(reasoning) = &options.reasoning {
            body["reasoning"] = reasoning.clone();
        }
        if !options.include.is_empty() {
            body["include"] = json!(options.include);
        }
        if let Some(key) = &options.prompt_cache_key {
            body["prompt_cache_key"] = json!(key);
        }
        if let Some(text) = &options.text_controls {
            body["text"] = text.clone();
        }

        Ok(body)
    }

    fn parse_sse_event(
        &self,
        _event_type: &str,
        data: &str,
    ) -> Result<Option<ResponseEvent>, ApiError> {
        let event: ResponsesStreamEvent = serde_json::from_str(data)
            .map_err(|e| ApiError::Stream(format!("Failed to parse SSE: {}", e)))?;

        match process_responses_event(event) {
            Ok(event) => Ok(event),
            Err(e) => Err(e.into_api_error()),
        }
    }

    fn streaming_endpoint(&self, _model: &str) -> String {
        "/responses".to_string()
    }

    fn is_completion_event(&self, event_type: &str) -> bool {
        matches!(event_type, "response.completed" | "response.done")
    }
}
