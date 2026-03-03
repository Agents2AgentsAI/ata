use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use codex_protocol::models::FunctionCallOutputBody;
use codex_treesitter::FileMark;
use codex_treesitter::GrepScope;
use codex_treesitter::SymbolKind;
use serde::Deserialize;
use serde_json::json;

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

const CODE_INTEL_TOOL_DESCRIPTION: &str = include_str!("tool_code_intel.txt");
const DEFAULT_LIMIT: usize = 20;
const MAX_RESULTS: usize = 50;
const MAX_RESULT_BYTES: usize = 8 * 1024;
const DEFAULT_CHUNK_SIZE: usize = 5000;
const DEFAULT_CHUNK_OVERLAP: usize = 200;

pub struct CodeIntelToolHandler {
    pub state: Arc<MultiRootState>,
}

#[derive(Debug, Deserialize)]
struct CodeIntelToolArgs {
    operation: CodeIntelOperation,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    definition: Option<String>,
    #[serde(default)]
    mark: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    depth: Option<usize>,
    #[serde(default)]
    chunk_size: Option<usize>,
    #[serde(default)]
    overlap: Option<usize>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    response_format: Option<ResponseFormat>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum CodeIntelOperation {
    SymbolSearch,
    Symbols,
    Callers,
    Tests,
    Variables,
    Implementation,
    Structure,
    Peek,
    Grep,
    ChunkIndices,
    DefineSymbol,
    RedefineSymbol,
    DefineFile,
    RedefineFile,
    MarkFile,
    SaveAnnotations,
    LoadAnnotations,
    AddRoot,
    RemoveRoot,
    ListRoots,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ResponseFormat {
    Text,
    Json,
}

impl Default for ResponseFormat {
    fn default() -> Self {
        Self::Text
    }
}

#[async_trait]
impl ToolHandler for CodeIntelToolHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, invocation: &ToolInvocation) -> bool {
        let Ok(arguments) =
            function_arguments_from_payload(invocation.payload.clone(), "code_intel")
        else {
            return false;
        };
        let Ok(args) = parse_arguments::<CodeIntelToolArgs>(&arguments) else {
            return false;
        };

        matches!(
            args.operation,
            CodeIntelOperation::DefineSymbol
                | CodeIntelOperation::RedefineSymbol
                | CodeIntelOperation::DefineFile
                | CodeIntelOperation::RedefineFile
                | CodeIntelOperation::MarkFile
                | CodeIntelOperation::SaveAnnotations
                | CodeIntelOperation::LoadAnnotations
                | CodeIntelOperation::AddRoot
                | CodeIntelOperation::RemoveRoot
        )
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation { payload, .. } = invocation;

        let arguments = function_arguments_from_payload(payload, "code_intel")?;

        let args: CodeIntelToolArgs = parse_arguments(&arguments)?;
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_RESULTS);
        let response_format = args.response_format.unwrap_or_default();

