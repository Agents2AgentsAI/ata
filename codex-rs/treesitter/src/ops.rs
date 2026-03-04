use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use rayon::prelude::*;
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

fn bounded_range(byte_range: (usize, usize), source_len: usize) -> Option<(usize, usize)> {
    if byte_range.0 > source_len {
        return None;
    }
    let end = byte_range.1.min(source_len);
    if byte_range.0 > end {
        return None;
    }
    Some((byte_range.0, end))
}

fn source_slice(source: &str, byte_range: (usize, usize)) -> Option<&str> {
    let (start, end) = bounded_range(byte_range, source.len())?;
    source.get(start..end)
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

    let implementation = source_slice(&source, symbol.byte_range).ok_or_else(|| {
        format!(
            "invalid byte range {}..{} for '{}' in '{}'",
            symbol.byte_range.0, symbol.byte_range.1, symbol.name, symbol.file
        )
    })?;
    Ok(implementation.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct CallerInfo {
    pub file: String,
    pub line: usize,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallersResult {
    pub callers: Vec<CallerInfo>,
    pub total_callers: usize,
    pub truncated: bool,
}

pub fn find_callers(
    root: &Path,
    file_tree: &FileTree,
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
    limit: usize,
) -> Result<CallersResult, String> {
    let _ = symbol_table
        .get(file, symbol_name)
        .ok_or_else(|| format!("symbol '{symbol_name}' not found in '{file}'"))?;

    let mut callers: Vec<CallerInfo> = file_tree
        .all_paths_with_language()
        .into_par_iter()
        .flat_map_iter(|(rel_path, language)| {
            // Skip non-code files (e.g. LICENSE, .md) that have no tree-sitter
            // support — regex fallback on these produces too many false positives.
            if !language.has_tree_sitter_support() {
                return Vec::new();
            }

            let source = match std::fs::read_to_string(root.join(&rel_path)) {
                Ok(source) => source,
                Err(_) => return Vec::new(),
            };

            find_callers_ast(&source, &rel_path, language, symbol_name, file)
        })
        .collect();

    callers.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    let total_callers = callers.len();
    let truncated = total_callers > limit;
    if truncated {
        callers.truncate(limit);
    }

    Ok(CallersResult {
        callers,
        total_callers,
        truncated,
    })
}

fn find_callers_ast(
    source: &str,
    rel_path: &str,
    language: Language,
    symbol_name: &str,
    definition_file: &str,
) -> Vec<CallerInfo> {
    let lines: Vec<&str> = source.lines().collect();
    let Some(callers) = with_parsed_query(
        source,
        language,
        QueryKind::Callers,
        |tree, query, capture_names| {
            let callee_index = capture_names.iter().position(|name| name == "callee");
            let qualifier_index = capture_names.iter().position(|name| name == "qualifier");
            let mut callers = Vec::new();
            let mut cursor = tree_sitter::QueryCursor::new();
            let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());

            while let Some(match_) = matches.next() {
                for capture in match_.captures {
                    if Some(capture.index as usize) != callee_index {
                        continue;
                    }

                    let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
                    if text != symbol_name {
                        continue;
                    }

                    let qualifier_text = qualifier_index.and_then(|qi| {
                        match_
                            .captures
                            .iter()
                            .find(|c| c.index as usize == qi)
                            .and_then(|c| c.node.utf8_text(source.as_bytes()).ok())
                            .map(|s| s.to_string())
                    });

                    let line = capture.node.start_position().row + 1;
                    let line_text = lines
                        .get(line.saturating_sub(1))
                        .copied()
                        .unwrap_or("")
                        .trim()
                        .to_string();

                    if rel_path == definition_file
                        && is_definition_line(&line_text, symbol_name, language)
                    {
                        continue;
                    }

                    callers.push(CallerInfo {
                        file: rel_path.to_string(),
                        line,
                        text: line_text,
                        qualifier: qualifier_text,
                    });
                }
            }

            callers
        },
    ) else {
        return find_callers_regex(source, rel_path, language, symbol_name, definition_file);
    };
    callers
}

fn find_callers_regex(
    source: &str,
    rel_path: &str,
    language: Language,
    symbol_name: &str,
    definition_file: &str,
) -> Vec<CallerInfo> {
    let pattern = match regex::Regex::new(&format!(r"\b{}\b", regex::escape(symbol_name))) {
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
            qualifier: None,
        });
    }

    callers
}

fn is_definition_line(line: &str, name: &str, language: Language) -> bool {
    queries::is_definition_line(language, line, name)
}

#[derive(Clone, Copy)]
enum QueryKind {
    Callers,
    Variables,
}

struct OpsQueryCache {
    parser: tree_sitter::Parser,
    callers_query: tree_sitter::Query,
    callers_capture_names: Vec<String>,
    variables_query: tree_sitter::Query,
    variables_capture_names: Vec<String>,
}

thread_local! {
    static OPS_QUERY_CACHE: RefCell<HashMap<Language, OpsQueryCache>> = RefCell::new(HashMap::new());
    static VARIABLE_REGEX_CACHE: RefCell<HashMap<Language, Vec<CompiledVariablePattern>>> =
        RefCell::new(HashMap::new());
}

struct CompiledVariablePattern {
    capture_group: usize,
    regex: regex::Regex,
}

