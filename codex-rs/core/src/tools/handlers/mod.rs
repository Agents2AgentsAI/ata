pub(crate) mod agent_jobs;
pub mod apply_patch;
pub(crate) mod attach_url_files;
#[cfg(feature = "treesitter")]
pub(crate) mod code_intel;
#[cfg(feature = "data")]
pub(crate) mod data;
pub(crate) mod document_reader;
mod dynamic;
mod grep_files;
mod js_repl;
mod list_dir;
#[cfg(feature = "lsp")]
pub(crate) mod lsp;
#[cfg(feature = "lsp")]
pub(crate) mod lsp_workspace_edit;
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

use codex_utils_absolute_path::AbsolutePathBufGuard;
pub use plan::PLAN_TOOL;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;

use crate::function_tool::FunctionCallError;
use crate::sandboxing::SandboxPermissions;
use crate::sandboxing::normalize_additional_permissions;
pub use apply_patch::ApplyPatchHandler;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AskForApproval;
#[cfg(feature = "data")]
pub(crate) use data::DataBridgeHandler;
pub use document_reader::APPEND_TO_SECTION_TOOL;
pub use document_reader::DocumentCache;
pub use document_reader::DocumentReaderHandler;
pub use document_reader::PATCH_DOCUMENT_SECTION_TOOL;
pub use document_reader::PRESENT_DOCUMENT_TOOL;
pub use document_reader::UPDATE_DOCUMENT_SECTION_TOOL;
pub use dynamic::DynamicToolHandler;
pub use grep_files::GrepFilesHandler;
pub use js_repl::JsReplHandler;
pub use js_repl::JsReplResetHandler;
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

#[cfg(any(feature = "lsp", feature = "treesitter"))]
pub(super) fn function_arguments_from_payload(
    payload: crate::tools::context::ToolPayload,
    handler_name: &str,
) -> Result<String, FunctionCallError> {
    match payload {
        crate::tools::context::ToolPayload::Function { arguments } => Ok(arguments),
        _ => Err(FunctionCallError::RespondToModel(format!(
            "{handler_name} handler received unsupported payload"
        ))),
    }
}

#[cfg(any(feature = "lsp", feature = "treesitter"))]
pub(super) fn truncate_tool_output(output: &str, max_bytes: usize) -> (String, bool) {
    if output.len() <= max_bytes {
        return (output.to_string(), false);
    }

    let prefix = codex_utils_string::take_bytes_at_char_boundary(output, max_bytes);
    if prefix.is_empty() {
        return ("... truncated".to_string(), true);
    }

    let cut = prefix.rfind('\n').unwrap_or(prefix.len());
    (format!("{}\n\n... truncated", &prefix[..cut]), true)
}

#[cfg(any(feature = "lsp", feature = "treesitter"))]
pub(super) const HANDLER_DEFAULT_LIMIT: usize = 10;
#[cfg(any(feature = "lsp", feature = "treesitter"))]
pub(super) const HANDLER_MAX_RESULTS: usize = 50;
#[cfg(any(feature = "lsp", feature = "treesitter"))]
pub(super) const HANDLER_MAX_RESULT_BYTES: usize = 8 * 1024;

/// Rejects relative paths outright instead of silently resolving them against
/// `cwd`. Use this for tool parameters where the model should always provide
/// an absolute path.
#[cfg(any(feature = "lsp", feature = "treesitter"))]
pub(super) fn require_absolute_path_argument(
    path_value: Option<&str>,
    key: &str,
) -> Result<PathBuf, FunctionCallError> {
    let raw = path_value
        .ok_or_else(|| FunctionCallError::RespondToModel(format!("`{key}` is required")))?;
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(FunctionCallError::RespondToModel(format!(
            "`{key}` must be an absolute path, got: {raw:?}"
        )));
    }
    Ok(path)
}

fn parse_arguments_with_base_path<T>(
    arguments: &str,
    base_path: &Path,
) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    let _guard = AbsolutePathBufGuard::new(base_path);
    parse_arguments(arguments)
}

fn resolve_workdir_base_path(
    arguments: &str,
    default_cwd: &Path,
) -> Result<PathBuf, FunctionCallError> {
    let arguments: Value = parse_arguments(arguments)?;
    Ok(arguments
        .get("workdir")
        .and_then(Value::as_str)
        .filter(|workdir| !workdir.is_empty())
        .map(PathBuf::from)
        .map_or_else(
            || default_cwd.to_path_buf(),
            |workdir| crate::util::resolve_path(default_cwd, &workdir),
        ))
}

/// Validates feature/policy constraints for `with_additional_permissions` and
/// returns normalized absolute paths. Errors if paths are invalid.
pub(super) fn normalize_and_validate_additional_permissions(
    request_permission_enabled: bool,
    approval_policy: AskForApproval,
    sandbox_permissions: SandboxPermissions,
    additional_permissions: Option<PermissionProfile>,
    _cwd: &Path,
) -> Result<Option<PermissionProfile>, String> {
    let uses_additional_permissions = matches!(
        sandbox_permissions,
        SandboxPermissions::WithAdditionalPermissions
    );

    if !request_permission_enabled
        && (uses_additional_permissions || additional_permissions.is_some())
    {
        return Err(
            "additional permissions are disabled; enable `features.request_permission` before using `with_additional_permissions`"
                .to_string(),
        );
    }

    if uses_additional_permissions {
        if !matches!(approval_policy, AskForApproval::OnRequest) {
            return Err(format!(
                "approval policy is {approval_policy:?}; reject command — you cannot request additional permissions unless the approval policy is OnRequest"
            ));
        }
        let Some(additional_permissions) = additional_permissions else {
            return Err(
                "missing `additional_permissions`; provide `file_system.read` and/or `file_system.write` when using `with_additional_permissions`"
                    .to_string(),
            );
        };
        let normalized = normalize_additional_permissions(additional_permissions)?;
        if normalized.is_empty() {
            return Err(
                "`additional_permissions` must include at least one path in `file_system.read` or `file_system.write`"
                    .to_string(),
            );
        }
        return Ok(Some(normalized));
    }

    if additional_permissions.is_some() {
        Err(
            "`additional_permissions` requires `sandbox_permissions` set to `with_additional_permissions`"
                .to_string(),
        )
    } else {
        Ok(None)
    }
}
