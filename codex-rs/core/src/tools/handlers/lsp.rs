//! LSP tool handler exposing code intelligence and preview refactor operations to the agent.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Write;
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use codex_lsp_client::ServerRegistry;
use codex_lsp_client::lsp_types::CallHierarchyItem;
use codex_lsp_client::lsp_types::CodeAction;
use codex_lsp_client::lsp_types::CodeActionKind;
use codex_lsp_client::lsp_types::CodeActionOrCommand;
use codex_lsp_client::lsp_types::Diagnostic;
use codex_lsp_client::lsp_types::DocumentSymbol;
use codex_lsp_client::lsp_types::DocumentSymbolResponse;
use codex_lsp_client::lsp_types::GotoDefinitionResponse;
use codex_lsp_client::lsp_types::Hover;
use codex_lsp_client::lsp_types::HoverContents;
use codex_lsp_client::lsp_types::Location;
use codex_lsp_client::lsp_types::MarkedString;
use codex_lsp_client::lsp_types::PrepareRenameResponse;
use codex_lsp_client::lsp_types::Range;
use codex_lsp_client::lsp_types::SymbolInformation;
use codex_lsp_client::lsp_types::request::GotoImplementationResponse;
use codex_protocol::models::FunctionCallOutputBody;
use serde::Deserialize;

use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::function_tool::FunctionCallError;
use crate::state::MultiRootState;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::handlers::HANDLER_DEFAULT_LIMIT;
use crate::tools::handlers::HANDLER_MAX_RESULT_BYTES;
use crate::tools::handlers::HANDLER_MAX_RESULTS;
use crate::tools::handlers::function_arguments_from_payload;
use crate::tools::handlers::lsp_workspace_edit::PatchLimits;
use crate::tools::handlers::lsp_workspace_edit::workspace_edit_to_apply_patch;
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::require_absolute_path_argument;
use crate::tools::handlers::truncate_tool_output;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::spec::JsonSchema;

mod code_actions;
mod formatting;
mod fuzz;
mod symbol_lookup;

use code_actions::choose_code_action;
use code_actions::filter_previewable_code_actions;
use formatting::format_definition;
use formatting::format_document_symbols;
use formatting::format_hover;
use formatting::format_implementation;
use formatting::format_prepare_rename;
use formatting::format_references;
use formatting::format_workspace_symbols;
use fuzz::fuzz_query;
use fuzz::is_empty_definition;
use fuzz::resolved_at_prefix;
use symbol_lookup::collect_document_symbol_candidates;
use symbol_lookup::symbol_name_matches;

const LSP_TOOL_DESCRIPTION: &str = include_str!("tool_lsp.txt");
const MAX_PATCH_BYTES: usize = 256 * 1024;
const FUZZ_ATTEMPTS_SHORT: usize = 12;
const FUZZ_ATTEMPTS_LONG: usize = 25;

/// Handler for the `lsp` tool.
pub struct LspToolHandler {
    pub state: Arc<MultiRootState>,
    /// Tracks files that have been synced at least once (with a diagnostic wait)
    /// so that subsequent queries skip the readiness delay.
    pub warmed_files: tokio::sync::Mutex<HashSet<PathBuf>>,
    /// Tracks workspace roots that have been warmed for workspace-wide queries.
    /// The `usize` value is the `Arc::as_ptr` identity of the `ServerRegistry`
    /// so that a removeRoot + addRoot cycle (which creates a new `Arc`) causes
    /// re-warmup even if the root name is identical.
    pub warmed_workspaces: tokio::sync::Mutex<HashMap<String, usize>>,
}

struct FileQueryContext {
    path: PathBuf,
    registry: Arc<ServerRegistry>,
}

struct PositionedQueryContext {
    path: PathBuf,
    registry: Arc<ServerRegistry>,
    line: u32,
    character: u32,
}

#[derive(Clone, Copy)]
enum CallHierarchyDirection {
    Incoming,
    Outgoing,
}

impl CallHierarchyDirection {
    fn operation_name(self) -> &'static str {
        match self {
            Self::Incoming => "incomingCalls",
            Self::Outgoing => "outgoingCalls",
        }
    }
}

