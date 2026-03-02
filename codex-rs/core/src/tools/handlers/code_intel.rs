use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use codex_protocol::models::FunctionCallOutputBody;
use codex_treesitter::FileMark;
use codex_treesitter::GrepScope;
use codex_treesitter::ProjectIndex;
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

const CODE_INTEL_TOOL_DESCRIPTION: &str = include_str!("tool_code_intel.txt");
const DEFAULT_LIMIT: usize = 20;
const MAX_RESULTS: usize = 50;
const MAX_RESULT_BYTES: usize = 8 * 1024;

pub struct CodeIntelToolHandler {
    pub index: Arc<ProjectIndex>,
}

#[derive(Debug, Deserialize)]
struct CodeIntelToolArgs {
    operation: String,
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
    line: Option<u32>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    depth: Option<usize>,
    #[serde(default)]
    scope: Option<String>,
}

#[async_trait]
impl ToolHandler for CodeIntelToolHandler {
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
                    "code_intel handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: CodeIntelToolArgs = parse_arguments(&arguments)?;
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_RESULTS);

        let output = match args.operation.as_str() {
            "symbolSearch" => {
                let query = require_param(args.query.as_deref(), "query")?;
                let symbols = self.index.search_symbols(query, limit);
                format_symbols(&symbols)
            }
            "callers" => {
                let symbol = require_param(args.symbol.as_deref(), "symbol")?;
                let rel_file = self.relative_file(args.file.as_deref())?;
                let callers = self
                    .index
                    .find_callers(symbol, &rel_file, limit)
                    .map_err(FunctionCallError::RespondToModel)?;
                format_callers(&callers)
            }
            "tests" => {
                let symbol = require_param(args.symbol.as_deref(), "symbol")?;
                let rel_file = self.relative_file(args.file.as_deref())?;
                let tests = self
                    .index
                    .find_tests(symbol, &rel_file, limit)
                    .map_err(FunctionCallError::RespondToModel)?;
                format_tests(&tests)
            }
            "variables" => {
                let function = require_param(args.symbol.as_deref(), "symbol")?;
                let rel_file = self.relative_file(args.file.as_deref())?;
                let variables = self
                    .index
                    .list_variables(function, &rel_file)
                    .map_err(FunctionCallError::RespondToModel)?;
                format_variables(function, &variables)
            }
            "implementation" => {
                let symbol = require_param(args.symbol.as_deref(), "symbol")?;
                let rel_file = self.relative_file(args.file.as_deref())?;
                self.index
                    .implementation(symbol, &rel_file)
                    .map_err(FunctionCallError::RespondToModel)?
            }
            "structure" => {
                let depth = args.depth.unwrap_or(3);
                self.index.structure(depth)
            }
            "peek" => {
                let rel_file = self.relative_file(args.file.as_deref())?;
                let start_line = args.line.unwrap_or(1) as usize;
                let line_count = limit;
                let peek = self
                    .index
                    .peek(&rel_file, start_line, line_count)
                    .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
                format!(
                    "{}:{}-{} ({} total lines)\n{}",
                    peek.file, peek.start_line, peek.end_line, peek.total_lines, peek.content
                )
            }
            "grep" => {
                let pattern = require_param(args.pattern.as_deref(), "pattern")?;
                let scope = GrepScope::from_input(args.scope.as_deref());
                let result = self
                    .index
                    .grep(pattern, scope, limit, 2)
                    .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
                format_grep(&result)
            }
            "defineSymbol" => {
                let symbol = require_param(args.symbol.as_deref(), "symbol")?;
                let rel_file = self.relative_file(args.file.as_deref())?;
                let definition = require_param(args.definition.as_deref(), "definition")?;
                self.index
                    .define_symbol(symbol, &rel_file, definition, false)
                    .map_err(FunctionCallError::RespondToModel)?;
                format!("Defined symbol '{symbol}' in {rel_file}")
            }
            "defineFile" => {
                let rel_file = self.relative_file(args.file.as_deref())?;
                let definition = require_param(args.definition.as_deref(), "definition")?;
                self.index
                    .define_file(&rel_file, definition, false)
                    .map_err(FunctionCallError::RespondToModel)?;
                format!("Defined file '{rel_file}'")
            }
            "markFile" => {
                let rel_file = self.relative_file(args.file.as_deref())?;
                let mark = require_param(args.mark.as_deref(), "mark")?;
                let mark_enum = FileMark::from_input(mark).ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "`mark` must be non-empty. Built-ins: test, docs, config, generated, entryPoint; custom values are also allowed."
                            .to_string(),
                    )
                })?;
                self.index
                    .mark_file(&rel_file, mark_enum)
                    .map_err(FunctionCallError::RespondToModel)?;
                format!("Marked file '{rel_file}' as {mark}")
            }
            other => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "unknown code_intel operation: {other}. Valid operations: symbolSearch, callers, tests, variables, implementation, structure, peek, grep, defineSymbol, defineFile, markFile"
                )));
            }
        };

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(truncate_output(&output)),
            success: Some(true),
        })
    }
}