        let (output_text, output_json) = match args.operation {
            CodeIntelOperation::SymbolSearch => {
                let query = require_param(args.query.as_deref(), "query")?;
                let indices = self.indices_for_query(args.root.as_deref()).await?;
                let mut symbols = Vec::new();
                for (root, index) in indices {
                    for symbol in index.search_symbols(query, limit) {
                        symbols.push((root.clone(), symbol));
                        if symbols.len() >= limit {
                            break;
                        }
                    }
                    if symbols.len() >= limit {
                        break;
                    }
                }
                (
                    format_symbols(&symbols),
                    json!({
                        "count": symbols.len(),
                        "symbols": symbols
                            .iter()
                            .map(|(root, symbol)| json!({"root": root, "symbol": symbol}))
                            .collect::<Vec<_>>()
                    }),
                )
            }
            CodeIntelOperation::Symbols => {
                let kind = parse_symbol_kind(args.kind.as_deref())?;
                let mut symbols = Vec::new();

                if let Some(file) = args.file.as_deref() {
                    let path = absolute_file(Some(file))?;
                    let (root, index, rel_file) =
                        self.index_for_file(&path, args.root.as_deref()).await?;
                    for symbol in index.list_symbols(kind, Some(&rel_file), limit) {
                        symbols.push((root.clone(), symbol));
                        if symbols.len() >= limit {
                            break;
                        }
                    }
                } else {
                    let indices = self.indices_for_query(args.root.as_deref()).await?;
                    for (root, index) in indices {
                        for symbol in index.list_symbols(kind, None, limit) {
                            symbols.push((root.clone(), symbol));
                            if symbols.len() >= limit {
                                break;
                            }
                        }
                        if symbols.len() >= limit {
                            break;
                        }
                    }
                }

                (
                    format_symbols(&symbols),
                    json!({
                        "count": symbols.len(),
                        "symbols": symbols
                            .iter()
                            .map(|(root, symbol)| json!({"root": root, "symbol": symbol}))
                            .collect::<Vec<_>>()
                    }),
                )
            }
            CodeIntelOperation::Callers => {
                let symbol = require_param(args.symbol.as_deref(), "symbol")?;
                let path = absolute_file(args.file.as_deref())?;
                let (root, index, rel_file) =
                    self.index_for_file(&path, args.root.as_deref()).await?;
                let callers = index
                    .find_callers(symbol, &rel_file, limit)
                    .map_err(FunctionCallError::RespondToModel)?;
                (
                    format_callers(&root, &callers),
                    json!({"root": root, "count": callers.len(), "callers": callers}),
                )
            }
            CodeIntelOperation::Tests => {
                let symbol = require_param(args.symbol.as_deref(), "symbol")?;
                let path = absolute_file(args.file.as_deref())?;
                let (root, index, rel_file) =
                    self.index_for_file(&path, args.root.as_deref()).await?;
                let tests = index
                    .find_tests(symbol, &rel_file, limit)
                    .map_err(FunctionCallError::RespondToModel)?;
                (
                    format_tests(&root, &tests),
                    json!({"root": root, "count": tests.len(), "tests": tests}),
                )
            }
            CodeIntelOperation::Variables => {
                let function = require_param(args.symbol.as_deref(), "symbol")?;
                let path = absolute_file(args.file.as_deref())?;
                let (root, index, rel_file) =
                    self.index_for_file(&path, args.root.as_deref()).await?;
                let variables = index
                    .list_variables(function, &rel_file)
                    .map_err(FunctionCallError::RespondToModel)?;
                (
                    format_variables(&root, function, &variables),
                    json!({"root": root, "function": function, "count": variables.len(), "variables": variables}),
                )
            }
            CodeIntelOperation::Implementation => {
                let symbol = require_param(args.symbol.as_deref(), "symbol")?;
                let path = absolute_file(args.file.as_deref())?;
                let (root, index, rel_file) =
                    self.index_for_file(&path, args.root.as_deref()).await?;
                let implementation = index
                    .implementation(symbol, &rel_file)
                    .map_err(FunctionCallError::RespondToModel)?;
                (
                    format!("[{root}]\n{implementation}"),
                    json!({
                        "root": root,
                        "file": rel_file,
                        "symbol": symbol,
                        "source": implementation,
                    }),
                )
            }
            CodeIntelOperation::Structure => {
                let depth = args.depth.unwrap_or(3);
                let indices = self.indices_for_query(args.root.as_deref()).await?;
                if indices.is_empty() {
                    return Err(FunctionCallError::RespondToModel(
                        "no TreeSitter roots are configured".to_string(),
                    ));
                }
                let mut out = Vec::new();
                let mut roots = Vec::new();
                for (root, index) in indices {
                    let structure = index.structure(depth);
                    out.push(format!("[{root}]"));
                    out.push(structure.clone());
                    roots.push(json!({
                        "root": root,
                        "depth": depth,
                        "tree": structure,
                        "file_count": index.file_tree().len(),
                        "language_breakdown": index
                            .file_tree()
                            .language_breakdown()
                            .into_iter()
                            .map(|(language, count)| json!({"language": language, "count": count}))
                            .collect::<Vec<_>>()
                    }));
                }
                (out.join("\n"), json!({"roots": roots}))
            }
            CodeIntelOperation::Peek => {
                let path = absolute_file(args.file.as_deref())?;
                let (root, index, rel_file) =
                    self.index_for_file(&path, args.root.as_deref()).await?;
                let start_line = args.line.unwrap_or(1) as usize;
                let line_count = limit;
                let peek = index
                    .peek(&rel_file, start_line, line_count)
                    .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
                (
                    format!(
                        "[{root}] {}:{}-{} ({} total lines)\n{}",
                        peek.file, peek.start_line, peek.end_line, peek.total_lines, peek.content
                    ),
                    json!({"root": root, "peek": peek}),
                )
            }
            CodeIntelOperation::Grep => {
                let pattern = require_param(args.pattern.as_deref(), "pattern")?;
                let scope = GrepScope::from_input(args.scope.as_deref());
                let indices = self.indices_for_query(args.root.as_deref()).await?;
                let mut all_matches = Vec::new();
                let mut total_matches = 0usize;
                let mut truncated = false;

                for (root, index) in indices {
                    let result = index
                        .grep(pattern, scope, limit, 2)
                        .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
                    total_matches += result.total_matches;
                    truncated |= result.truncated;
                    for matched in result.matches {
                        all_matches.push((root.clone(), matched));
                        if all_matches.len() >= limit {
                            truncated = true;
                            break;
                        }
                    }
                    if all_matches.len() >= limit {
                        break;
                    }
                }

                (
                    format_grep(pattern, total_matches, truncated, &all_matches),
                    json!({
                        "pattern": pattern,
                        "total_matches": total_matches,
                        "truncated": truncated,
                        "matches": all_matches
                            .iter()
                            .map(|(root, matched)| json!({"root": root, "match": matched}))
                            .collect::<Vec<_>>()
                    }),
                )
            }
            CodeIntelOperation::ChunkIndices => {
                let path = absolute_file(args.file.as_deref())?;
                let chunk_size = args.chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE);
                let overlap = args.overlap.unwrap_or(DEFAULT_CHUNK_OVERLAP);
                let (root, index, rel_file) =
                    self.index_for_file(&path, args.root.as_deref()).await?;
                let result = index
                    .chunk_indices(&rel_file, chunk_size, overlap)
                    .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
                (
                    format_chunk_indices(&root, &result),
                    json!({"root": root, "chunk_indices": result}),
                )
            }
            CodeIntelOperation::DefineSymbol => {
                let symbol = require_param(args.symbol.as_deref(), "symbol")?;
                let path = absolute_file(args.file.as_deref())?;
                let (root, index, rel_file) =
                    self.index_for_file(&path, args.root.as_deref()).await?;
                let definition = require_param(args.definition.as_deref(), "definition")?;
                index
                    .define_symbol(symbol, &rel_file, definition, false)
                    .map_err(FunctionCallError::RespondToModel)?;
                let message = format!("Defined symbol '{symbol}' in [{root}] {rel_file}");
                (
                    message.clone(),
                    json!({"ok": true, "root": root, "file": rel_file, "symbol": symbol, "message": message}),
                )
            }
            CodeIntelOperation::RedefineSymbol => {
                let symbol = require_param(args.symbol.as_deref(), "symbol")?;
                let path = absolute_file(args.file.as_deref())?;
                let (root, index, rel_file) =
                    self.index_for_file(&path, args.root.as_deref()).await?;
                let definition = require_param(args.definition.as_deref(), "definition")?;
                index
                    .define_symbol(symbol, &rel_file, definition, true)
                    .map_err(FunctionCallError::RespondToModel)?;
                let message = format!("Redefined symbol '{symbol}' in [{root}] {rel_file}");
                (
                    message.clone(),
                    json!({"ok": true, "root": root, "file": rel_file, "symbol": symbol, "message": message}),
                )
            }
            CodeIntelOperation::DefineFile => {
                let path = absolute_file(args.file.as_deref())?;
                let (root, index, rel_file) =
                    self.index_for_file(&path, args.root.as_deref()).await?;
                let definition = require_param(args.definition.as_deref(), "definition")?;
                index
                    .define_file(&rel_file, definition, false)
                    .map_err(FunctionCallError::RespondToModel)?;
                let message = format!("Defined file [{root}] '{rel_file}'");
                (
                    message.clone(),
                    json!({"ok": true, "root": root, "file": rel_file, "message": message}),
                )
            }
            CodeIntelOperation::RedefineFile => {
                let path = absolute_file(args.file.as_deref())?;
                let (root, index, rel_file) =
                    self.index_for_file(&path, args.root.as_deref()).await?;
                let definition = require_param(args.definition.as_deref(), "definition")?;
                index
                    .define_file(&rel_file, definition, true)
                    .map_err(FunctionCallError::RespondToModel)?;
                let message = format!("Redefined file [{root}] '{rel_file}'");
                (
                    message.clone(),
                    json!({"ok": true, "root": root, "file": rel_file, "message": message}),
                )
            }
            CodeIntelOperation::MarkFile => {
                let path = absolute_file(args.file.as_deref())?;
                let (root, index, rel_file) =
                    self.index_for_file(&path, args.root.as_deref()).await?;
                let mark = require_param(args.mark.as_deref(), "mark")?;
                let mark_enum = FileMark::from_input(mark).ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "`mark` must be non-empty. Built-ins: test, docs, config, generated, entryPoint; custom values are also allowed."
                            .to_string(),
                    )
                })?;
                index
                    .mark_file(&rel_file, mark_enum)
                    .map_err(FunctionCallError::RespondToModel)?;
                let message = format!("Marked file [{root}] '{rel_file}' as {mark}");
                (
                    message.clone(),
                    json!({"ok": true, "root": root, "file": rel_file, "mark": mark, "message": message}),
                )
            }
            CodeIntelOperation::SaveAnnotations => {
                let indices = self.indices_for_query(args.root.as_deref()).await?;
                let mut results = Vec::new();
                for (root, index) in indices {
                    let stats = index
                        .save_annotations()
                        .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
                    results.push((root, stats));
                }
                (
                    format_save_annotations(&results),
                    json!({
                        "count": results.len(),
                        "results": results
                            .iter()
                            .map(|(root, stats)| json!({"root": root, "stats": stats}))
                            .collect::<Vec<_>>()
                    }),
                )
            }
            CodeIntelOperation::LoadAnnotations => {
                let indices = self.indices_for_query(args.root.as_deref()).await?;
                let mut results = Vec::new();
                for (root, index) in indices {
                    let stats = index
                        .load_annotations()
                        .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
                    results.push((root, stats));
                }
                (
                    format_load_annotations(&results),
                    json!({
                        "count": results.len(),
                        "results": results
                            .iter()
                            .map(|(root, stats)| json!({"root": root, "stats": stats}))
                            .collect::<Vec<_>>()
                    }),
                )
            }
            CodeIntelOperation::AddRoot => {
                let root = require_param(args.root.as_deref(), "root")?;
                let path = absolute_root_path(args.path.as_deref())?;
                let added = self
                    .state
                    .add_root(root.to_string(), path)
                    .await
                    .map_err(FunctionCallError::RespondToModel)?;
                let message = format!("Added root '{}' at {}", added.name, added.path.display());
                (
                    message.clone(),
                    json!({"ok": true, "root": added.name, "path": added.path, "message": message}),
                )
            }
            CodeIntelOperation::RemoveRoot => {
                let root = require_param(args.root.as_deref(), "root")?;
                self.state
                    .remove_root(root)
                    .await
                    .map_err(FunctionCallError::RespondToModel)?;
                let message = format!("Removed root '{root}'");
                (
                    message.clone(),
                    json!({"ok": true, "root": root, "message": message}),
                )
            }
            CodeIntelOperation::ListRoots => {
                let statuses = self.state.root_statuses().await;
                let text = if statuses.is_empty() {
                    "No roots registered.".to_string()
                } else {
                    let mut lines = Vec::with_capacity(statuses.len() + 1);
                    lines.push("Registered roots:".to_string());
                    for status in &statuses {
                        let mut caps = Vec::new();
                        #[cfg(feature = "lsp")]
                        if status.has_lsp {
                            caps.push("lsp");
                        }
                        #[cfg(feature = "treesitter")]
                        if status.has_treesitter {
                            caps.push("treesitter");
                        }
                        let marker = if status.is_primary { "*" } else { "-" };
                        lines.push(format!(
                            "{marker} {} -> {} [{}]",
                            status.name,
                            status.path.display(),
                            if caps.is_empty() {
                                "none".to_string()
                            } else {
                                caps.join(", ")
                            }
                        ));
                    }
                    lines.join("\n")
                };

                let statuses_json = statuses
                    .into_iter()
                    .map(|status| {
                        let mut capabilities = Vec::new();
                        #[cfg(feature = "lsp")]
                        if status.has_lsp {
                            capabilities.push("lsp");
                        }
                        #[cfg(feature = "treesitter")]
                        if status.has_treesitter {
                            capabilities.push("treesitter");
                        }
                        json!({
                            "name": status.name,
                            "path": status.path,
                            "is_primary": status.is_primary,
                            "capabilities": capabilities,
                        })
                    })
                    .collect::<Vec<_>>();

                (
                    text,
                    json!({"count": statuses_json.len(), "roots": statuses_json}),
                )
            }
        };

        let output = match response_format {
            ResponseFormat::Text => output_text,
            ResponseFormat::Json => {
                serde_json::to_string_pretty(&output_json).unwrap_or(output_text)
            }
        };

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(truncate_tool_output(&output, MAX_RESULT_BYTES)),
            success: Some(true),
        })
    }
}

