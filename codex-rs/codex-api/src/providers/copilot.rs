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

/// Convert a Responses-API `input` array (with optional system `instructions`)
/// into a Chat Completions `messages` array. Only handles the cases we
/// actually need for Copilot: text-only user/assistant messages, tool calls,
/// and tool outputs. Anything unrecognized is dropped.
fn responses_input_to_chat_messages(instructions: &str, input: &[Value]) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::new();
    if !instructions.is_empty() {
        messages.push(json!({"role": "system", "content": instructions}));
    }

    for item in input {
        let item_type = item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message");

        match item_type {
            "message" => {
                let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                let text = extract_message_text(item);
                if text.is_empty() {
                    continue;
                }
                messages.push(json!({"role": role, "content": text}));
            }
            "function_call" => {
                let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
                let name = item.get("name").and_then(Value::as_str).unwrap_or("");
                let arguments = item.get("arguments").and_then(Value::as_str).unwrap_or("");
                messages.push(json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": call_id,
                        "type": "function",
                        "function": {"name": name, "arguments": arguments},
                    }],
                }));
            }
            "function_call_output" => {
                let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
                let output = item.get("output").and_then(Value::as_str).unwrap_or("");
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output,
                }));
            }
            _ => {}
        }
    }

    messages
}

/// Convert Responses-API tool definitions (flat `{type, name, description,
/// parameters}`) into Chat Completions tools (`{type, function: {name,
/// description, parameters}}`). Tools that lack a non-empty `name` are
/// dropped — Copilot rejects requests that include them.
fn responses_tools_to_chat_tools(tools: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for tool in tools {
        // Already nested? pass through if name is non-empty.
        if let Some(func) = tool.get("function").and_then(Value::as_object) {
            let name = func.get("name").and_then(Value::as_str).unwrap_or("");
            if !name.is_empty() {
                out.push(tool.clone());
            }
            continue;
        }

        let kind = tool.get("type").and_then(Value::as_str).unwrap_or("function");
        if kind != "function" {
            continue;
        }
        let name = tool.get("name").and_then(Value::as_str).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let description = tool.get("description").cloned();
        let parameters = tool.get("parameters").cloned().unwrap_or_else(|| json!({}));

        let mut function = serde_json::Map::new();
        function.insert("name".to_string(), json!(name));
        if let Some(desc) = description {
            function.insert("description".to_string(), desc);
        }
        function.insert("parameters".to_string(), parameters);

        out.push(json!({
            "type": "function",
            "function": Value::Object(function),
        }));
    }
    out
}

fn extract_message_text(item: &Value) -> String {
    if let Some(s) = item.get("content").and_then(Value::as_str) {
        return s.to_string();
    }
    let Some(content) = item.get("content").and_then(Value::as_array) else {
        return String::new();
    };
    let mut out = String::new();
    for part in content {
        let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
        if part_type == "input_text" || part_type == "output_text" {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                out.push_str(text);
            }
        }
    }
    out
}

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
        let input_items: Vec<Value> = rewritten_input
            .as_array()
            .cloned()
            .unwrap_or_default();
        let messages = responses_input_to_chat_messages(instructions, &input_items);

        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": true,
        });

        let chat_tools = responses_tools_to_chat_tools(tools);
        if !chat_tools.is_empty() {
            body["tools"] = json!(chat_tools);
            body["tool_choice"] = json!("auto");
            body["parallel_tool_calls"] = json!(options.parallel_tool_calls);
        }

        Ok(body)
    }

    fn streaming_endpoint(&self, _model: &str) -> String {
        "/chat/completions".to_string()
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
    fn streaming_endpoint_is_chat_completions() {
        let adapter = CopilotAdapter::new();
        assert_eq!(adapter.streaming_endpoint("any"), "/chat/completions");
    }

    #[test]
    fn build_request_body_emits_chat_completions_shape() {
        let adapter = CopilotAdapter::new();
        let input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Hello"}]
        })];
        let body = adapter
            .build_request_body("gpt-4o", "be terse", &input, &[], &RequestOptions::default())
            .expect("request body");

        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["stream"], true);
        let messages = body["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "be terse");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "Hello");
        assert!(body.get("input").is_none());
    }

    #[test]
    fn responses_tools_translate_to_chat_tools() {
        let tools = vec![
            json!({
                "type": "function",
                "name": "shell",
                "description": "Run a command",
                "parameters": {"type": "object"}
            }),
            json!({"type": "function", "name": ""}),
            json!({"type": "function"}),
        ];
        let chat = responses_tools_to_chat_tools(&tools);
        assert_eq!(chat.len(), 1);
        assert_eq!(chat[0]["type"], "function");
        assert_eq!(chat[0]["function"]["name"], "shell");
        assert_eq!(chat[0]["function"]["description"], "Run a command");
        assert_eq!(chat[0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn function_call_round_trips_to_tool_call() {
        let input = vec![
            json!({
                "type": "function_call",
                "call_id": "abc",
                "name": "get_weather",
                "arguments": "{\"city\":\"SF\"}"
            }),
            json!({
                "type": "function_call_output",
                "call_id": "abc",
                "output": "sunny"
            }),
        ];
        let messages = responses_input_to_chat_messages("", &input);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["tool_calls"][0]["id"], "abc");
        assert_eq!(messages[0]["tool_calls"][0]["function"]["name"], "get_weather");
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "abc");
        assert_eq!(messages[1]["content"], "sunny");
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
