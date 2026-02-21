pub mod apply_patch;
pub(crate) mod attach_url_files;
#[cfg(feature = "data")]
pub(crate) mod data;
mod document_reader;
mod dynamic;
mod grep_files;
mod js_repl;
#[cfg(feature = "kb")]
pub(crate) mod kb;
mod list_dir;
mod mcp;
mod mcp_resource;
pub(crate) mod multi_agents;
mod plan;
mod read_file;
mod request_user_input;
pub(crate) mod research;
mod search_tool_bm25;
mod shell;
mod test_sync;
pub(crate) mod unified_exec;
mod view_image;

pub use document_reader::APPEND_TO_SECTION_TOOL;
pub use document_reader::DocumentCache;
pub use document_reader::DocumentReaderHandler;
pub use document_reader::PATCH_DOCUMENT_SECTION_TOOL;
pub use document_reader::PRESENT_DOCUMENT_TOOL;
pub use document_reader::UPDATE_DOCUMENT_SECTION_TOOL;
pub use plan::PLAN_TOOL;
use serde::Deserialize;

use crate::function_tool::FunctionCallError;
pub use apply_patch::ApplyPatchHandler;
#[cfg(feature = "data")]
pub(crate) use data::DataBridgeHandler;
pub use dynamic::DynamicToolHandler;
pub use grep_files::GrepFilesHandler;
pub use js_repl::JsReplHandler;
pub use js_repl::JsReplResetHandler;
#[cfg(feature = "kb")]
pub(crate) use kb::KbBridgeHandler;
pub use list_dir::ListDirHandler;
pub use mcp::McpHandler;
pub use mcp_resource::McpResourceHandler;
pub use multi_agents::MultiAgentHandler;
pub use plan::PlanHandler;
pub use read_file::ReadFileHandler;
pub use request_user_input::RequestUserInputHandler;
pub(crate) use request_user_input::request_user_input_tool_description;
pub(crate) use research::ResearchBridgeHandler;
pub(crate) use search_tool_bm25::DEFAULT_LIMIT as SEARCH_TOOL_BM25_DEFAULT_LIMIT;
pub(crate) use search_tool_bm25::SEARCH_TOOL_BM25_TOOL_NAME;
pub use search_tool_bm25::SearchToolBm25Handler;
pub use shell::ShellCommandHandler;
pub use shell::ShellHandler;
pub use test_sync::TestSyncHandler;
pub use unified_exec::UnifiedExecHandler;
pub use view_image::ViewImageHandler;

fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}
