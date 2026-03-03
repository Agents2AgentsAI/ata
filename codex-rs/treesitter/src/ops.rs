use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use tree_sitter::StreamingIterator;

use crate::file_entry::Language;
use crate::file_tree::FileTree;
use crate::queries;
use crate::symbol::Symbol;
use crate::symbol::SymbolKind;
use crate::symbol_table::SymbolTable;

pub fn search_symbols(symbol_table: &SymbolTable, query: &str, limit: usize) -> Vec<Symbol> {
    symbol_table.search(query, limit)
}

pub fn get_implementation(
    root: &Path,
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
) -> Result<String, String> {
    let symbol = symbol_table
        .get(file, symbol_name)
        .ok_or_else(|| format!("symbol '{symbol_name}' not found in '{file}'"))?;

    let source = std::fs::read_to_string(root.join(&symbol.file))
        .map_err(|error| format!("failed to read '{}': {error}", symbol.file))?;

    let end = symbol.byte_range.1.min(source.len());
    Ok(source[symbol.byte_range.0..end].to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct CallerInfo {
    pub file: String,
    pub line: usize,
    pub text: String,
}

pub fn find_callers(
    root: &Path,
    file_tree: &FileTree,
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
    limit: usize,
) -> Result<Vec<CallerInfo>, String> {
    let _ = symbol_table
        .get(file, symbol_name)
        .ok_or_else(|| format!("symbol '{symbol_name}' not found in '{file}'"))?;

    let mut callers = Vec::new();

    for (rel_path, language) in file_tree.all_paths_with_language() {
        let source = match std::fs::read_to_string(root.join(&rel_path)) {
            Ok(source) => source,
            Err(_) => continue,
        };

        let file_callers = if language.has_tree_sitter_support() {
            find_callers_ast(&source, &rel_path, language, symbol_name, file)
        } else {
            find_callers_regex(&source, &rel_path, language, symbol_name, file)
        };

        for caller in file_callers {
            callers.push(caller);
            if callers.len() >= limit {
                return Ok(callers);
            }
        }
    }

    Ok(callers)
}

fn find_callers_ast(
    source: &str,
    rel_path: &str,
    language: Language,
    symbol_name: &str,
    definition_file: &str,
) -> Vec<CallerInfo> {
    let Some((tree, query)) = try_parse_with_query(source, language, QueryKind::Callers) else {
        return find_callers_regex(source, rel_path, language, symbol_name, definition_file);
    };

    let capture_names: Vec<String> = query
        .capture_names()
        .iter()
        .map(ToString::to_string)
        .collect();
    let callee_index = capture_names.iter().position(|name| name == "callee");

    let mut callers = Vec::new();
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            if Some(capture.index as usize) != callee_index {
                continue;
            }

            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
            if text != symbol_name {
                continue;
            }

            let line = capture.node.start_position().row + 1;
            if rel_path == definition_file
                && source
                    .lines()
                    .nth(line - 1)
                    .is_some_and(|line_text| is_definition_line(line_text, symbol_name, language))
            {
                continue;
            }

            let line_text = source
                .lines()
                .nth(line - 1)
                .map(|line_text| line_text.trim().to_string())
                .unwrap_or_default();

            callers.push(CallerInfo {
                file: rel_path.to_string(),
                line,
                text: line_text,
            });
        }
    }

    callers
}

fn find_callers_regex(
    source: &str,
    rel_path: &str,
    language: Language,
    symbol_name: &str,
    definition_file: &str,
) -> Vec<CallerInfo> {
    let pattern = match regex::Regex::new(&regex::escape(symbol_name)) {
        Ok(pattern) => pattern,
        Err(_) => return Vec::new(),
    };

    let mut callers = Vec::new();

    for (line_idx, line) in source.lines().enumerate() {
        if !pattern.is_match(line) {
            continue;
        }

        if rel_path == definition_file && is_definition_line(line, symbol_name, language) {
            continue;
        }

        callers.push(CallerInfo {
            file: rel_path.to_string(),
            line: line_idx + 1,
            text: line.trim().to_string(),
        });
    }

    callers
}

