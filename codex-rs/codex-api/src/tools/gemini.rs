//! Gemini tool format implementation.
//!
//! This module handles tool formatting for Google Gemini API, including
//! the critical transformation of removing `additionalProperties` and `$schema`
//! from JSON schemas, which Gemini does not support.

use serde_json::Value;
use serde_json::json;

use super::ToolFormatter;
use crate::error::ApiError;

/// Gemini tool formatter.
///
/// Converts tools from OpenAI format to Gemini's `functionDeclarations` format
/// and removes unsupported JSON Schema properties.
pub struct GeminiToolFormatter;

impl GeminiToolFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GeminiToolFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolFormatter for GeminiToolFormatter {
    fn format_tools(&self, tools: &[Value]) -> Result<Vec<Value>, ApiError> {
        let mut function_declarations = Vec::new();

        for tool in tools {
            // OpenAI format: { "type": "function", "function": { "name", "description", "parameters" } }
            // Gemini format: { "name", "description", "parameters" }
            if let Some(function) = tool.get("function") {
                let name = function.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let description = function
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let parameters = function
                    .get("parameters")
                    .cloned()
                    .map(|p| process_schema_for_gemini(&p))
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}}));

                function_declarations.push(json!({
                    "name": name,
                    "description": description,
                    "parameters": parameters
                }));
            } else if tool.get("type").and_then(|v| v.as_str()) == Some("function") {
                // Handle case where function properties are at root level
                let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let description = tool
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let parameters = tool
                    .get("parameters")
                    .cloned()
                    .map(|p| process_schema_for_gemini(&p))
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}}));

                function_declarations.push(json!({
                    "name": name,
                    "description": description,
                    "parameters": parameters
                }));
            }
        }

        Ok(function_declarations)
    }

    fn format_tool_result(
        &self,
        _call_id: &str,
        tool_name: &str,
        result: &str,
        _is_error: bool,
    ) -> Result<Value, ApiError> {
        // Gemini uses functionResponse format
        // Note: Gemini doesn't use call_id in the same way as OpenAI
        let content = match serde_json::from_str::<Value>(result) {
            Ok(json_result) => json_result,
            Err(_) => json!({ "result": result }),
        };

        Ok(json!({
            "functionResponse": {
                "name": tool_name,
                "response": content
            }
        }))
    }
}

/// Recursively removes `additionalProperties` and `$schema` from JSON schemas.
///
/// Gemini API does not support these JSON Schema properties and will reject
/// requests that include them. This function traverses the entire schema
/// and removes these fields at all levels.
///
/// Based on TensorZero's `process_jsonschema_for_gcp_vertex_gemini()`.
pub fn process_schema_for_gemini(schema: &Value) -> Value {
    let mut schema = schema.clone();
    remove_unsupported_properties(&mut schema);
    schema
}

fn remove_unsupported_properties(value: &mut Value) {
    match value {
        Value::Object(obj) => {
            // Remove unsupported properties
            obj.remove("additionalProperties");
            obj.remove("$schema");

            // Recursively process all nested values
            for (_, v) in obj.iter_mut() {
                remove_unsupported_properties(v);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                remove_unsupported_properties(v);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_process_schema_removes_additional_properties() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "nested": {
                    "type": "object",
                    "properties": {
                        "value": {"type": "number"}
                    },
                    "additionalProperties": false
                }
            },
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        });

        let processed = process_schema_for_gemini(&schema);

        assert!(processed.get("additionalProperties").is_none());
        assert!(processed.get("$schema").is_none());
        assert!(
            processed
                .get("properties")
                .and_then(|p| p.get("nested"))
                .and_then(|n| n.get("additionalProperties"))
                .is_none()
        );
    }

    #[test]
    fn test_format_tools_openai_format() {
        let formatter = GeminiToolFormatter::new();
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
                    "required": ["location"],
                    "additionalProperties": false
                }
            }
        })];

        let result = formatter.format_tools(&tools).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["name"], "get_weather");
        assert_eq!(result[0]["description"], "Get the current weather");
        assert!(result[0]["parameters"]["additionalProperties"].is_null());
    }

    #[test]
    fn test_format_tool_result() {
        let formatter = GeminiToolFormatter::new();
        let result = formatter
            .format_tool_result("call_123", "get_weather", r#"{"temperature": 72}"#, false)
            .unwrap();

        assert_eq!(result["functionResponse"]["name"], "get_weather");
        assert_eq!(result["functionResponse"]["response"]["temperature"], 72);
    }

    #[test]
    fn test_format_tool_result_plain_string() {
        let formatter = GeminiToolFormatter::new();
        let result = formatter
            .format_tool_result("call_123", "echo", "Hello, World!", false)
            .unwrap();

        assert_eq!(result["functionResponse"]["name"], "echo");
        assert_eq!(
            result["functionResponse"]["response"]["result"],
            "Hello, World!"
        );
    }
}
