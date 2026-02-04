//! Gemini tool format implementation.
//!
//! Gemini's GenerateContent API uses a different tool format:
//! ```json
//! {
//!     "functionDeclarations": [
//!         {
//!             "name": "tool_name",
//!             "description": "Tool description",
//!             "parameters": { ... JSON Schema ... }
//!         }
//!     ]
//! }
//! ```

use serde_json::{json, Value};

use super::ToolFormatter;
use crate::error::ApiError;

/// Gemini tool formatter.
///
/// Converts OpenAI-style tool definitions to Gemini's functionDeclarations format.
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
        let declarations: Vec<Value> = tools
            .iter()
            .filter_map(|tool| {
                // Handle both wrapped (type: "function") and unwrapped formats
                let func = tool.get("function").or_else(|| {
                    // If no "function" wrapper, check if it's already the function object
                    if tool.get("name").is_some() {
                        Some(tool)
                    } else {
                        // Handle special tool types
                        let tool_type = tool.get("type")?.as_str()?;
                        if tool_type == "local_shell" || tool_type == "web_search" {
                            // Skip these - they're not function tools
                            return None;
                        }
                        None
                    }
                })?;

                let name = func.get("name")?;
                let description = func.get("description").cloned().unwrap_or(json!(""));
                let parameters = func.get("parameters").cloned().unwrap_or(json!({
                    "type": "object",
                    "properties": {}
                }));

                Some(json!({
                    "name": name,
                    "description": description,
                    "parameters": parameters
                }))
            })
            .collect();

        // Return the tools wrapper object
        if declarations.is_empty() {
            Ok(vec![])
        } else {
            Ok(vec![json!({
                "functionDeclarations": declarations
            })])
        }
    }

    fn format_tool_result(
        &self,
        _call_id: &str,
        tool_name: &str,
        result: &str,
        _is_error: bool,
    ) -> Result<Value, ApiError> {
        // Gemini expects function responses as JSON objects
        let response_value: Value =
            serde_json::from_str(result).unwrap_or_else(|_| json!({"result": result}));

        Ok(json!({
            "functionResponse": {
                "name": tool_name,
                "response": response_value
            }
        }))
    }
}