fn is_definition_line(line: &str, name: &str, language: Language) -> bool {
    match language {
        Language::Rust => line.contains(&format!("fn {name}")),
        Language::Python => line.contains(&format!("def {name}")),
        Language::TypeScript | Language::JavaScript => {
            line.contains(&format!("function {name}")) || line.contains(&format!("{name} ="))
        }
        Language::Go => line.contains(&format!("func {name}")),
        Language::Java => {
            line.contains(&format!("class {name}"))
                || line.contains(&format!("interface {name}"))
                || line.contains(&format!("enum {name}"))
                || (line.contains(name)
                    && (line.contains("void ")
                        || line.contains("int ")
                        || line.contains("String ")
                        || line.contains("boolean ")
                        || line.contains("long ")
                        || line.contains("double ")
                        || line.contains("float ")
                        || line.contains("public ")
                        || line.contains("private ")
                        || line.contains("protected ")))
        }
        Language::Scala => {
            line.contains(&format!("def {name}"))
                || line.contains(&format!("object {name}"))
                || line.contains(&format!("class {name}"))
                || line.contains(&format!("trait {name}"))
        }
        Language::Other => false,
    }
}

#[derive(Clone, Copy)]
enum QueryKind {
    Callers,
    Variables,
}

fn try_parse_with_query(
    source: &str,
    language: Language,
    query_kind: QueryKind,
) -> Option<(tree_sitter::Tree, tree_sitter::Query)> {
    let config = queries::get_language_config(language)?;
    let query_str = match query_kind {
        QueryKind::Callers => config.callers_query,
        QueryKind::Variables => config.variables_query,
    };

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&config.language).ok()?;

    let tree = parser.parse(source, None)?;
    let query = tree_sitter::Query::new(&config.language, query_str).ok()?;

    Some((tree, query))
}

#[derive(Debug, Clone, Serialize)]
pub struct TestInfo {
    pub name: String,
    pub file: String,
    pub line: usize,
    pub signature: String,
}

pub fn find_tests(
    root: &Path,
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
    limit: usize,
) -> Result<Vec<TestInfo>, String> {
    let _ = symbol_table
        .get(file, symbol_name)
        .ok_or_else(|| format!("symbol '{symbol_name}' not found in '{file}'"))?;

    let mut tests = Vec::new();

    for symbol in symbol_table.test_symbols() {
        let source = match std::fs::read_to_string(root.join(&symbol.file)) {
            Ok(source) => source,
            Err(_) => continue,
        };

        let end = symbol.byte_range.1.min(source.len());
        let body = &source[symbol.byte_range.0..end];
        if !body.contains(symbol_name) {
            continue;
        }

        tests.push(TestInfo {
            name: symbol.name,
            file: symbol.file,
            line: symbol.line_range.0,
            signature: symbol.signature,
        });

        if tests.len() >= limit {
            break;
        }
    }

    Ok(tests)
}

#[derive(Debug, Clone, Serialize)]
pub struct VariableInfo {
    pub name: String,
    pub function: String,
}

pub fn list_variables(
    root: &Path,
    symbol_table: &SymbolTable,
    function_name: &str,
    file: &str,
) -> Result<Vec<VariableInfo>, String> {
    let symbol = symbol_table
        .get(file, function_name)
        .ok_or_else(|| format!("symbol '{function_name}' not found in '{file}'"))?;

    let source = std::fs::read_to_string(root.join(&symbol.file))
        .map_err(|error| format!("failed to read '{}': {error}", symbol.file))?;

    let end = symbol.byte_range.1.min(source.len());
    if symbol.language.has_tree_sitter_support() {
        return Ok(list_variables_ast(
            &source,
            symbol.language,
            symbol.byte_range.0,
            end,
            function_name,
        ));
    }

    Ok(list_variables_regex(
        &source[symbol.byte_range.0..end],
        symbol.language,
        function_name,
    ))
}