#[derive(Deserialize)]
struct LspToolArgs {
    operation: LspOperation,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    character: Option<u32>,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    fuzz: Option<bool>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    end_line: Option<u32>,
    #[serde(default)]
    end_character: Option<u32>,
    #[serde(default)]
    new_name: Option<String>,
    #[serde(default)]
    only: Option<String>,
    #[serde(default)]
    title: Option<String>,
    /// Serialized CallHierarchyItem for incoming/outgoing calls.
    #[serde(default)]
    item: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum LspOperation {
    GoToDefinition,
    FindReferences,
    Hover,
    DocumentSymbol,
    WorkspaceSymbol,
    GoToImplementation,
    PrepareCallHierarchy,
    IncomingCalls,
    OutgoingCalls,
    PrepareRename,
    RenamePreview,
    CodeActionPreview,
    Diagnostics,
}

#[async_trait]
impl ToolHandler for LspToolHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        false
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let turn_cwd = invocation.turn.cwd.clone();
        let payload = invocation.payload;

        let arguments = function_arguments_from_payload(payload, "lsp")?;
        let args: LspToolArgs = parse_arguments(&arguments)?;
        let limit = args
            .limit
            .unwrap_or(HANDLER_DEFAULT_LIMIT)
            .clamp(1, HANDLER_MAX_RESULTS);
        let fuzz = args.fuzz.unwrap_or(true);