impl CodeIntelToolHandler {
    async fn indices_for_query(
        &self,
        root: Option<&str>,
    ) -> Result<Vec<(String, Arc<codex_treesitter::ProjectIndex>)>, FunctionCallError> {
        let indices = self
            .state
            .treesitter_indices(root)
            .await
            .map_err(FunctionCallError::RespondToModel)?;
        if indices.is_empty() {
            return Err(FunctionCallError::RespondToModel(match root {
                Some(root) => format!("unknown root '{root}' or root has no TreeSitter index"),
                None => "no TreeSitter roots are configured".to_string(),
            }));
        }
        Ok(indices)
    }

    async fn index_for_file(
        &self,
        file: &Path,
        root: Option<&str>,
    ) -> Result<(String, Arc<codex_treesitter::ProjectIndex>, String), FunctionCallError> {
        let (root_name, index) = self
            .state
            .treesitter_index_for_file(file, root)
            .await
            .map_err(FunctionCallError::RespondToModel)?
            .ok_or_else(|| {
                if let Some(root) = root {
                    FunctionCallError::RespondToModel(format!(
                        "file '{}' is not inside root '{root}', or root has no TreeSitter index",
                        file.display()
                    ))
                } else {
                    FunctionCallError::RespondToModel(format!(
                        "no registered root contains file '{}'",
                        file.display()
                    ))
                }
            })?;

        let rel_file = index
            .rel_path_for_absolute(file)
            .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
        Ok((root_name, index, rel_file))
    }
}