fn list_variables_ast(
    source: &str,
    language: Language,
    function_start: usize,
    function_end: usize,
    function_name: &str,
) -> Vec<VariableInfo> {
    let Some((tree, query)) = try_parse_with_query(source, language, QueryKind::Variables) else {
        return list_variables_regex(
            &source[function_start..function_end],
            language,
            function_name,
        );
    };

    let capture_names: Vec<String> = query
        .capture_names()
        .iter()
        .map(ToString::to_string)
        .collect();
    let variable_capture_index = capture_names.iter().position(|name| name == "var.name");

    let mut variables = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = tree_sitter::QueryCursor::new();
    cursor.set_byte_range(function_start..function_end);

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            if Some(capture.index as usize) != variable_capture_index {
                continue;
            }

            let name = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
            if name.is_empty() || name == "self" || name == "_" || !seen.insert(name.to_string()) {
                continue;
            }

            variables.push(VariableInfo {
                name: name.to_string(),
                function: function_name.to_string(),
            });
        }
    }

    variables
}

fn list_variables_regex(body: &str, language: Language, function_name: &str) -> Vec<VariableInfo> {
    let mut variables = Vec::new();

    match language {
        Language::Rust => {
            if let Ok(pattern) = regex::Regex::new(r"let\s+(mut\s+)?(\w+)") {
                for captures in pattern.captures_iter(body) {
                    variables.push(VariableInfo {
                        name: captures[2].to_string(),
                        function: function_name.to_string(),
                    });
                }
            }
        }
        Language::Python => {
            if let Ok(pattern) = regex::Regex::new(r"^\s+(\w+)\s*=") {
                for captures in pattern.captures_iter(body) {
                    let name = captures[1].to_string();
                    if name != "self" && !name.starts_with('_') {
                        variables.push(VariableInfo {
                            name,
                            function: function_name.to_string(),
                        });
                    }
                }
            }
        }
        Language::TypeScript | Language::JavaScript => {
            if let Ok(pattern) = regex::Regex::new(r"(?:let|const|var)\s+(\w+)") {
                for captures in pattern.captures_iter(body) {
                    variables.push(VariableInfo {
                        name: captures[1].to_string(),
                        function: function_name.to_string(),
                    });
                }
            }
        }
        Language::Go => {
            if let Ok(short_pattern) = regex::Regex::new(r"(\w+)\s*:=") {
                for captures in short_pattern.captures_iter(body) {
                    variables.push(VariableInfo {
                        name: captures[1].to_string(),
                        function: function_name.to_string(),
                    });
                }
            }
            if let Ok(var_pattern) = regex::Regex::new(r"var\s+(\w+)") {
                for captures in var_pattern.captures_iter(body) {
                    variables.push(VariableInfo {
                        name: captures[1].to_string(),
                        function: function_name.to_string(),
                    });
                }
            }
        }
        Language::Java => {
            if let Ok(pattern) = regex::Regex::new(
                r"\b(?:int|long|float|double|boolean|char|byte|short|String|var|final\s+\w+)\s+(\w+)\s*[=;,)]",
            ) {
                for captures in pattern.captures_iter(body) {
                    variables.push(VariableInfo {
                        name: captures[1].to_string(),
                        function: function_name.to_string(),
                    });
                }
            }
        }
        Language::Scala => {
            if let Ok(pattern) = regex::Regex::new(r"\b(?:val|var)\s+(\w+)") {
                for captures in pattern.captures_iter(body) {
                    variables.push(VariableInfo {
                        name: captures[1].to_string(),
                        function: function_name.to_string(),
                    });
                }
            }
        }
        Language::Other => {}
    }

    variables.sort_by(|a, b| a.name.cmp(&b.name));
    variables.dedup_by(|a, b| a.name == b.name);
    variables
}

pub fn list_symbols(
    symbol_table: &SymbolTable,
    kind: Option<SymbolKind>,
    file: Option<&str>,
    limit: usize,
) -> Vec<Symbol> {
    let mut symbols = if let Some(file) = file {
        symbol_table.symbols_in_file(file)
    } else {
        symbol_table.all_symbols()
    };

    if let Some(kind) = kind {
        symbols.retain(|symbol| symbol.kind == kind);
    }

    symbols.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line_range.0.cmp(&b.line_range.0))
    });

    if symbols.len() > limit {
        symbols.truncate(limit);
    }
    symbols
}