        let (result, is_patch) = match args.operation {
            LspOperation::GoToDefinition => {
                let context = self
                    .prepare_position_query_context(&args, turn_cwd.as_path())
                    .await?;
                let (resp, resolved) = fuzz_query(
                    fuzz,
                    context.line,
                    context.character,
                    FUZZ_ATTEMPTS_LONG,
                    |line, character| context.registry.definition(&context.path, line, character),
                    is_empty_definition,
                )
                .await;

                let mut out = resolved_at_prefix(&context.path, resolved);
                out.push_str(&format_definition(resp));
                (out, false)
            }
            LspOperation::FindReferences => {
                let context = self
                    .prepare_position_query_context(&args, turn_cwd.as_path())
                    .await?;
                let (refs, resolved) = fuzz_query(
                    fuzz,
                    context.line,
                    context.character,
                    FUZZ_ATTEMPTS_SHORT,
                    |line, character| context.registry.references(&context.path, line, character),
                    |refs: &Vec<Location>| refs.is_empty(),
                )
                .await;

                let mut out = resolved_at_prefix(&context.path, resolved);
                out.push_str(&format_references(&refs, limit));
                (out, false)
            }
            LspOperation::Hover => {
                let context = self
                    .prepare_position_query_context(&args, turn_cwd.as_path())
                    .await?;
                let (hover, resolved) = fuzz_query(
                    fuzz,
                    context.line,
                    context.character,
                    FUZZ_ATTEMPTS_LONG,
                    |line, character| context.registry.hover(&context.path, line, character),
                    |hover: &Option<Hover>| hover.is_none(),
                )
                .await;

                let mut out = resolved_at_prefix(&context.path, resolved);
                out.push_str(&format_hover(hover));
                (out, false)
            }
            LspOperation::DocumentSymbol => {
                let context = self
                    .prepare_file_query_context(&args, turn_cwd.as_path())
                    .await?;
                let resp = context.registry.document_symbol(&context.path).await;
                if resp.is_none() {
                    tracing::debug!(
                        path = %context.path.display(),
                        "documentSymbol returned None (server may still be initializing or file not yet analyzed)"
                    );
                }
                (format_document_symbols(resp), false)
            }
            LspOperation::WorkspaceSymbol => {
                let query = args.query.as_deref().map(str::trim).unwrap_or("");
                if query.is_empty() {
                    return Err(FunctionCallError::RespondToModel(
                        "workspaceSymbol requires a non-empty `query` string".to_string(),
                    ));
                }
                let registries = self.state.lsp_registries(args.root.as_deref()).await;
                if registries.is_empty() {
                    return Err(FunctionCallError::RespondToModel(
                        if let Some(root) = args.root.as_deref() {
                            format!("unknown root '{root}' or root has no LSP registry")
                        } else {
                            "no LSP roots are configured".to_string()
                        },
                    ));
                }
                let mut symbols = Vec::new();
                let mut any_running_clients = false;
                for (root_name, registry) in &registries {
                    self.warm_workspace(root_name, registry).await;
                    symbols.extend(registry.workspace_symbol(query).await);
                    any_running_clients |= registry.running_client_count().await > 0;
                }
                // Retry with progressive backoff — TS server may need time to index.
                for attempt in 1..=3u64 {
                    if !symbols.is_empty() || !any_running_clients {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(attempt)).await;
                    for (_root_name, registry) in &registries {
                        symbols.extend(registry.workspace_symbol(query).await);
                    }
                }

                if symbols.is_empty() && !any_running_clients {
                    return Err(FunctionCallError::RespondToModel(
                        "no LSP servers are running (failed to start any). Use `code_intel` for tree-sitter-based analysis or `grep` for text search instead.".to_string(),
                    ));
                }
                let total = symbols.len();
                let truncated = total > limit;
                (
                    format_workspace_symbols(&symbols, limit, total, truncated),
                    false,
                )
            }
            LspOperation::GoToImplementation => {
                let context = self
                    .prepare_position_query_context(&args, turn_cwd.as_path())
                    .await?;
                let (resp, resolved) = fuzz_query(
                    fuzz,
                    context.line,
                    context.character,
                    FUZZ_ATTEMPTS_LONG,
                    |line, character| {
                        context
                            .registry
                            .implementation(&context.path, line, character)
                    },
                    is_empty_definition,
                )
                .await;

                let mut out = resolved_at_prefix(&context.path, resolved);
                out.push_str(&format_implementation(resp));
                (out, false)
            }
            LspOperation::PrepareCallHierarchy => {
                let context = self
                    .prepare_position_query_context(&args, turn_cwd.as_path())
                    .await?;
                let (items, _) = fuzz_query(
                    fuzz,
                    context.line,
                    context.character,
                    FUZZ_ATTEMPTS_LONG,
                    |line, character| {
                        context
                            .registry
                            .prepare_call_hierarchy(&context.path, line, character)
                    },
                    |items: &Vec<CallHierarchyItem>| items.is_empty(),
                )
                .await;

                (
                    serde_json::to_string_pretty(
                        &items.into_iter().take(limit).collect::<Vec<_>>(),
                    )
                    .unwrap_or_else(|_| "[]".to_string()),
                    false,
                )
            }
            LspOperation::IncomingCalls => (
                self.call_hierarchy_query(&args, limit, CallHierarchyDirection::Incoming)
                    .await?,
                false,
            ),
            LspOperation::OutgoingCalls => (
                self.call_hierarchy_query(&args, limit, CallHierarchyDirection::Outgoing)
                    .await?,
                false,
            ),
            LspOperation::PrepareRename => {
                let context = self
                    .prepare_position_query_context(&args, turn_cwd.as_path())
                    .await?;
                let (resp, resolved) = fuzz_query(
                    fuzz,
                    context.line,
                    context.character,
                    FUZZ_ATTEMPTS_LONG,
                    |line, character| {
                        context
                            .registry
                            .prepare_rename(&context.path, line, character)
                    },
                    |resp: &Option<PrepareRenameResponse>| resp.is_none(),
                )
                .await;

                let mut out = resolved_at_prefix(&context.path, resolved);
                out.push_str(&format_prepare_rename(resp));
                (out, false)
            }
            LspOperation::RenamePreview => {
                let new_name = args.new_name.as_deref().map(str::trim).unwrap_or("");
                if new_name.is_empty() {
                    return Err(FunctionCallError::RespondToModel(
                        "renamePreview requires `new_name`".to_string(),
                    ));
                }

                let context = self
                    .prepare_position_query_context(&args, turn_cwd.as_path())
                    .await?;
                let (edit, _) = fuzz_query(
                    fuzz,
                    context.line,
                    context.character,
                    FUZZ_ATTEMPTS_LONG,
                    |line, character| {
                        context
                            .registry
                            .rename(&context.path, line, character, new_name)
                    },
                    |edit: &Option<codex_lsp_client::lsp_types::WorkspaceEdit>| edit.is_none(),
                )
                .await;

                let edit = edit.ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "rename produced no workspace edit".to_string(),
                    )
                })?;

                let patch = workspace_edit_to_apply_patch(
                    &edit,
                    PatchLimits {
                        max_files_touched: 50,
                        max_patch_bytes: MAX_PATCH_BYTES,
                    },
                )
                .map_err(FunctionCallError::RespondToModel)?;
                (patch, true)
            }
            LspOperation::CodeActionPreview => {
                let context = self
                    .prepare_position_query_context(&args, turn_cwd.as_path())
                    .await?;
                let line = context.line;
                let character = context.character;
                let end_line = args.end_line.unwrap_or(line.saturating_add(1));
                let end_character = args.end_character.unwrap_or(character.saturating_add(1));
                if end_line == 0 || end_character == 0 {
                    return Err(FunctionCallError::RespondToModel(
                        "`end_line` and `end_character` must be 1-based".to_string(),
                    ));
                }
                let range = Range {
                    start: codex_lsp_client::lsp_types::Position { line, character },
                    end: codex_lsp_client::lsp_types::Position {
                        line: end_line - 1,
                        character: end_character - 1,
                    },
                };

                let only_kind = args
                    .only
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("quickfix");
                let only = Some(vec![CodeActionKind::from(only_kind.to_string())]);

                let diags: Vec<Diagnostic> =
                    context.registry.diagnostics_for_file(&context.path).await;
                let (actions, _) = fuzz_query(
                    fuzz,
                    line,
                    character,
                    FUZZ_ATTEMPTS_SHORT,
                    |query_line, query_char| {
                        let range = if query_line == line && query_char == character {
                            range
                        } else {
                            Range {
                                start: codex_lsp_client::lsp_types::Position {
                                    line: query_line,
                                    character: query_char,
                                },
                                end: codex_lsp_client::lsp_types::Position {
                                    line: query_line,
                                    character: query_char,
                                },
                            }
                        };
                        let diagnostics = if query_line == line && query_char == character {
                            diags.clone()
                        } else {
                            Vec::new()
                        };
                        context.registry.code_action(
                            &context.path,
                            range,
                            only.clone(),
                            diagnostics,
                        )
                    },
                    |actions: &Vec<CodeActionOrCommand>| actions.is_empty(),
                )
                .await;

                let title = args
                    .title
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let (ready, resolvable) = filter_previewable_code_actions(&actions);
                if ready.is_empty() && resolvable.is_empty() {
                    return Err(FunctionCallError::RespondToModel(
                        "no previewable code actions found (need an action with `edit` and no `command`, or with `data` for resolve)".to_string(),
                    ));
                }

                // Combine ready + resolvable for title matching; prefer ready.
                let mut candidates: Vec<CodeAction> = ready;
                candidates.extend(resolvable);

                let mut chosen = choose_code_action(&candidates, title)?;

                // If the chosen action has `data` but no `edit`, try resolving it.
                if chosen.edit.is_none()
                    && let Some(resolved) = context
                        .registry
                        .code_action_resolve(&context.path, chosen.clone())
                        .await
                {
                    chosen = resolved;
                }

                let edit = chosen.edit.ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "selected code action has no edit (resolve did not provide one)"
                            .to_string(),
                    )
                })?;
                let patch = workspace_edit_to_apply_patch(
                    &edit,
                    PatchLimits {
                        max_files_touched: 50,
                        max_patch_bytes: MAX_PATCH_BYTES,
                    },
                )
                .map_err(FunctionCallError::RespondToModel)?;
                (patch, true)
            }
            LspOperation::Diagnostics => {
                let context = self
                    .prepare_file_query_context(&args, turn_cwd.as_path())
                    .await?;
                // Wait for diagnostics (touch_file with wait=true triggers the
                // debounced diagnostic collection from all applicable servers).
                let all_diags = context.registry.touch_file(&context.path, true).await;
                let mut out = String::new();
                let mut count = 0usize;
                for (server_id, diags) in &all_diags {
                    for diag in diags {
                        count += 1;
                        let severity = match diag.severity {
                            Some(codex_lsp_client::lsp_types::DiagnosticSeverity::ERROR) => "ERROR",
                            Some(codex_lsp_client::lsp_types::DiagnosticSeverity::WARNING) => {
                                "WARNING"
                            }
                            Some(codex_lsp_client::lsp_types::DiagnosticSeverity::INFORMATION) => {
                                "INFO"
                            }
                            Some(codex_lsp_client::lsp_types::DiagnosticSeverity::HINT) => "HINT",
                            _ => "UNKNOWN",
                        };
                        let line = diag.range.start.line + 1;
                        let col = diag.range.start.character + 1;
                        let _ = writeln!(
                            out,
                            "{severity} [{line}:{col}] ({server_id}) {}",
                            diag.message
                        );
                    }
                }
                if count == 0 {
                    out.push_str("No diagnostics.");
                } else {
                    out.insert_str(
                        0,
                        &format!("{count} diagnostic(s) for {}:\n", context.path.display()),
                    );
                }
                (out, false)
            }
        };

        let out = if is_patch {
            if result.len() > MAX_PATCH_BYTES {
                return Err(FunctionCallError::RespondToModel(format!(
                    "preview patch is {} bytes, exceeding limit {MAX_PATCH_BYTES}",
                    result.len()
                )));
            }
            result
        } else {
            let (truncated_output, was_truncated) =
                truncate_tool_output(&result, HANDLER_MAX_RESULT_BYTES);
            if was_truncated {
                format!("[output truncated to 8KB]\n{truncated_output}")
            } else {
                truncated_output
            }
        };

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(out),
            success: Some(true),
        })
    }
}

