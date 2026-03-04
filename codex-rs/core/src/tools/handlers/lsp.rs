//! LSP tool handler exposing code intelligence and preview refactor operations to the agent.

use std::collections::HashSet;
use std::fmt::Write;
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use codex_lsp_client::ServerRegistry;
use codex_lsp_client::lsp_types::CallHierarchyItem;
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
use crate::tools::handlers::absolute_path_argument;
use crate::tools::handlers::function_arguments_from_payload;
use crate::tools::handlers::lsp_workspace_edit::PatchLimits;
use crate::tools::handlers::lsp_workspace_edit::workspace_edit_to_apply_patch;
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::truncate_tool_output;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::spec::JsonSchema;

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
    pub warmed_workspaces: tokio::sync::Mutex<HashSet<String>>,
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
        let ToolInvocation { payload, .. } = invocation;

        let arguments = function_arguments_from_payload(payload, "lsp")?;
        let args: LspToolArgs = parse_arguments(&arguments)?;
        let limit = args
            .limit
            .unwrap_or(HANDLER_DEFAULT_LIMIT)
            .clamp(1, HANDLER_MAX_RESULTS);
        let fuzz = args.fuzz.unwrap_or(true);

        let (result, is_patch) = match args.operation {
            LspOperation::GoToDefinition => {
                let context = self.prepare_position_query_context(&args).await?;
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
                let context = self.prepare_position_query_context(&args).await?;
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
                let context = self.prepare_position_query_context(&args).await?;
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
                let context = self.prepare_file_query_context(&args).await?;
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
                // Retry once if empty — TS server may still be indexing workspace.
                if symbols.is_empty() && any_running_clients {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    for (_root_name, registry) in &registries {
                        symbols.extend(registry.workspace_symbol(query).await);
                    }
                }

                if symbols.is_empty() && !any_running_clients {
                    return Err(FunctionCallError::RespondToModel(
                        "no LSP servers are running (failed to start any)".to_string(),
                    ));
                }
                (format_workspace_symbols(&symbols, limit), false)
            }
            LspOperation::GoToImplementation => {
                let context = self.prepare_position_query_context(&args).await?;
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
                let context = self.prepare_position_query_context(&args).await?;
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
                let context = self.prepare_position_query_context(&args).await?;
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

                let context = self.prepare_position_query_context(&args).await?;
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
                let context = self.prepare_position_query_context(&args).await?;
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
                let previewable = filter_previewable_code_actions(&actions);
                if previewable.is_empty() {
                    return Err(FunctionCallError::RespondToModel(
                        "no previewable code actions found (need an action with `edit` and no `command`)".to_string(),
                    ));
                }

                let chosen = choose_code_action(&previewable, title)?;
                let edit = chosen.edit.clone().ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "selected code action has no edit".to_string(),
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
                let context = self.prepare_file_query_context(&args).await?;
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
            truncate_tool_output(&result, HANDLER_MAX_RESULT_BYTES)
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
    ) -> Result<FileQueryContext, FunctionCallError> {
        let path = absolute_path_argument(args.file.as_deref(), "file")?;
        let (root_name, registry) = self.registry_for_file(&path, args.root.as_deref()).await?;
        self.sync_file_for_query(&registry, &root_name, &path)
            .await?;
        Ok(FileQueryContext { path, registry })
    }

    async fn prepare_position_query_context(
        &self,
        args: &LspToolArgs,
    ) -> Result<PositionedQueryContext, FunctionCallError> {
        let path = absolute_path_argument(args.file.as_deref(), "file")?;

        // Fast-fail: validate 1-based positions before expensive server sync.
        if let (Some(line), Some(character)) = (args.line, args.character) {
            if line == 0 || character == 0 {
                return Err(FunctionCallError::RespondToModel(
                    "`line` and `character` must be 1-based".to_string(),
                ));
            }
        }

        let (root_name, registry) = self.registry_for_file(&path, args.root.as_deref()).await?;
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
        let registry = if let Some(path) = path {
            self.registry_for_file(&path, args.root.as_deref()).await?.1
        } else if let Some(root) = args.root.as_deref() {
            self.registry_for_root(root).await?.1
        } else {
            return Err(FunctionCallError::RespondToModel(format!(
                "{operation_name} could not infer a file root from item URI; pass `root` explicitly"
            )));
        };

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
                "no LSP server configured for {} under root '{}'",
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
                "no LSP server could be started for {display_path} under root '{root_name}'{details_block}"
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
    /// issued against a given root.
    async fn warm_workspace(&self, root_name: &str, registry: &ServerRegistry) {
        {
            let warmed = self.warmed_workspaces.lock().await;
            if warmed.contains(root_name) {
                return;
            }
        }

        // Shallow walk (depth ≤ 3) for the first file the registry can handle.
        let ws_root = registry.workspace_root().to_path_buf();
        let candidate = Self::find_warmup_candidate(&ws_root, registry, 3);

        if let Some(file) = candidate {
            // Best-effort: ignore errors (server may still start for other reasons).
            let _ = self.sync_file_for_query(registry, root_name, &file).await;
        }

        self.warmed_workspaces
            .lock()
            .await
            .insert(root_name.to_string());
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
                    .map_or(false, |n| n.starts_with('.') || n == "node_modules")
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
        {
            if let Ok(rel) = index.rel_path_for_absolute(path) {
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
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_definition_like(resp: Option<GotoDefinitionResponse>, not_found_msg: &str) -> String {
    match resp {
        None => not_found_msg.to_string(),
        Some(GotoDefinitionResponse::Scalar(loc)) => format_location(&loc),
        Some(GotoDefinitionResponse::Array(locs)) => {
            if locs.is_empty() {
                not_found_msg.to_string()
            } else {
                locs.iter()
                    .map(format_location)
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            if links.is_empty() {
                not_found_msg.to_string()
            } else {
                links
                    .iter()
                    .map(|l| {
                        format_uri_position(
                            l.target_uri.as_str(),
                            l.target_range.start.line,
                            l.target_range.start.character,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    }
}

fn format_definition(resp: Option<GotoDefinitionResponse>) -> String {
    format_definition_like(resp, "No definition found.")
}

fn format_implementation(resp: Option<GotoImplementationResponse>) -> String {
    format_definition_like(resp, "No implementation found.")
}

fn format_references(refs: &[Location], limit: usize) -> String {
    if refs.is_empty() {
        "No references found.".to_string()
    } else {
        refs.iter()
            .take(limit)
            .map(format_location)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn format_uri_position(uri: &str, line0: u32, char0: u32) -> String {
    if let Ok(url) = url::Url::parse(uri)
        && let Ok(path) = url.to_file_path()
    {
        return format!("{}:{}:{}", path.display(), line0 + 1, char0 + 1);
    }
    format!("{uri}:{}:{}", line0 + 1, char0 + 1)
}

fn format_location(loc: &Location) -> String {
    format_uri_position(
        loc.uri.as_str(),
        loc.range.start.line,
        loc.range.start.character,
    )
}

fn format_hover(hover: Option<Hover>) -> String {
    match hover {
        None => "No hover information available.".to_string(),
        Some(h) => match h.contents {
            HoverContents::Scalar(content) => format_markup_content(content),
            HoverContents::Array(contents) => contents
                .into_iter()
                .map(format_markup_content)
                .collect::<Vec<_>>()
                .join("\n---\n"),
            HoverContents::Markup(markup) => markup.value,
        },
    }
}

fn format_markup_content(content: MarkedString) -> String {
    match content {
        MarkedString::String(s) => s,
        MarkedString::LanguageString(ls) => {
            format!("```{}\n{}\n```", ls.language, ls.value)
        }
    }
}

fn format_document_symbols(resp: Option<DocumentSymbolResponse>) -> String {
    match resp {
        None => "No symbols found.".to_string(),
        Some(DocumentSymbolResponse::Flat(symbols)) => {
            if symbols.is_empty() {
                return "No symbols found.".to_string();
            }
            #[allow(deprecated)]
            symbols
                .iter()
                .map(|s| {
                    format!(
                        "{:?} {} [{}:{}]",
                        s.kind,
                        s.name,
                        s.location.range.start.line + 1,
                        s.location.range.start.character + 1
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        Some(DocumentSymbolResponse::Nested(symbols)) => {
            if symbols.is_empty() {
                return "No symbols found.".to_string();
            }
            let mut lines = Vec::new();
            format_nested_symbols(&symbols, 0, &mut lines);
            lines.join("\n")
        }
    }
}

fn format_nested_symbols(symbols: &[DocumentSymbol], depth: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    for s in symbols {
        out.push(format!(
            "{indent}{:?} {} [{}:{}]",
            s.kind,
            s.name,
            s.range.start.line + 1,
            s.range.start.character + 1
        ));
        if let Some(children) = &s.children {
            format_nested_symbols(children, depth + 1, out);
        }
    }
}

#[allow(deprecated)]
fn format_workspace_symbols(symbols: &[SymbolInformation], limit: usize) -> String {
    if symbols.is_empty() {
        return "No symbols found.".to_string();
    }
    symbols
        .iter()
        .take(limit)
        .map(|s| {
            let pos = format_uri_position(
                s.location.uri.as_str(),
                s.location.range.start.line,
                s.location.range.start.character,
            );
            format!("{:?} {} @ {}", s.kind, s.name, pos)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_empty_definition(resp: &Option<GotoDefinitionResponse>) -> bool {
    match resp {
        None => true,
        Some(GotoDefinitionResponse::Scalar(_)) => false,
        Some(GotoDefinitionResponse::Array(locs)) => locs.is_empty(),
        Some(GotoDefinitionResponse::Link(links)) => links.is_empty(),
    }
}

async fn fuzz_query<T, Q, Fut, E>(
    fuzz: bool,
    line: u32,
    character: u32,
    max_attempts: usize,
    mut query: Q,
    mut is_empty: E,
) -> (T, Option<(u32, u32)>)
where
    Q: FnMut(u32, u32) -> Fut,
    Fut: Future<Output = T>,
    E: FnMut(&T) -> bool,
{
    let mut result = query(line, character).await;
    let mut resolved = None;
    if fuzz && is_empty(&result) {
        for (candidate_line, candidate_character) in fuzz_positions(line, character)
            .into_iter()
            .skip(1)
            .take(max_attempts)
        {
            let candidate = query(candidate_line, candidate_character).await;
            if !is_empty(&candidate) {
                result = candidate;
                resolved = Some((candidate_line, candidate_character));
                break;
            }
        }
    }
    (result, resolved)
}

fn resolved_at_prefix(path: &Path, resolved: Option<(u32, u32)>) -> String {
    let mut out = String::new();
    if let Some((line, character)) = resolved {
        let _ = writeln!(
            out,
            "Resolved at {}:{}:{}",
            path.display(),
            line + 1,
            character + 1
        );
    }
    out
}

fn fuzz_positions(line0: u32, char0: u32) -> Vec<(u32, u32)> {
    const LINE_OFFSETS: [i32; 5] = [0, -1, 1, -2, 2];
    const CHAR_OFFSETS: [i32; 17] = [0, -1, 1, -2, 2, -3, 3, -4, 4, -5, 5, -6, 6, -7, 7, -8, 8];

    let mut out = Vec::with_capacity(LINE_OFFSETS.len() * CHAR_OFFSETS.len());
    let mut seen = std::collections::HashSet::new();
    for lo in LINE_OFFSETS {
        for co in CHAR_OFFSETS {
            let l = line0 as i32 + lo;
            let c = char0 as i32 + co;
            if l < 0 || c < 0 {
                continue;
            }
            let pos = (l as u32, c as u32);
            if seen.insert(pos) {
                out.push(pos);
            }
        }
    }
    out
}

fn symbol_name_matches(name: &str, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return false;
    }
    if name == query {
        return true;
    }
    if let Some(last) = query.rsplit("::").next() {
        return name == last;
    }
    false
}

fn collect_document_symbol_candidates(
    resp: &Option<DocumentSymbolResponse>,
    query: &str,
    out: &mut Vec<(u32, u32, String, String)>,
) {
    let Some(resp) = resp else {
        return;
    };

    match resp {
        DocumentSymbolResponse::Flat(symbols) =>
        {
            #[allow(deprecated)]
            for s in symbols {
                if symbol_name_matches(&s.name, query) {
                    out.push((
                        s.location.range.start.line,
                        s.location.range.start.character,
                        s.name.clone(),
                        format!("{:?}", s.kind),
                    ));
                }
            }
        }
        DocumentSymbolResponse::Nested(symbols) => {
            collect_nested_doc_symbols(symbols, query, out);
        }
    }
}

fn collect_nested_doc_symbols(
    symbols: &[DocumentSymbol],
    query: &str,
    out: &mut Vec<(u32, u32, String, String)>,
) {
    for s in symbols {
        if symbol_name_matches(&s.name, query) {
            out.push((
                s.selection_range.start.line,
                s.selection_range.start.character,
                s.name.clone(),
                format!("{:?}", s.kind),
            ));
        }
        if let Some(children) = &s.children {
            collect_nested_doc_symbols(children, query, out);
        }
    }
}

fn format_prepare_rename(resp: Option<PrepareRenameResponse>) -> String {
    let Some(resp) = resp else {
        return "No rename available.".to_string();
    };

    match resp {
        PrepareRenameResponse::Range(r) => format!(
            "Rename range: [{}:{}]-[{}:{}]",
            r.start.line + 1,
            r.start.character + 1,
            r.end.line + 1,
            r.end.character + 1
        ),
        PrepareRenameResponse::RangeWithPlaceholder { range, placeholder } => format!(
            "Rename range: [{}:{}]-[{}:{}]\nPlaceholder: {placeholder}",
            range.start.line + 1,
            range.start.character + 1,
            range.end.line + 1,
            range.end.character + 1
        ),
        PrepareRenameResponse::DefaultBehavior { default_behavior } => {
            format!("Rename supported (default behavior: {default_behavior}).",)
        }
    }
}

fn filter_previewable_code_actions(
    actions: &[CodeActionOrCommand],
) -> Vec<codex_lsp_client::lsp_types::CodeAction> {
    actions
        .iter()
        .filter_map(|item| match item {
            CodeActionOrCommand::CodeAction(action)
                if action.edit.is_some() && action.command.is_none() =>
            {
                Some(action.clone())
            }
            _ => None,
        })
        .collect()
}

fn choose_code_action(
    actions: &[codex_lsp_client::lsp_types::CodeAction],
    title: Option<&str>,
) -> Result<codex_lsp_client::lsp_types::CodeAction, FunctionCallError> {
    if let Some(title) = title {
        if let Some(action) = actions.iter().find(|a| a.title == title) {
            return Ok(action.clone());
        }
        return Err(FunctionCallError::RespondToModel(format!(
            "no code action titled '{title}'. Available:\n- {}",
            actions
                .iter()
                .map(|a| a.title.as_str())
                .collect::<Vec<_>>()
                .join("\n- ")
        )));
    }

    if actions.len() == 1 {
        return Ok(actions[0].clone());
    }

    Err(FunctionCallError::RespondToModel(format!(
        "multiple previewable code actions found; pass `title` to select one:\n- {}",
        actions
            .iter()
            .map(|a| a.title.as_str())
            .collect::<Vec<_>>()
            .join("\n- ")
    )))
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
            description: Some("Result limit (default 20, max 50).".to_string()),
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
