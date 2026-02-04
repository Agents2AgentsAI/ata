//! Anthropic tool format implementation.
//!
//! Anthropic's Messages API uses a different tool format:
//! ```json
//! {
//!     "name": "tool_name",
//!     "description": "Tool description",
//!     "input_schema": { ... JSON Schema ... }
//! }
//! ```

use serde_json::{json, Value};

use super::ToolFormatter;
use crate::error::ApiError;

/// Anthropic tool formatter.
///
/// Converts OpenAI-style tool definitions to Anthropic's format.
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
        tools
            .iter()
            .filter_map(|tool| {
                // Handle both wrapped (type: "function") and unwrapped formats
                let func = tool
                    .get("function")
                    .or_else(|| {
                        // If no "function" wrapper, check if it's already the function object
                        if tool.get("name").is_some() {
                            Some(tool)
                        } else {
                            None
                        }
                    })
                    .or_else(|| {
                        // Handle local_shell and other special tool types
                        let tool_type = tool.get("type")?.as_str()?;
                        if tool_type == "local_shell" || tool_type == "web_search" {
                            // Skip these - they're not function tools
                            return None;
                        }
                        None
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
                    "input_schema": parameters
                }))
            })
            .collect::<Vec<_>>()
            .pipe(Ok)
    }

    fn format_tool_result(
        &self,
        tool_use_id: &str,
        _tool_name: &str,
        result: &str,
        is_error: bool,
    ) -> Result<Value, ApiError> {
        let mut obj = json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": result
        });

        if is_error {
            obj["is_error"] = json!(true);
        }

        Ok(obj)
    }
}

/// Extension trait for pipe operations.
trait Pipe: Sized {
    fn pipe<T, F: FnOnce(Self) -> T>(self, f: F) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}
