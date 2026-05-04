//! GitHub Copilot provider adapter implementation.
//!
//! Authenticates via a GitHub Personal Access Token read from the
//! `GITHUB_COPILOT_TOKEN` environment variable and targets the Copilot
//! inline completions endpoint. The wire format is identical to the OpenAI
//! Responses API, so request/response handling is the same.

use http::HeaderMap;
use http::HeaderValue;
use serde_json::Value;
use serde_json::json;

use crate::error::ApiError;
use crate::file_support::rewrite_openai_url_file_blocks_in_payload;
use crate::provider_adapter::ProviderAdapter;
use crate::provider_adapter::RequestOptions;

/// GitHub Copilot inline completions adapter.
///
/// Reads the PAT from `GITHUB_COPILOT_TOKEN` at construction time and injects
/// it as an `Authorization: Bearer <token>` header on every request.
pub struct CopilotAdapter {
    token: Option<String>,
}

impl CopilotAdapter {
    pub fn new() -> Self {
        Self {
            token: std::env::var("GITHUB_COPILOT_TOKEN").ok(),
        }
    }
}

impl Default for CopilotAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderAdapter for CopilotAdapter {
    fn provider_id(&self) -> &str {
        "copilot"
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
        "/copilot/inline".to_string()
    }

    fn extra_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(token) = &self.token {
            if let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}")) {
                headers.insert(http::header::AUTHORIZATION, value);
            }
        }
        headers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_is_copilot() {
        let adapter = CopilotAdapter::new();
        assert_eq!(adapter.provider_id(), "copilot");
    }

    #[test]
    fn streaming_endpoint_is_copilot_inline() {
        let adapter = CopilotAdapter::new();
        assert_eq!(adapter.streaming_endpoint("cushman"), "/copilot/inline");
        assert_eq!(adapter.streaming_endpoint("codex"), "/copilot/inline");
        assert_eq!(
            adapter.streaming_endpoint("copilot-codex"),
            "/copilot/inline"
        );
    }

    #[test]
    fn build_request_body_includes_required_fields() {
        let adapter = CopilotAdapter::new();
        let input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Hello"}]
        })];
        let body = adapter
            .build_request_body("cushman", "inst", &input, &[], &RequestOptions::default())
            .expect("request body");

        assert_eq!(body["model"], "cushman");
        assert_eq!(body["stream"], true);
        assert!(body.get("input").is_some());
    }

    #[test]
    fn extra_headers_empty_when_no_token() {
        // Ensure that constructing without the env var yields no auth header.
        // We can only verify the no-token path without mutating process env.
        let adapter = CopilotAdapter { token: None };
        assert!(adapter.extra_headers().is_empty());
    }

    #[test]
    fn extra_headers_bearer_when_token_present() {
        let adapter = CopilotAdapter {
            token: Some("ghp_testtoken".to_string()),
        };
        let headers = adapter.extra_headers();
        let auth = headers
            .get(http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .expect("Authorization header");
        assert_eq!(auth, "Bearer ghp_testtoken");
    }
}