impl CodeIntelToolHandler {
    fn relative_file(&self, file: Option<&str>) -> Result<String, FunctionCallError> {
        let file = require_param(file, "file")?;
        let path = PathBuf::from(file);
        if !path.is_absolute() {
            return Err(FunctionCallError::RespondToModel(
                "`file` must be an absolute path".to_string(),
            ));
        }

        self.index
            .rel_path_for_absolute(&path)
            .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))
    }
}

fn require_param<'a>(value: Option<&'a str>, key: &str) -> Result<&'a str, FunctionCallError> {
    value.ok_or_else(|| FunctionCallError::RespondToModel(format!("`{key}` is required")))
}

fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_RESULT_BYTES {
        return output.to_string();
    }

    let prefix = &output[..MAX_RESULT_BYTES];
    let cut = prefix.rfind('\n').unwrap_or(MAX_RESULT_BYTES);
    format!("{}\n\n... truncated", &output[..cut])
}

fn format_symbols(symbols: &[codex_treesitter::Symbol]) -> String {
    if symbols.is_empty() {
        return "No symbols found.".to_string();
    }

    let mut out = vec![format!("Found {} symbol(s):", symbols.len())];
    for symbol in symbols {
        out.push(format!(
            "{}:{} {:?} {}",
            symbol.file, symbol.line_range.0, symbol.kind, symbol.name
        ));
        if !symbol.signature.is_empty() {
            out.push(format!("  {}", symbol.signature));
        }
    }
    out.join("\n")
}

fn format_callers(callers: &[codex_treesitter::CallerInfo]) -> String {
    if callers.is_empty() {
        return "No callers found.".to_string();
    }

    let mut out = vec![format!("Found {} caller(s):", callers.len())];
    for caller in callers {
        out.push(format!("{}:{} {}", caller.file, caller.line, caller.text));
    }
    out.join("\n")
}

fn format_tests(tests: &[codex_treesitter::TestInfo]) -> String {
    if tests.is_empty() {
        return "No tests found.".to_string();
    }

    let mut out = vec![format!("Found {} test(s):", tests.len())];
    for test in tests {
        out.push(format!("{}:{} {}", test.file, test.line, test.name));
        if !test.signature.is_empty() {
            out.push(format!("  {}", test.signature));
        }
    }
    out.join("\n")
}

fn format_variables(function: &str, variables: &[codex_treesitter::VariableInfo]) -> String {
    if variables.is_empty() {
        return format!("No local variables found in '{function}'.");
    }

    let mut out = vec![format!(
        "Found {} variable(s) in '{function}':",
        variables.len()
    )];
    for variable in variables {
        out.push(variable.name.clone());
    }
    out.join("\n")
}

fn format_grep(result: &codex_treesitter::GrepResult) -> String {
    if result.matches.is_empty() {
        return format!("No matches found for '{}'.", result.pattern);
    }

    let mut out = vec![format!(
        "Found {} match(es) for '{}'{}:",
        result.total_matches,
        result.pattern,
        if result.truncated { " (truncated)" } else { "" }
    )];

    for matched in &result.matches {
        out.push(format!(
            "{}:{} {}",
            matched.file, matched.line, matched.text
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
                "Operation name: symbolSearch, callers, tests, variables, implementation, structure, peek, grep, defineSymbol, defineFile, markFile"
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
        "pattern".to_string(),
        JsonSchema::String {
            description: Some("Regex pattern for grep.".to_string()),
        },
    );
    properties.insert(
        "definition".to_string(),
        JsonSchema::String {
            description: Some("Definition text for defineSymbol/defineFile.".to_string()),
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
        "scope".to_string(),
        JsonSchema::String {
            description: Some("Grep scope: all or code.".to_string()),
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
