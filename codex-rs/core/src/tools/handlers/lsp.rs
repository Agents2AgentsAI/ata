//! LSP tool handler exposing 9 code intelligence operations to the agent.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use codex_lsp_client::ServerRegistry;
use codex_protocol::models::FunctionCallOutputBody;
use serde::Deserialize;

use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::spec::JsonSchema;

const LSP_TOOL_DESCRIPTION: &str = include_str!("tool_lsp.txt");

/// Handler for the `lsp` tool.
pub struct LspToolHandler {
    pub registry: Arc<ServerRegistry>,
}

#[derive(Deserialize)]
struct LspToolArgs {
    operation: String,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    character: Option<u32>,
    #[serde(default)]
    query: Option<String>,
    /// Serialized CallHierarchyItem for incoming/outgoing calls.
    #[serde(default)]
    item: Option<serde_json::Value>,
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

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "lsp handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: LspToolArgs = parse_arguments(&arguments)?;

        let result = match args.operation.as_str() {
            "goToDefinition" => {
                let (path, line, char) = extract_position(&args)?;
                let resp = self.registry.definition(&path, line, char).await;
                format_definition(resp)
            }
            "findReferences" => {
                let (path, line, char) = extract_position(&args)?;
                let refs = self.registry.references(&path, line, char).await;
                format_references(&refs)
            }
            "hover" => {
                let (path, line, char) = extract_position(&args)?;
                let hover = self.registry.hover(&path, line, char).await;
                format_hover(hover)
            }
            "documentSymbol" => {
                let path = extract_file(&args)?;
                let resp = self.registry.document_symbol(&path).await;
                format_document_symbols(resp)
            }
            "workspaceSymbol" => {
                let query = args.query.as_deref().unwrap_or("");
                let symbols = self.registry.workspace_symbol(query).await;
                format_workspace_symbols(&symbols)
            }
            "goToImplementation" => {
                let (path, line, char) = extract_position(&args)?;
                let resp = self.registry.implementation(&path, line, char).await;
                format_implementation(resp)
            }
            "prepareCallHierarchy" => {
                let (path, line, char) = extract_position(&args)?;
                let items = self
                    .registry
                    .prepare_call_hierarchy(&path, line, char)
                    .await;
                serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string())
            }
            "incomingCalls" => {
                let item_val = args.item.ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "incomingCalls requires `item` from prepareCallHierarchy".to_string(),
                    )
                })?;
                let item: codex_lsp_client::lsp_types::CallHierarchyItem =
                    serde_json::from_value(item_val).map_err(|e| {
                        FunctionCallError::RespondToModel(format!("invalid item: {e}"))
                    })?;
                let calls = self.registry.incoming_calls(item).await;
                serde_json::to_string_pretty(&calls).unwrap_or_else(|_| "[]".to_string())
            }
            "outgoingCalls" => {
                let item_val = args.item.ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "outgoingCalls requires `item` from prepareCallHierarchy".to_string(),
                    )
                })?;
                let item: codex_lsp_client::lsp_types::CallHierarchyItem =
                    serde_json::from_value(item_val).map_err(|e| {
                        FunctionCallError::RespondToModel(format!("invalid item: {e}"))
                    })?;
                let calls = self.registry.outgoing_calls(item).await;
                serde_json::to_string_pretty(&calls).unwrap_or_else(|_| "[]".to_string())
            }
            other => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "unknown LSP operation: {other}. Valid operations: goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, goToImplementation, prepareCallHierarchy, incomingCalls, outgoingCalls"
                )));
            }
        };

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(result),
            success: Some(true),
        })
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

fn format_definition(resp: Option<codex_lsp_client::lsp_types::GotoDefinitionResponse>) -> String {
    match resp {
        None => "No definition found.".to_string(),
        Some(codex_lsp_client::lsp_types::GotoDefinitionResponse::Scalar(loc)) => {
            format_location(&loc)
        }
        Some(codex_lsp_client::lsp_types::GotoDefinitionResponse::Array(locs)) => {
            if locs.is_empty() {
                "No definition found.".to_string()
            } else {
                locs.iter()
                    .map(format_location)
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        Some(codex_lsp_client::lsp_types::GotoDefinitionResponse::Link(links)) => {
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

fn format_implementation(
    resp: Option<codex_lsp_client::lsp_types::request::GotoImplementationResponse>,
) -> String {
    // GotoImplementationResponse is a type alias for GotoDefinitionResponse.
    format_definition(resp)
}

fn format_references(refs: &[codex_lsp_client::lsp_types::Location]) -> String {
    if refs.is_empty() {
        "No references found.".to_string()
    } else {
        refs.iter()
            .map(format_location)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn format_location(loc: &codex_lsp_client::lsp_types::Location) -> String {
    format!(
        "{}:{}:{}",
        loc.uri.as_str(),
        loc.range.start.line + 1,
        loc.range.start.character + 1
    )
}

fn format_hover(hover: Option<codex_lsp_client::lsp_types::Hover>) -> String {
    match hover {
        None => "No hover information available.".to_string(),
        Some(h) => match h.contents {
            codex_lsp_client::lsp_types::HoverContents::Scalar(content) => {
                format_markup_content(content)
            }
            codex_lsp_client::lsp_types::HoverContents::Array(contents) => contents
                .into_iter()
                .map(format_markup_content)
                .collect::<Vec<_>>()
                .join("\n---\n"),
            codex_lsp_client::lsp_types::HoverContents::Markup(markup) => markup.value,
        },
    }
}

fn format_markup_content(content: codex_lsp_client::lsp_types::MarkedString) -> String {
    match content {
        codex_lsp_client::lsp_types::MarkedString::String(s) => s,
        codex_lsp_client::lsp_types::MarkedString::LanguageString(ls) => {
            format!("```{}\n{}\n```", ls.language, ls.value)
        }
    }
}

fn format_document_symbols(
    resp: Option<codex_lsp_client::lsp_types::DocumentSymbolResponse>,
) -> String {
    match resp {
        None => "No symbols found.".to_string(),
        Some(codex_lsp_client::lsp_types::DocumentSymbolResponse::Flat(symbols)) => {
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
        Some(codex_lsp_client::lsp_types::DocumentSymbolResponse::Nested(symbols)) => {
            if symbols.is_empty() {
                return "No symbols found.".to_string();
            }
            let mut lines = Vec::new();
            format_nested_symbols(&symbols, 0, &mut lines);
            lines.join("\n")
        }
    }
}

fn format_nested_symbols(
    symbols: &[codex_lsp_client::lsp_types::DocumentSymbol],
    depth: usize,
    out: &mut Vec<String>,
) {
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
fn format_workspace_symbols(symbols: &[codex_lsp_client::lsp_types::SymbolInformation]) -> String {
    if symbols.is_empty() {
        return "No symbols found.".to_string();
    }
    symbols
        .iter()
        .take(10) // Limit to 10 results.
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
