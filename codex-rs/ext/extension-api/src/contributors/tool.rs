use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use codex_tools::ResponsesApiTool;
use serde_json::Value;
use thiserror::Error;

// TMP while we don't have the fully extracted tools
#[derive(Clone)]
pub struct ToolContribution {
    spec: ResponsesApiTool,
    handler: Arc<dyn ToolHandler>,
    supports_parallel_tool_calls: bool,
}

impl ToolContribution {
    pub fn new(spec: ResponsesApiTool, handler: Arc<dyn ToolHandler>) -> Self {
        Self {
            spec,
            handler,
            supports_parallel_tool_calls: false,
        }
    }

    #[must_use]
    pub fn allow_parallel_calls(mut self) -> Self {
        self.supports_parallel_tool_calls = true;
        self
    }

    pub fn spec(&self) -> &ResponsesApiTool {
        &self.spec
    }

    pub fn supports_parallel_tool_calls(&self) -> bool {
        self.supports_parallel_tool_calls
    }

    pub fn handler(&self) -> Arc<dyn ToolHandler> {
        Arc::clone(&self.handler)
    }
}

//////// Just to make it compile ////////////////////////////////
pub trait ToolHandler: Send + Sync {
    /// Handles one JSON-encoded invocation for this tool.
    fn handle<'a>(
        &'a self,
        arguments: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ToolCallError>> + Send + 'a>>;
}

/// Error returned by a contributed native tool handler.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct ToolCallError {
    message: String,
}

impl ToolCallError {
    /// Creates a contributed-tool error with the supplied model-visible text.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