impl LspToolHandler {
    async fn registry_for_root(
        &self,
        root_name: &str,
    ) -> Result<(String, Arc<ServerRegistry>), FunctionCallError> {
        self.state
            .lsp_registries(Some(root_name))
            .await
            .into_iter()
            .next()
            .ok_or_else(|| {
                FunctionCallError::RespondToModel(format!(
                    "unknown root '{root_name}' or root has no LSP registry"
                ))
            })
    }

    async fn registry_for_file(
        &self,
        path: &Path,
        root_name: Option<&str>,
    ) -> Result<(String, Arc<ServerRegistry>), FunctionCallError> {
        self.state
            .lsp_registry_for_file(path, root_name)
            .await
            .ok_or_else(|| {
                if let Some(root_name) = root_name {
                    FunctionCallError::RespondToModel(format!(
                        "file '{}' is not inside root '{root_name}', or root has no LSP registry",
                        path.display()
                    ))
                } else {
                    FunctionCallError::RespondToModel(format!(
                        "no registered root contains file '{}'",
                        path.display()
                    ))
                }
            })
    }

    async fn prepare_file_query_context(
        &self,
        args: &LspToolArgs,
        _default_cwd: &Path,
    ) -> Result<FileQueryContext, FunctionCallError> {
        let path = require_absolute_path_argument(args.file.as_deref(), "file")?;
        let (root_name, registry) = self.registry_for_file(&path, args.root.as_deref()).await?;
        self.warm_workspace(&root_name, &registry).await;
        self.sync_file_for_query(&registry, &root_name, &path)
            .await?;
        Ok(FileQueryContext { path, registry })
    }