fn require_param<'a>(value: Option<&'a str>, key: &str) -> Result<&'a str, FunctionCallError> {
    value.ok_or_else(|| FunctionCallError::RespondToModel(format!("`{key}` is required")))
}

fn parse_symbol_kind(value: Option<&str>) -> Result<Option<SymbolKind>, FunctionCallError> {
    let Some(value) = value else {
        return Ok(None);
    };

    let kind = match value.trim().to_ascii_lowercase().as_str() {
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "class" => SymbolKind::Class,
        "struct" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        "trait" => SymbolKind::Trait,
        "interface" => SymbolKind::Interface,
        "constant" => SymbolKind::Constant,
        "variable" => SymbolKind::Variable,
        "type" => SymbolKind::Type,
        "module" => SymbolKind::Module,
        other => {
            return Err(FunctionCallError::RespondToModel(format!(
                "unsupported symbol kind '{other}'. Valid kinds: function, method, class, struct, enum, trait, interface, constant, variable, type, module"
            )));
        }
    };

    Ok(Some(kind))
}

fn absolute_file(file: Option<&str>) -> Result<PathBuf, FunctionCallError> {
    absolute_path(file, "file")
}

fn absolute_root_path(path: Option<&str>) -> Result<PathBuf, FunctionCallError> {
    absolute_path(path, "path")
}

