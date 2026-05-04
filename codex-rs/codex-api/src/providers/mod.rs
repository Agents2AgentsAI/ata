//! Provider adapter implementations.
//!
//! This module contains implementations of the `ProviderAdapter` trait for
//! each supported LLM provider.

pub mod anthropic;
pub mod copilot;
pub mod gemini;
pub mod openai;

pub use anthropic::AnthropicAdapter;
pub use copilot::CopilotAdapter;
pub use gemini::GeminiAdapter;
pub use openai::OpenAiAdapter;