    async fn prepare_position_query_context(
        &self,
        args: &LspToolArgs,
        _default_cwd: &Path,
    ) -> Result<PositionedQueryContext, FunctionCallError> {
        let path = require_absolute_path_argument(args.file.as_deref(), "file")?;

        // Fast-fail: validate 1-based positions before expensive server sync.
        if let (Some(line), Some(character)) = (args.line, args.character)
            && (line == 0 || character == 0)
        {
            return Err(FunctionCallError::RespondToModel(
                "`line` and `character` must be 1-based".to_string(),
            ));
        }

        let (root_name, registry) = self.registry_for_file(&path, args.root.as_deref()).await?;
        self.warm_workspace(&root_name, &registry).await;
        self.sync_file_for_query(&registry, &root_name, &path)
            .await?;
        let (line, character) = self
            .resolve_line_char(&registry, &root_name, &path, args)
            .await?;
        Ok(PositionedQueryContext {
            path,
            registry,
            line,
            character,
        })
    }

    async fn call_hierarchy_query(
        &self,
        args: &LspToolArgs,
        limit: usize,
        direction: CallHierarchyDirection,
    ) -> Result<String, FunctionCallError> {
        let operation_name = direction.operation_name();
        let item_value = args.item.clone().ok_or_else(|| {
            FunctionCallError::RespondToModel(format!(
                "{operation_name} requires `item` from prepareCallHierarchy"
            ))
        })?;
        let item: CallHierarchyItem = serde_json::from_value(item_value)
            .map_err(|error| FunctionCallError::RespondToModel(format!("invalid item: {error}")))?;

        let path = url::Url::parse(item.uri.as_str())
            .ok()
            .and_then(|uri| uri.to_file_path().ok());
        let (root_name, registry) = if let Some(path) = path {
            self.registry_for_file(&path, args.root.as_deref()).await?
        } else if let Some(root) = args.root.as_deref() {
            self.registry_for_root(root).await?
        } else {
            return Err(FunctionCallError::RespondToModel(format!(
                "{operation_name} could not infer a file root from item URI; pass `root` explicitly"
            )));
        };
        self.warm_workspace(&root_name, &registry).await;

        let pretty = match direction {
            CallHierarchyDirection::Incoming => {
                let calls = registry.incoming_calls(item).await;
                serde_json::to_string_pretty(&calls.into_iter().take(limit).collect::<Vec<_>>())
            }
            CallHierarchyDirection::Outgoing => {
                let calls = registry.outgoing_calls(item).await;
                serde_json::to_string_pretty(&calls.into_iter().take(limit).collect::<Vec<_>>())
            }
        };
        Ok(pretty.unwrap_or_else(|_| "[]".to_string()))
    }

