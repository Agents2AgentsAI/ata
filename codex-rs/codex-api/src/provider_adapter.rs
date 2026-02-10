//! Provider adapter trait for multi-provider API support.
//!
//! This module defines the `ProviderAdapter` trait which abstracts provider-specific
//! behavior for building requests, formatting tools, and parsing SSE events.

use http::HeaderMap;
use serde_json::Value;

use crate::error::ApiError;

/// Options for building provider-specific API requests.
#[derive(Debug, Clone, Default)]
pub struct RequestOptions {
    /// Reasoning configuration (OpenAI-specific).
    pub reasoning: Option<Value>,
    /// Fields to include in the response.
    pub include: Vec<String>,
    /// Prompt cache key for optimization.
    pub prompt_cache_key: Option<String>,
    /// Text output controls.
    pub text_controls: Option<Value>,
    /// Whether to store the conversation.
    pub store: bool,
    /// Whether parallel tool calls are permitted.
    pub parallel_tool_calls: bool,
}

/// Trait for provider-specific API interactions.
///
/// Each LLM provider (OpenAI, Anthropic, Gemini) implements this trait to handle
/// the differences in their API formats while maintaining a unified internal format.
pub trait ProviderAdapter: Send + Sync {
    /// Returns the provider identifier (e.g., "openai", "anthropic", "gemini").
    fn provider_id(&self) -> &str;

    /// Formats tool specifications from internal format to provider-specific format.
    ///
    /// Takes tools in the internal format (based on OpenAI's format) and converts
    /// them to the provider's expected format.
    fn format_tools(&self, tools: &[Value]) -> Result<Vec<Value>, ApiError>;

    /// Builds the complete request body for the provider's streaming API.
    ///
    /// # Arguments
    /// * `model` - The model identifier
    /// * `instructions` - System instructions/prompt
    /// * `input` - Conversation history as ResponseItem JSON values
    /// * `tools` - Formatted tool specifications
    /// * `options` - Additional request options
    fn build_request_body(
        &self,
        model: &str,
        instructions: &str,
        input: &[Value],
        tools: &[Value],
        options: &RequestOptions,
    ) -> Result<Value, ApiError>;

    /// Returns the API endpoint path for streaming requests.
    fn streaming_endpoint(&self, model: &str) -> String;

    /// Returns provider-specific headers (beyond auth) required for requests.
    fn extra_headers(&self) -> HeaderMap {
        HeaderMap::new()
    }

    /// Returns the authentication header name for this provider.
    /// Defaults to "Authorization" with Bearer token scheme.
    fn auth_header_name(&self) -> &str {
        "Authorization"
    }

    /// Formats the API key for use in the auth header.
    /// Defaults to Bearer token format.
    fn format_auth_header(&self, api_key: &str) -> String {
        format!("Bearer {api_key}")
    }
}
