//! Anthropic tool format implementation.
//!
//! This module handles tool formatting for Anthropic Claude API,
//! converting from OpenAI's format to Anthropic's expected format.

use serde_json::{json, Value};

use super::ToolFormatter;
use crate::error::ApiError;

/// Anthropic tool formatter.
///
/// Converts tools from OpenAI format `{ type: "function", function: {...} }`
/// to Anthropic's format `{ name, description, input_schema }`.
pub struct AnthropicToolFormatter;

impl AnthropicToolFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AnthropicToolFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolFormatter for AnthropicToolFormatter {
    fn format_tools(&self, tools: &[Value]) -> Result<Vec<Value>, ApiError> {
        let mut anthropic_tools = Vec::new();

        for tool in tools {
            // OpenAI format: { "type": "function", "function": { "name", "description", "parameters" } }
            // Anthropic format: { "name", "description", "input_schema" }
            if let Some(function) = tool.get("function") {
                let name = function
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let description = function
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let input_schema = function
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}}));

                anthropic_tools.push(json!({
                    "name": name,
                    "description": description,
                    "input_schema": input_schema
                }));
            } else if tool.get("type").and_then(|v| v.as_str()) == Some("function") {
                // Handle case where function properties are at root level
                let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let description = tool
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let input_schema = tool
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}}));

                anthropic_tools.push(json!({
                    "name": name,
                    "description": description,
                    "input_schema": input_schema
                }));
            }
        }

        Ok(anthropic_tools)
    }

    fn format_tool_result(
        &self,
        call_id: &str,
        _tool_name: &str,
        result: &str,
        is_error: bool,
    ) -> Result<Value, ApiError> {
        // Anthropic uses tool_result format with tool_use_id
        let mut tool_result = json!({
            "type": "tool_result",
            "tool_use_id": call_id,
            "content": result
        });

        if is_error {
            tool_result["is_error"] = json!(true);
        }

        Ok(tool_result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_format_tools_openai_format() {
        let formatter = AnthropicToolFormatter::new();
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get the current weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    },
                    "required": ["location"]
                }
            }
        })];

        let result = formatter.format_tools(&tools).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["name"], "get_weather");
        assert_eq!(result[0]["description"], "Get the current weather");
        assert_eq!(result[0]["input_schema"]["type"], "object");
        assert!(result[0]["input_schema"]["properties"]["location"].is_object());
    }

    #[test]
    fn test_format_tool_result() {
        let formatter = AnthropicToolFormatter::new();
        let result = formatter
            .format_tool_result("toolu_123", "get_weather", "The weather is sunny", false)
            .unwrap();

        assert_eq!(result["type"], "tool_result");
        assert_eq!(result["tool_use_id"], "toolu_123");
        assert_eq!(result["content"], "The weather is sunny");
        assert!(result.get("is_error").is_none());
    }

    #[test]
    fn test_format_tool_result_with_error() {
        let formatter = AnthropicToolFormatter::new();
        let result = formatter
            .format_tool_result("toolu_123", "get_weather", "API error occurred", true)
            .unwrap();

        assert_eq!(result["type"], "tool_result");
        assert_eq!(result["tool_use_id"], "toolu_123");
        assert_eq!(result["content"], "API error occurred");
        assert_eq!(result["is_error"], true);
    }
}