    async fn sync_file_for_query(
        &self,
        registry: &ServerRegistry,
        root_name: &str,
        path: &Path,
    ) -> Result<(), FunctionCallError> {
        if !path.exists() {
            return Err(FunctionCallError::RespondToModel(format!(
                "file not found: {}",
                path.display()
            )));
        }
        if !path.is_file() {
            return Err(FunctionCallError::RespondToModel(format!(
                "path is not a file: {}",
                path.display()
            )));
        }
        if !registry.has_servers_for(path) {
            return Err(FunctionCallError::RespondToModel(format!(
                "no LSP server configured for {} under root '{}'. Use `code_intel` for tree-sitter-based analysis or `grep` for text search instead.",
                path.display(),
                root_name
            )));
        }

        let mut clients = registry.get_clients(path).await;
        if clients.is_empty() {
            // Retry once: a previously broken server may now work if
            // the agent installed missing dependencies during this session.
            registry.clear_broken_for_path(path).await;
            clients = registry.get_clients(path).await;
        }
        if clients.is_empty() {
            let display_path = path.display();
            let details = registry.explain_unavailable_servers(path).await;
            let details_block = if details.is_empty() {
                String::new()
            } else {
                format!("\nstartup details:\n- {}", details.join("\n- "))
            };
            return Err(FunctionCallError::RespondToModel(format!(
                "no LSP server could be started for {display_path} under root '{root_name}'{details_block}\n\nUse `code_intel` for tree-sitter-based analysis or `grep` for text search instead."
            )));
        }

        // On first contact with a file, wait for diagnostics as a proxy for
        // server readiness (e.g. Pyright needs time to index after startup).
        // Subsequent queries skip the wait to avoid latency.
        let first_contact = {
            let mut warmed = self.warmed_files.lock().await;
            warmed.insert(path.to_path_buf())
        };
        let _ = registry.touch_file(path, first_contact).await;
        Ok(())
    }

