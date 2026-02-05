//! Factory for creating provider adapters based on wire API type.

use std::sync::Arc;

use crate::provider_adapter::ProviderAdapter;
use crate::providers::AnthropicAdapter;
use crate::providers::GeminiAdapter;
use crate::providers::OpenAiAdapter;

/// Wire API variants for provider selection.
///
/// This mirrors the `WireApi` enum in `codex-core` to avoid circular dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireApi {
    /// OpenAI Responses API at `/v1/responses`.
    Responses,
    /// Anthropic Messages API at `/v1/messages`.
    AnthropicMessages,
    /// Google Gemini GenerateContent API.
    GeminiGenerate,
}

/// Factory for creating provider adapters.
pub struct ProviderFactory;

impl ProviderFactory {
    /// Creates the appropriate adapter for the given wire API.
    pub fn create_adapter(wire_api: WireApi) -> Arc<dyn ProviderAdapter> {
        match wire_api {
            WireApi::Responses => Arc::new(OpenAiAdapter::new()),
            WireApi::AnthropicMessages => Arc::new(AnthropicAdapter::new()),
            WireApi::GeminiGenerate => Arc::new(GeminiAdapter::new()),
        }
    }
}