fn absolute_path(path_value: Option<&str>, key: &str) -> Result<PathBuf, FunctionCallError> {
    let path = require_param(path_value, key)?;
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(FunctionCallError::RespondToModel(format!(
            "`{key}` must be an absolute path"
        )));
    }
    Ok(path)
}

fn format_symbols(symbols: &[(String, codex_treesitter::Symbol)]) -> String {
    if symbols.is_empty() {
        return "No symbols found.".to_string();
    }

    let mut out = vec![format!("Found {} symbol(s):", symbols.len())];
    for (root, symbol) in symbols {
        out.push(format!(
            "[{root}] {}:{} {:?} {}",
            symbol.file, symbol.line_range.0, symbol.kind, symbol.name
        ));
        if !symbol.signature.is_empty() {
            out.push(format!("  {}", symbol.signature));
        }
    }
    out.join("\n")
}

fn format_callers(root: &str, callers: &[codex_treesitter::CallerInfo]) -> String {
    if callers.is_empty() {
        return "No callers found.".to_string();
    }

    let mut out = vec![format!("Found {} caller(s) in [{root}]:", callers.len())];
    for caller in callers {
        out.push(format!("{}:{} {}", caller.file, caller.line, caller.text));
    }
    out.join("\n")
}

fn format_tests(root: &str, tests: &[codex_treesitter::TestInfo]) -> String {
    if tests.is_empty() {
        return "No tests found.".to_string();
    }

    let mut out = vec![format!("Found {} test(s) in [{root}]:", tests.len())];
    for test in tests {
        out.push(format!("{}:{} {}", test.file, test.line, test.name));
        if !test.signature.is_empty() {
            out.push(format!("  {}", test.signature));
        }
    }
    out.join("\n")
}

