mod description;
mod response;
mod runtime;
mod service;

// Force a strong reference to the v8 shim crate so its `__hash_memory`
// symbol shows up in every binary that links libv8 on Linux. Without this
// `extern crate`, rustc may drop the rlib if no symbol is referenced from
// our own code, and the linker is left with the unresolved C++ name.
#[cfg(target_os = "linux")]
extern crate codex_v8_shim as _;

pub use description::CODE_MODE_PRAGMA_PREFIX;
pub use description::CodeModeToolKind;
pub use description::ToolDefinition;
pub use description::ToolNamespaceDescription;
pub use description::augment_tool_definition;
pub use description::build_exec_tool_description;
pub use description::build_wait_tool_description;
pub use description::is_code_mode_nested_tool;
pub use description::normalize_code_mode_identifier;
pub use description::parse_exec_source;
pub use description::render_code_mode_sample;
pub use description::render_json_schema_to_typescript;
pub use response::DEFAULT_IMAGE_DETAIL;
pub use response::FunctionCallOutputContentItem;
pub use response::ImageDetail;
pub use runtime::CodeModeNestedToolCall;
pub use runtime::DEFAULT_EXEC_YIELD_TIME_MS;
pub use runtime::DEFAULT_MAX_OUTPUT_TOKENS_PER_EXEC_CALL;
pub use runtime::DEFAULT_WAIT_YIELD_TIME_MS;
pub use runtime::ExecuteRequest;
pub use runtime::RuntimeResponse;
pub use runtime::WaitOutcome;
pub use runtime::WaitRequest;
pub use service::CodeModeService;
pub use service::CodeModeTurnHost;
pub use service::CodeModeTurnWorker;

pub const PUBLIC_TOOL_NAME: &str = "exec";
pub const WAIT_TOOL_NAME: &str = "wait";
