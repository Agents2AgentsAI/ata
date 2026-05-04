//! GitHub Copilot provider adapter.
//!
//! Authentication is handled out-of-band via OAuth (see
//! `codex_core::auth::copilot_oauth`); the resolved Copilot bearer token is
//! threaded through the standard `ApiAuthProvider` path. This adapter is
//! responsible for the VS Code impersonation headers Copilot requires.

use http::HeaderMap;
use http::HeaderValue;
use serde_json::Value;
use serde_json::json;

use crate::error::ApiError;
use crate::file_support::rewrite_openai_url_file_blocks_in_payload;
use crate::provider_adapter::ProviderAdapter;
use crate::provider_adapter::RequestOptions;

const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
const COPILOT_EDITOR_VERSION: &str = "vscode/1.107.0";
const COPILOT_EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
const COPILOT_INTEGRATION_ID: &str = "vscode-chat";

/// GitHub Copilot adapter.
pub struct CopilotAdapter;

impl CopilotAdapter {
    pub fn new() -> Self {
        Self
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
        "/responses".to_string()
    }

    fn extra_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::USER_AGENT,
            HeaderValue::from_static(COPILOT_USER_AGENT),
        );
        headers.insert(
            "Editor-Version",
            HeaderValue::from_static(COPILOT_EDITOR_VERSION),
        );
        headers.insert(
            "Editor-Plugin-Version",
            HeaderValue::from_static(COPILOT_EDITOR_PLUGIN_VERSION),
        );
        headers.insert(
            "Copilot-Integration-Id",
            HeaderValue::from_static(COPILOT_INTEGRATION_ID),
        );
        headers.insert("Openai-Intent", HeaderValue::from_static("conversation-edits"));
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
    fn streaming_endpoint_is_responses() {
        let adapter = CopilotAdapter::new();
        assert_eq!(adapter.streaming_endpoint("any"), "/responses");
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
    fn extra_headers_include_vscode_impersonation() {
        let adapter = CopilotAdapter::new();
        let headers = adapter.extra_headers();
        assert_eq!(
            headers
                .get(http::header::USER_AGENT)
                .and_then(|v| v.to_str().ok()),
            Some(COPILOT_USER_AGENT)
        );
        assert_eq!(
            headers.get("Editor-Version").and_then(|v| v.to_str().ok()),
            Some(COPILOT_EDITOR_VERSION)
        );
        assert_eq!(
            headers
                .get("Copilot-Integration-Id")
                .and_then(|v| v.to_str().ok()),
            Some(COPILOT_INTEGRATION_ID)
        );
        // Auth header is supplied by the auth provider, not the adapter.
        assert!(headers.get(http::header::AUTHORIZATION).is_none());
    }
}