fn format_variables(
    root: &str,
    function: &str,
    variables: &[codex_treesitter::VariableInfo],
) -> String {
    if variables.is_empty() {
        return format!("No local variables found in '{function}' for root [{root}].");
    }

    let mut out = vec![format!(
        "Found {} variable(s) in '{function}' for root [{root}]:",
        variables.len(),
    )];
    for variable in variables {
        out.push(variable.name.clone());
    }
    out.join("\n")
}

fn format_grep(
    pattern: &str,
    total_matches: usize,
    truncated: bool,
    matches: &[(String, codex_treesitter::GrepMatch)],
) -> String {
    if matches.is_empty() {
        return format!("No matches found for '{pattern}'.");
    }

    let mut out = vec![format!(
        "Found {} match(es) for '{}'{}:",
        total_matches,
        pattern,
        if truncated { " (truncated)" } else { "" }
    )];

    for (root, matched) in matches {
        out.push(format!(
            "[{root}] {}:{} {}",
            matched.file, matched.line, matched.text
        ));
    }

    out.join("\n")
}

fn format_chunk_indices(root: &str, result: &codex_treesitter::ChunkIndicesResult) -> String {
    if result.chunks.is_empty() {
        return format!(
            "[{root}] {} has no chunks ({} bytes total).",
            result.file, result.total_bytes
        );
    }

    let mut out = vec![format!(
        "[{root}] {}: {} chunk(s), chunk_size={}, overlap={}, total_bytes={}",
        result.file,
        result.chunks.len(),
        result.chunk_size,
        result.overlap,
        result.total_bytes
    )];

    for chunk in &result.chunks {
        out.push(format!(
            "  #{} [{}..{})",
            chunk.index, chunk.start, chunk.end
        ));
    }

    out.join("\n")
}

