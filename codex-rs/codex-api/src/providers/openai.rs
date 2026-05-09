//! OpenAI provider adapter implementation.
//!
//! This adapter wraps the existing OpenAI Responses API implementation,
//! providing a consistent interface through the `ProviderAdapter` trait.

use serde_json::Value;
use serde_json::json;

use crate::error::ApiError;
use crate::file_support::rewrite_openai_url_file_blocks_in_payload;
use crate::provider_adapter::ProviderAdapter;
use crate::provider_adapter::RequestOptions;

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
        let mut rewritten_input = serde_json::json!(input);
        rewrite_openai_url_file_blocks_in_payload(&mut rewritten_input);

        let mut body = json!({
            "model": model,
            "instructions": instructions,
            "input": rewritten_input,
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

    fn streaming_endpoint(&self, _model: &str) -> String {
        "/responses".to_string()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn build_request_body_passes_input_through_unchanged() {
        let adapter = OpenAiAdapter::new();
        let input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{
                "type": "input_file",
                "file_data": "data:application/pdf;base64,JVBERi0xLjQ=",
                "mime_type": "application/pdf"
            }]
        })];
        let body = adapter
            .build_request_body("gpt-test", "inst", &input, &[], &RequestOptions::default())
            .expect("request body");

        assert_eq!(
            body["input"][0]["content"][0]["file_data"],
            "data:application/pdf;base64,JVBERi0xLjQ="
        );
    }

    #[test]
    fn build_request_body_rewrites_url_file_for_openai_wire_shape() {
        let adapter = OpenAiAdapter::new();
        let input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{
                "type": "url_file",
                "url": "https://example.com/report.pdf",
                "filename": "report.pdf",
                "mime_type": "application/pdf"
            }]
        })];

        let body = adapter
            .build_request_body("gpt-test", "inst", &input, &[], &RequestOptions::default())
            .expect("request body");

        assert_eq!(
            body["input"][0]["content"][0],
            json!({
                "type": "input_file",
                "file_url": "https://example.com/report.pdf"
            })
        );
    }
}
