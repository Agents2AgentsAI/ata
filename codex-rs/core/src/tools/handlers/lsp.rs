//! LSP tool handler exposing 9 code intelligence operations to the agent.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use codex_lsp_client::ServerRegistry;
use codex_lsp_client::lsp_types::CallHierarchyItem;
use codex_lsp_client::lsp_types::DocumentSymbol;
use codex_lsp_client::lsp_types::DocumentSymbolResponse;
use codex_lsp_client::lsp_types::GotoDefinitionResponse;
use codex_lsp_client::lsp_types::Hover;
use codex_lsp_client::lsp_types::HoverContents;
use codex_lsp_client::lsp_types::Location;
use codex_lsp_client::lsp_types::MarkedString;
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
use crate::tools::handlers::function_arguments_from_payload;
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::truncate_tool_output;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::spec::JsonSchema;

const LSP_TOOL_DESCRIPTION: &str = include_str!("tool_lsp.txt");
const DEFAULT_LIMIT: usize = 20;
const MAX_RESULTS: usize = 50;
const MAX_RESULT_BYTES: usize = 8 * 1024;

/// Handler for the `lsp` tool.
pub struct LspToolHandler {
    pub state: Arc<MultiRootState>,
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
    query: Option<String>,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
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
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_RESULTS);

        let result = match args.operation {
            LspOperation::GoToDefinition => {
                let (path, line, char) = extract_position(&args)?;
                let (root_name, registry) =
                    self.registry_for_file(&path, args.root.as_deref()).await?;
                self.sync_file_for_query(&registry, &root_name, &path)
                    .await?;
                let resp = registry.definition(&path, line, char).await;
                format_definition(resp)
            }
            LspOperation::FindReferences => {
                let (path, line, char) = extract_position(&args)?;
                let (root_name, registry) =
                    self.registry_for_file(&path, args.root.as_deref()).await?;
                self.sync_file_for_query(&registry, &root_name, &path)
                    .await?;
                let refs = registry.references(&path, line, char).await;
                format_references(&refs, limit)
            }
            LspOperation::Hover => {
                let (path, line, char) = extract_position(&args)?;
                let (root_name, registry) =
                    self.registry_for_file(&path, args.root.as_deref()).await?;
                self.sync_file_for_query(&registry, &root_name, &path)
                    .await?;
                let hover = registry.hover(&path, line, char).await;
                format_hover(hover)
            }
            LspOperation::DocumentSymbol => {
                let path = extract_file(&args)?;
                let (root_name, registry) =
                    self.registry_for_file(&path, args.root.as_deref()).await?;
                self.sync_file_for_query(&registry, &root_name, &path)
                    .await?;
                let resp = registry.document_symbol(&path).await;
                format_document_symbols(resp)
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
                for (_, registry) in registries {
                    symbols.extend(registry.workspace_symbol(query).await);
                    any_running_clients |= registry.running_client_count().await > 0;
                }
                if symbols.is_empty() && !any_running_clients {
                    return Err(FunctionCallError::RespondToModel(
                        "no LSP servers are running (failed to start any)".to_string(),
                    ));
                }
                format_workspace_symbols(&symbols, limit)
            }
            LspOperation::GoToImplementation => {
                let (path, line, char) = extract_position(&args)?;
                let (root_name, registry) =
                    self.registry_for_file(&path, args.root.as_deref()).await?;
                self.sync_file_for_query(&registry, &root_name, &path)
                    .await?;
                let resp = registry.implementation(&path, line, char).await;
                format_implementation(resp)
            }
            LspOperation::PrepareCallHierarchy => {
                let (path, line, char) = extract_position(&args)?;
                let (root_name, registry) =
                    self.registry_for_file(&path, args.root.as_deref()).await?;
                self.sync_file_for_query(&registry, &root_name, &path)
                    .await?;
                let items = registry.prepare_call_hierarchy(&path, line, char).await;
                serde_json::to_string_pretty(&items.into_iter().take(limit).collect::<Vec<_>>())
                    .unwrap_or_else(|_| "[]".to_string())
            }
            LspOperation::IncomingCalls => {
                let item_val = args.item.ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "incomingCalls requires `item` from prepareCallHierarchy".to_string(),
                    )
                })?;
                let item: CallHierarchyItem = serde_json::from_value(item_val)
                    .map_err(|e| FunctionCallError::RespondToModel(format!("invalid item: {e}")))?;
                let path = url::Url::parse(item.uri.as_str())
                    .ok()
                    .and_then(|uri| uri.to_file_path().ok());
                let registry = if let Some(path) = path {
                    self.registry_for_file(&path, args.root.as_deref()).await?.1
                } else if let Some(root) = args.root.as_deref() {
                    self.registry_for_root(root).await?.1
                } else {
                    return Err(FunctionCallError::RespondToModel(
                        "incomingCalls could not infer a file root from item URI; pass `root` explicitly".to_string(),
                    ));
                };
                let calls = registry.incoming_calls(item).await;
                serde_json::to_string_pretty(&calls.into_iter().take(limit).collect::<Vec<_>>())
                    .unwrap_or_else(|_| "[]".to_string())
            }
            LspOperation::OutgoingCalls => {
                let item_val = args.item.ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "outgoingCalls requires `item` from prepareCallHierarchy".to_string(),
                    )
                })?;
                let item: CallHierarchyItem = serde_json::from_value(item_val)
                    .map_err(|e| FunctionCallError::RespondToModel(format!("invalid item: {e}")))?;
                let path = url::Url::parse(item.uri.as_str())
                    .ok()
                    .and_then(|uri| uri.to_file_path().ok());
                let registry = if let Some(path) = path {
                    self.registry_for_file(&path, args.root.as_deref()).await?.1
                } else if let Some(root) = args.root.as_deref() {
                    self.registry_for_root(root).await?.1
                } else {
                    return Err(FunctionCallError::RespondToModel(
                        "outgoingCalls could not infer a file root from item URI; pass `root` explicitly".to_string(),
                    ));
                };
                let calls = registry.outgoing_calls(item).await;
                serde_json::to_string_pretty(&calls.into_iter().take(limit).collect::<Vec<_>>())
                    .unwrap_or_else(|_| "[]".to_string())
            }
        };

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(truncate_tool_output(&result, MAX_RESULT_BYTES)),
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

        let clients = registry.get_clients(path).await;
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

        let _ = registry.touch_file(path, false).await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Argument extraction