fn format_save_annotations(results: &[(String, codex_treesitter::AnnotationSaveStats)]) -> String {
    if results.is_empty() {
        return "No TreeSitter roots are configured.".to_string();
    }

    let mut out = vec![format!("Saved annotations for {} root(s):", results.len())];
    for (root, stats) in results {
        out.push(format!(
            "[{root}] persisted={} path={} file_definitions={} file_marks={} symbol_definitions={}",
            stats.persisted,
            stats.path,
            stats.file_definitions,
            stats.file_marks,
            stats.symbol_definitions
        ));
    }
    out.join("\n")
}

fn format_load_annotations(results: &[(String, codex_treesitter::AnnotationLoadStats)]) -> String {
    if results.is_empty() {
        return "No TreeSitter roots are configured.".to_string();
    }

    let mut out = vec![format!("Loaded annotations for {} root(s):", results.len())];
    for (root, stats) in results {
        out.push(format!(
            "[{root}] loaded={} path={} file_definitions_applied={} file_marks_applied={} symbol_definitions_applied={}",
            stats.loaded,
            stats.path,
            stats.file_definitions_applied,
            stats.file_marks_applied,
            stats.symbol_definitions_applied
        ));
    }
    out.join("\n")
}

pub(crate) fn create_code_intel_tool() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "operation".to_string(),
        JsonSchema::String {
            description: Some(
                "Operation name: symbolSearch, symbols, callers, tests, variables, implementation, structure, peek, grep, chunkIndices, defineSymbol, redefineSymbol, defineFile, redefineFile, markFile, saveAnnotations, loadAnnotations, addRoot, removeRoot, listRoots"
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "symbol".to_string(),
        JsonSchema::String {
            description: Some("Symbol/function name for symbol-scoped operations.".to_string()),
        },
    );
    properties.insert(
        "file".to_string(),
        JsonSchema::String {
            description: Some("Absolute file path for file-scoped operations.".to_string()),
        },
    );
    properties.insert(
        "query".to_string(),
        JsonSchema::String {
            description: Some("Symbol search query for symbolSearch.".to_string()),
        },
    );
    properties.insert(
        "kind".to_string(),
        JsonSchema::String {
            description: Some("Optional symbol kind filter for symbols operation.".to_string()),
        },
    );
    properties.insert(
        "pattern".to_string(),
        JsonSchema::String {
            description: Some("Regex pattern for grep.".to_string()),
        },
    );
    properties.insert(
        "definition".to_string(),
        JsonSchema::String {
            description: Some("Definition text for define/redefine operations.".to_string()),
        },
    );
    properties.insert(
        "mark".to_string(),
        JsonSchema::String {
            description: Some(
                "Mark for markFile. Built-ins: test, docs, config, generated, entryPoint. Custom values are allowed.".to_string(),
            ),
        },
    );
    properties.insert(
        "line".to_string(),
        JsonSchema::Number {
            description: Some("1-based start line for peek.".to_string()),
        },
    );
    properties.insert(
        "limit".to_string(),
        JsonSchema::Number {
            description: Some("Result count limit (default 20, max 50).".to_string()),
        },
    );
    properties.insert(
        "depth".to_string(),
        JsonSchema::Number {
            description: Some("Depth for structure tree (default 3).".to_string()),
        },
    );
    properties.insert(
        "chunk_size".to_string(),
        JsonSchema::Number {
            description: Some("Chunk size in bytes for chunkIndices (default 5000).".to_string()),
        },
    );
    properties.insert(
        "overlap".to_string(),
        JsonSchema::Number {
            description: Some("Chunk overlap in bytes for chunkIndices (default 200).".to_string()),
        },
    );
    properties.insert(
        "scope".to_string(),
        JsonSchema::String {
            description: Some("Grep scope: all or code.".to_string()),
        },
    );
    properties.insert(
        "root".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional root name for query scoping. Required by removeRoot; used by addRoot as the new root name."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "path".to_string(),
        JsonSchema::String {
            description: Some("Absolute directory path for addRoot.".to_string()),
        },
    );
    properties.insert(
        "response_format".to_string(),
        JsonSchema::String {
            description: Some("Output format: text (default) or json.".to_string()),
        },
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "code_intel".to_string(),
        description: CODE_INTEL_TOOL_DESCRIPTION.to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["operation".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}