    /// Ensure workspace-wide LSP readiness by syncing a representative file
    /// the first time a workspace-scoped query (e.g. `workspaceSymbol`) is
    /// issued against a given root, or when the registry instance has changed
    /// (e.g. after removeRoot + addRoot with the same name).
    async fn warm_workspace(&self, root_name: &str, registry: &Arc<ServerRegistry>) {
        let ptr = Arc::as_ptr(registry) as usize;
        {
            let warmed = self.warmed_workspaces.lock().await;
            if warmed.get(root_name) == Some(&ptr) {
                return;
            }
        }

        // Clear stale warmed_files entries under this root so they get
        // re-synced with the new registry instance.
        let ws_root = registry.workspace_root().to_path_buf();
        {
            let mut files = self.warmed_files.lock().await;
            files.retain(|p| !p.starts_with(&ws_root));
        }

        // Shallow walk (depth ≤ 3) for the first file the registry can handle.
        let candidate = Self::find_warmup_candidate(&ws_root, registry, 3);

        if let Some(file) = candidate {
            // Best-effort: ignore errors (server may still start for other reasons).
            let _ = self.sync_file_for_query(registry, root_name, &file).await;
        }

        self.warmed_workspaces
            .lock()
            .await
            .insert(root_name.to_string(), ptr);
    }

    /// Walk `dir` up to `max_depth` levels looking for the first file
    /// that `registry.has_servers_for()` matches.
    fn find_warmup_candidate(
        dir: &Path,
        registry: &ServerRegistry,
        max_depth: usize,
    ) -> Option<PathBuf> {
        Self::walk_for_candidate(dir, registry, 0, max_depth)
    }

    fn walk_for_candidate(
        dir: &Path,
        registry: &ServerRegistry,
        depth: usize,
        max_depth: usize,
    ) -> Option<PathBuf> {
        let entries = std::fs::read_dir(dir).ok()?;
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && registry.has_servers_for(&path) {
                return Some(path);
            }
            if path.is_dir()
                && depth < max_depth
                && !path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.') || n == "node_modules")
            {
                subdirs.push(path);
            }
        }
        for sub in subdirs {
            if let Some(found) = Self::walk_for_candidate(&sub, registry, depth + 1, max_depth) {
                return Some(found);
            }
        }
        None
    }

    async fn resolve_line_char(
        &self,
        registry: &ServerRegistry,
        root_name: &str,
        path: &Path,
        args: &LspToolArgs,
    ) -> Result<(u32, u32), FunctionCallError> {
        if let (Some(line), Some(character)) = (args.line, args.character) {
            if line == 0 || character == 0 {
                return Err(FunctionCallError::RespondToModel(
                    "`line` and `character` must be 1-based".to_string(),
                ));
            }
            return Ok((line - 1, character - 1));
        }

        let symbol = args.symbol.as_deref().map(str::trim).unwrap_or("");
        if !symbol.is_empty() {
            return self
                .resolve_symbol_to_position(registry, root_name, path, symbol)
                .await;
        }

        Err(FunctionCallError::RespondToModel(
            "position required: provide (`line`, `character`) or `symbol`".to_string(),
        ))
    }

    async fn resolve_symbol_to_position(
        &self,
        registry: &ServerRegistry,
        root_name: &str,
        path: &Path,
        symbol: &str,
    ) -> Result<(u32, u32), FunctionCallError> {
        // Fast path: if tree-sitter index is already ready, use it without blocking.
        #[cfg(feature = "treesitter")]
        if let Some((_, index)) = self
            .state
            .try_treesitter_index_for_file(path, Some(root_name))
            .await
            && let Ok(rel) = index.rel_path_for_absolute(path)
        {
            let symbols = index.symbol_table().symbols_in_file(&rel);
            let matches: Vec<_> = symbols
                .into_iter()
                .filter(|s| symbol_name_matches(&s.name, symbol))
                .collect();

            if matches.len() == 1 {
                let s = &matches[0];
                let line0 = (s.line_range.0 as u32).saturating_sub(1);
                return Ok((line0, 0));
            }

            if !matches.is_empty() {
                let mut lines = Vec::new();
                for s in matches {
                    lines.push(format!(
                        "{:?} {} [{}-{}] {}",
                        s.kind, s.name, s.line_range.0, s.line_range.1, s.signature
                    ));
                }
                return Err(FunctionCallError::RespondToModel(format!(
                    "multiple symbols named '{symbol}' found in {}:\n- {}",
                    path.display(),
                    lines.join("\n- ")
                )));
            }
        }
        // Fallback: ask the server for document symbols and pick the best match.
        let resp = registry.document_symbol(path).await;
        let mut candidates = Vec::new();
        collect_document_symbol_candidates(&resp, symbol, &mut candidates);

        if candidates.len() == 1 {
            return Ok((candidates[0].0, candidates[0].1));
        }

        if candidates.is_empty() {
            return Err(FunctionCallError::RespondToModel(format!(
                "symbol '{symbol}' not found in {}",
                path.display()
            )));
        }

        let mut lines = Vec::new();
        for (line0, char0, name, kind) in candidates {
            lines.push(format!(
                "{kind} {name} @ {}:{}:{}",
                path.display(),
                line0 + 1,
                char0 + 1
            ));
        }
        Err(FunctionCallError::RespondToModel(format!(
            "multiple symbols matching '{symbol}' found in {}:\n- {}",
            path.display(),
            lines.join("\n- ")
        )))
    }
}