fn with_parsed_query<R>(
    source: &str,
    language: Language,
    query_kind: QueryKind,
    handler: impl FnOnce(tree_sitter::Tree, &tree_sitter::Query, &[String]) -> R,
) -> Option<R> {
    OPS_QUERY_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(&language) {
            let config = queries::get_language_config(language)?;
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&config.language).ok()?;

            let callers_query =
                tree_sitter::Query::new(&config.language, config.callers_query).ok()?;
            let callers_capture_names = callers_query
                .capture_names()
                .iter()
                .map(ToString::to_string)
                .collect();

            let variables_query =
                tree_sitter::Query::new(&config.language, config.variables_query).ok()?;
            let variables_capture_names = variables_query
                .capture_names()
                .iter()
                .map(ToString::to_string)
                .collect();

            cache.insert(
                language,
                OpsQueryCache {
                    parser,
                    callers_query,
                    callers_capture_names,
                    variables_query,
                    variables_capture_names,
                },
            );
        }

        let entry = cache.get_mut(&language)?;
        let tree = entry.parser.parse(source, None)?;
        let (query, capture_names) = match query_kind {
            QueryKind::Callers => (&entry.callers_query, entry.callers_capture_names.as_slice()),
            QueryKind::Variables => (
                &entry.variables_query,
                entry.variables_capture_names.as_slice(),
            ),
        };
        Some(handler(tree, query, capture_names))
    })
}

fn with_compiled_variable_patterns<R>(
    language: Language,
    handler: impl FnOnce(&[CompiledVariablePattern], fn(&str) -> bool) -> R,
) -> Option<R> {
    let config = queries::get_language_config(language)?;
    VARIABLE_REGEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(&language) {
            let mut compiled = Vec::with_capacity(config.variable_regex_patterns.len());
            for pattern in config.variable_regex_patterns {
                let regex = regex::Regex::new(pattern.regex).ok()?;
                compiled.push(CompiledVariablePattern {
                    capture_group: pattern.capture_group,
                    regex,
                });
            }
            cache.insert(language, compiled);
        }
        let compiled = cache.get(&language)?;
        Some(handler(compiled, config.variable_name_filter))
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct TestInfo {
    pub name: String,
    pub file: String,
    pub line: usize,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestsResult {
    pub tests: Vec<TestInfo>,
    pub total_tests: usize,
    pub truncated: bool,
}

pub fn find_tests(
    root: &Path,
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
    limit: usize,
) -> Result<TestsResult, String> {
    let _ = symbol_table
        .get(file, symbol_name)
        .ok_or_else(|| format!("symbol '{symbol_name}' not found in '{file}'"))?;

    let mut tests: Vec<TestInfo> = symbol_table
        .test_symbols()
        .into_par_iter()
        .filter_map(|symbol| {
            let source = std::fs::read_to_string(root.join(&symbol.file)).ok()?;
            let body = source_slice(&source, symbol.byte_range)?;
            if !body.contains(symbol_name) {
                return None;
            }

            Some(TestInfo {
                name: symbol.name,
                file: symbol.file,
                line: symbol.line_range.0,
                signature: symbol.signature,
            })
        })
        .collect();

    tests.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    let total_tests = tests.len();
    let truncated = total_tests > limit;
    if truncated {
        tests.truncate(limit);
    }

    Ok(TestsResult {
        tests,
        total_tests,
        truncated,
    })
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

    let (start, end) = bounded_range(symbol.byte_range, source.len()).ok_or_else(|| {
        format!(
            "invalid byte range {}..{} for '{}' in '{}'",
            symbol.byte_range.0, symbol.byte_range.1, symbol.name, symbol.file
        )
    })?;
    let body = source_slice(&source, symbol.byte_range).ok_or_else(|| {
        format!(
            "invalid UTF-8 slice {}..{} for '{}' in '{}'",
            symbol.byte_range.0, symbol.byte_range.1, symbol.name, symbol.file
        )
    })?;

    if symbol.language.has_tree_sitter_support() {
        return Ok(list_variables_ast(
            &source,
            symbol.language,
            start,
            end,
            function_name,
        ));
    }

    Ok(list_variables_regex(body, symbol.language, function_name))
}

fn list_variables_ast(
    source: &str,
    language: Language,
    function_start: usize,
    function_end: usize,
    function_name: &str,
) -> Vec<VariableInfo> {
    let body = source.get(function_start..function_end).unwrap_or("");
    let Some(variables) = with_parsed_query(
        source,
        language,
        QueryKind::Variables,
        |tree, query, capture_names| {
            let variable_capture_index = capture_names.iter().position(|name| name == "var.name");

            let mut variables = Vec::new();
            let mut seen = HashSet::new();
            let mut cursor = tree_sitter::QueryCursor::new();
            cursor.set_byte_range(function_start..function_end);

            let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
            while let Some(match_) = matches.next() {
                for capture in match_.captures {
                    if Some(capture.index as usize) != variable_capture_index {
                        continue;
                    }

                    let name = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
                    if name.is_empty()
                        || name == "self"
                        || name == "_"
                        || !seen.insert(name.to_string())
                    {
                        continue;
                    }

                    variables.push(VariableInfo {
                        name: name.to_string(),
                        function: function_name.to_string(),
                    });
                }
            }

            variables
        },
    ) else {
        return list_variables_regex(body, language, function_name);
    };
    variables
}

fn list_variables_regex(body: &str, language: Language, function_name: &str) -> Vec<VariableInfo> {
    let Some(mut variables) =
        with_compiled_variable_patterns(language, |patterns, variable_name_filter| {
            let mut variables = Vec::new();
            for pattern in patterns {
                for captures in pattern.regex.captures_iter(body) {
                    let Some(name) = captures.get(pattern.capture_group).map(|m| m.as_str()) else {
                        continue;
                    };
                    if !variable_name_filter(name) {
                        continue;
                    }
                    variables.push(VariableInfo {
                        name: name.to_string(),
                        function: function_name.to_string(),
                    });
                }
            }
            variables
        })
    else {
        return Vec::new();
    };

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