// ---------------------------------------------------------------------------

fn extract_file(args: &LspToolArgs) -> Result<PathBuf, FunctionCallError> {
    let file = args
        .file
        .as_deref()
        .ok_or_else(|| FunctionCallError::RespondToModel("`file` is required".to_string()))?;
    let path = PathBuf::from(file);
    if !path.is_absolute() {
        return Err(FunctionCallError::RespondToModel(
            "`file` must be an absolute path".to_string(),
        ));
    }
    Ok(path)
}

fn extract_position(args: &LspToolArgs) -> Result<(PathBuf, u32, u32), FunctionCallError> {
    let path = extract_file(args)?;
    let line = args.line.ok_or_else(|| {
        FunctionCallError::RespondToModel("`line` is required (1-based)".to_string())
    })?;
    let character = args.character.ok_or_else(|| {
        FunctionCallError::RespondToModel("`character` is required (1-based)".to_string())
    })?;
    if line == 0 || character == 0 {
        return Err(FunctionCallError::RespondToModel(
            "`line` and `character` must be 1-based".to_string(),
        ));
    }
    // Convert from 1-based (user) to 0-based (LSP).
    Ok((path, line - 1, character - 1))
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_definition(resp: Option<GotoDefinitionResponse>) -> String {
    match resp {
        None => "No definition found.".to_string(),
        Some(GotoDefinitionResponse::Scalar(loc)) => format_location(&loc),
        Some(GotoDefinitionResponse::Array(locs)) => {
            if locs.is_empty() {
                "No definition found.".to_string()
            } else {
                locs.iter()
                    .map(format_location)
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            if links.is_empty() {
                "No definition found.".to_string()
            } else {
                links
                    .iter()
                    .map(|l| {
                        format!(
                            "{}:{}:{}",
                            l.target_uri.as_str(),
                            l.target_range.start.line + 1,
                            l.target_range.start.character + 1
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    }
}

fn format_implementation(resp: Option<GotoImplementationResponse>) -> String {
    // GotoImplementationResponse is a type alias for GotoDefinitionResponse.
    format_definition(resp)
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

fn format_location(loc: &Location) -> String {
    format!(
        "{}:{}:{}",
        loc.uri.as_str(),
        loc.range.start.line + 1,
        loc.range.start.character + 1
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
            format!(
                "{:?} {} @ {}:{}:{}",
                s.kind,
                s.name,
                s.location.uri.as_str(),
                s.location.range.start.line + 1,
                s.location.range.start.character + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
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
                "The LSP operation to perform: goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, goToImplementation, prepareCallHierarchy, incomingCalls, outgoingCalls".to_string(),
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