// ---------------------------------------------------------------------------
// Tool spec construction
// ---------------------------------------------------------------------------

pub(crate) fn create_lsp_tool() -> ToolSpec {
    use std::collections::BTreeMap;

    let mut properties = BTreeMap::new();
    properties.insert(
        "operation".to_string(),
        JsonSchema::String {
            description: Some(
                "The LSP operation to perform: goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, goToImplementation, prepareCallHierarchy, incomingCalls, outgoingCalls, prepareRename, renamePreview, codeActionPreview, diagnostics".to_string(),
            ),
        },
    );
    properties.insert(
        "file".to_string(),
        JsonSchema::String {
            description: Some(
                "Absolute path to the file (required for most operations)".to_string(),
            ),
        },
    );
    properties.insert(
        "line".to_string(),
        JsonSchema::Number {
            description: Some("1-based line number".to_string()),
        },
    );
    properties.insert(
        "character".to_string(),
        JsonSchema::Number {
            description: Some("1-based character/column number".to_string()),
        },
    );
    properties.insert(
        "symbol".to_string(),
        JsonSchema::String {
            description: Some(
                "Alternative to line/character: resolve a symbol name within `file` to a position (best-effort)."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "fuzz".to_string(),
        JsonSchema::Boolean {
            description: Some(
                "When a position-based query returns empty, retry nearby positions (default true)."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "query".to_string(),
        JsonSchema::String {
            description: Some("Search query for workspaceSymbol operation".to_string()),
        },
    );
    properties.insert(
        "root".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional root name. If omitted, root is inferred from file path or all roots are searched when supported."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "limit".to_string(),
        JsonSchema::Number {
            description: Some("Result limit (default 10, max 50).".to_string()),
        },
    );
    properties.insert(
        "end_line".to_string(),
        JsonSchema::Number {
            description: Some("1-based end line number (for codeActionPreview range).".to_string()),
        },
    );
    properties.insert(
        "end_character".to_string(),
        JsonSchema::Number {
            description: Some(
                "1-based end character/column number (for codeActionPreview range).".to_string(),
            ),
        },
    );
    properties.insert(
        "new_name".to_string(),
        JsonSchema::String {
            description: Some("New symbol name (for renamePreview).".to_string()),
        },
    );
    properties.insert(
        "only".to_string(),
        JsonSchema::String {
            description: Some(
                "Code action kind filter (for codeActionPreview), e.g. 'quickfix' (default)."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "title".to_string(),
        JsonSchema::String {
            description: Some(
                "Code action title selector when multiple actions are available (for codeActionPreview)."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "item".to_string(),
        JsonSchema::Object {
            properties: BTreeMap::new(),
            required: None,
            additional_properties: Some(true.into()),
        },
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "lsp".to_string(),
        description: LSP_TOOL_DESCRIPTION.to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["operation".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}
