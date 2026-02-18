//! OpenAI tool format implementation.

use serde_json::Value;
use serde_json::json;

use super::ToolFormatter;
use crate::error::ApiError;

/// OpenAI tool formatter.
///
/// Tools are already in OpenAI format internally, so this is mostly a pass-through.
pub struct OpenAiToolFormatter;

impl OpenAiToolFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for OpenAiToolFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolFormatter for OpenAiToolFormatter {
    fn format_tools(&self, tools: &[Value]) -> Result<Vec<Value>, ApiError> {
        // Tools are already in OpenAI format (ResponsesApiTool)
        Ok(tools.to_vec())
    }

    fn format_tool_result(
        &self,
        call_id: &str,
        _tool_name: &str,
        result: &str,
        _is_error: bool,
    ) -> Result<Value, ApiError> {
        Ok(json!({
            "type": "function_call_output",
            "call_id": call_id,
            "output": result
        }))
    }
}
