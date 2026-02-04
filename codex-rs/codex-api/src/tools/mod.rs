//! Tool format translation for different LLM providers.
//!
//! Each provider has a different format for defining function/tool specifications.
//! This module provides formatters that convert from the internal format (based on
//! OpenAI's format) to each provider's expected format.

pub mod anthropic;
pub mod gemini;
pub mod openai;

pub use anthropic::AnthropicToolFormatter;
pub use gemini::GeminiToolFormatter;
pub use openai::OpenAiToolFormatter;

use serde_json::Value;

use crate::error::ApiError;

/// Trait for converting tool specifications to provider-specific formats.
pub trait ToolFormatter: Send + Sync {
    /// Converts internal tool specs to provider-specific format.
    fn format_tools(&self, tools: &[Value]) -> Result<Vec<Value>, ApiError>;

    /// Converts a single tool call result to provider format.
    fn format_tool_result(
        &self,
        call_id: &str,
        tool_name: &str,
        result: &str,
        is_error: bool,
    ) -> Result<Value, ApiError>;
}
