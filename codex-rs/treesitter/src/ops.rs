use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use dashmap::DashMap;
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

fn load_file_cached(
    root: &Path,
    rel_file: &str,
    cache: Option<&Arc<DashMap<String, Arc<String>>>>,
) -> std::io::Result<Arc<String>> {
    if let Some(cache) = cache {
        if let Some(existing) = cache.get(rel_file) {
            return Ok(Arc::clone(existing.value()));
        }
        let content = Arc::new(std::fs::read_to_string(root.join(rel_file))?);
        cache.insert(rel_file.to_string(), Arc::clone(&content));
        return Ok(content);
    }
    Ok(Arc::new(std::fs::read_to_string(root.join(rel_file))?))
}

pub fn get_implementation(
    root: &Path,
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
) -> Result<String, String> {
    get_implementation_resolved(root, resolve_symbol(symbol_table, symbol_name, file, None)?)
}

pub fn get_implementation_at(
    root: &Path,
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
    byte_offset: usize,
) -> Result<String, String> {
    get_implementation_resolved(
        root,
        resolve_symbol(symbol_table, symbol_name, file, Some(byte_offset))?,
    )
}

fn get_implementation_resolved(root: &Path, symbol: Symbol) -> Result<String, String> {
    let file = symbol.file.clone();
    let source = std::fs::read_to_string(root.join(&file))
        .map_err(|error| format!("failed to read '{file}': {error}"))?;

    let implementation = source_slice(&source, symbol.byte_range).ok_or_else(|| {
        format!(
            "invalid byte range {}..{} for '{}' in '{}'",
            symbol.byte_range.0, symbol.byte_range.1, symbol.name, file
        )
    })?;
    Ok(implementation.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallerInfo {
    pub file: String,
    pub line: usize,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallersResult {
    pub callers: Vec<CallerInfo>,
    pub total_callers: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CallersIndex {
    by_name: HashMap<String, Vec<CallerInfo>>,
}

impl CallersIndex {
    pub fn callers_for(
        &self,
        symbol_name: &str,
        _definition_file: &str,
        limit: usize,
    ) -> CallersResult {
        let mut callers = self.by_name.get(symbol_name).cloned().unwrap_or_default();
        callers.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
        let total_callers = callers.len();
        let truncated = total_callers > limit;
        if truncated {
            callers.truncate(limit);
        }
        CallersResult {
            callers,
            total_callers,
            truncated,
        }
    }
}

pub fn find_callers(
    root: &Path,
    file_tree: &FileTree,
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
    limit: usize,
) -> Result<CallersResult, String> {
    let _ = resolve_symbol(symbol_table, symbol_name, file, None)?;

    find_callers_resolved(root, file_tree, symbol_name, file, limit)
}

pub fn find_callers_at(
    root: &Path,
    file_tree: &FileTree,
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
    limit: usize,
    byte_offset: usize,
) -> Result<CallersResult, String> {
    let _ = resolve_symbol(symbol_table, symbol_name, file, Some(byte_offset))?;

    find_callers_resolved(root, file_tree, symbol_name, file, limit)
}

fn find_callers_resolved(
    root: &Path,
    file_tree: &FileTree,
    symbol_name: &str,
    file: &str,
    limit: usize,
) -> Result<CallersResult, String> {
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

pub fn build_callers_index(root: &Path, file_tree: &FileTree) -> CallersIndex {
    let entries = file_tree
        .all_paths_with_language()
        .into_par_iter()
        .flat_map_iter(|(rel_path, language)| {
            // Skip non-code files to avoid false positives from non-tree-sitter paths.
            if !language.has_tree_sitter_support() {
                return Vec::new();
            }
            let source = match std::fs::read_to_string(root.join(&rel_path)) {
                Ok(source) => source,
                Err(_) => return Vec::new(),
            };
            collect_callers_by_callee_ast(&source, &rel_path, language)
        })
        .collect::<Vec<_>>();

    let mut by_name = HashMap::<String, Vec<CallerInfo>>::new();
    for (callee, caller) in entries {
        by_name.entry(callee).or_default().push(caller);
    }
    for callers in by_name.values_mut() {
        callers.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
        callers.dedup_by(|left, right| {
            left.file == right.file
                && left.line == right.line
                && left.text == right.text
                && left.qualifier == right.qualifier
        });
    }
    CallersIndex { by_name }
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
                            .map(ToString::to_string)
                    });

                    let line = capture.node.start_position().row + 1;
                    let line_text = lines
                        .get(line.saturating_sub(1))
                        .copied()
                        .unwrap_or("")
                        .trim()
                        .to_string();

                    if is_definition_line(&line_text, symbol_name, language) {
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

fn collect_callers_by_callee_ast(
    source: &str,
    rel_path: &str,
    language: Language,
) -> Vec<(String, CallerInfo)> {
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
                    let callee = capture
                        .node
                        .utf8_text(source.as_bytes())
                        .unwrap_or("")
                        .trim();
                    if callee.is_empty() {
                        continue;
                    }
                    let qualifier_text = qualifier_index.and_then(|qi| {
                        match_
                            .captures
                            .iter()
                            .find(|c| c.index as usize == qi)
                            .and_then(|c| c.node.utf8_text(source.as_bytes()).ok())
                            .map(ToString::to_string)
                    });
                    let line = capture.node.start_position().row + 1;
                    let line_text = lines
                        .get(line.saturating_sub(1))
                        .copied()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if is_definition_line(&line_text, callee, language) {
                        continue;
                    }
                    callers.push((
                        callee.to_string(),
                        CallerInfo {
                            file: rel_path.to_string(),
                            line,
                            text: line_text,
                            qualifier: qualifier_text,
                        },
                    ));
                }
            }
            callers
        },
    ) else {
        return collect_callers_by_callee_regex(source, rel_path, language);
    };
    callers
}

fn collect_callers_by_callee_regex(
    source: &str,
    rel_path: &str,
    language: Language,
) -> Vec<(String, CallerInfo)> {
    let pattern = match regex::Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*[!(]") {
        Ok(pattern) => pattern,
        Err(_) => return Vec::new(),
    };
    let excluded_ranges = crate::content::compute_non_code_ranges(source, language);
    let lines: Vec<&str> = source.lines().collect();
    let line_offsets = crate::content::line_offsets(source, &lines);
    let mut callers = Vec::new();

    for (line_idx, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        for captures in pattern.captures_iter(line) {
            let Some(callee_match) = captures.get(1) else {
                continue;
            };
            if !excluded_ranges.is_empty()
                && let Some(found) = captures.get(0)
            {
                let byte_offset = line_offsets[line_idx] + found.start();
                if crate::content::is_in_excluded_range(byte_offset, &excluded_ranges) {
                    continue;
                }
            }
            let callee = callee_match.as_str();
            if is_definition_line(line, callee, language) {
                continue;
            }
            callers.push((
                callee.to_string(),
                CallerInfo {
                    file: rel_path.to_string(),
                    line: line_idx + 1,
                    text: line.trim().to_string(),
                    qualifier: None,
                },
            ));
        }
    }

    callers
}

fn find_callers_regex(
    source: &str,
    rel_path: &str,
    language: Language,
    symbol_name: &str,
    _definition_file: &str,
) -> Vec<CallerInfo> {
    let pattern = match regex::Regex::new(&format!(r"\b{}\s*[!(]", regex::escape(symbol_name))) {
        Ok(pattern) => pattern,
        Err(_) => return Vec::new(),
    };

    // Compute non-code ranges to filter comments/strings (same as grep scope=code).
    let excluded_ranges = crate::content::compute_non_code_ranges(source, language);
    let lines: Vec<&str> = source.lines().collect();
    let line_offsets = crate::content::line_offsets(source, &lines);

    let mut callers = Vec::new();

    for (line_idx, line) in lines.iter().enumerate() {
        if !pattern.is_match(line) {
            continue;
        }

        if !excluded_ranges.is_empty()
            && let Some(found) = pattern.find(line)
        {
            let byte_offset = line_offsets[line_idx] + found.start();
            if crate::content::is_in_excluded_range(byte_offset, &excluded_ranges) {
                continue;
            }
        }

        if is_definition_line(line, symbol_name, language) {
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
        if let std::collections::hash_map::Entry::Vacant(e) = cache.entry(language) {
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

            e.insert(OpsQueryCache {
                parser,
                callers_query,
                callers_capture_names,
                variables_query,
                variables_capture_names,
            });
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
        if let std::collections::hash_map::Entry::Vacant(e) = cache.entry(language) {
            let mut compiled = Vec::with_capacity(config.variable_regex_patterns.len());
            for pattern in config.variable_regex_patterns {
                let regex = regex::Regex::new(pattern.regex).ok()?;
                compiled.push(CompiledVariablePattern {
                    capture_group: pattern.capture_group,
                    regex,
                });
            }
            e.insert(compiled);
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
    pub total_test_symbols: usize,
}

pub fn find_tests(
    root: &Path,
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
    limit: usize,
    content_cache: Option<&Arc<DashMap<String, Arc<String>>>>,
) -> Result<TestsResult, String> {
    find_tests_resolved(
        root,
        symbol_table,
        symbol_name,
        file,
        limit,
        None,
        content_cache,
    )
}

pub fn find_tests_at(
    root: &Path,
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
    limit: usize,
    byte_offset: usize,
) -> Result<TestsResult, String> {
    find_tests_resolved(
        root,
        symbol_table,
        symbol_name,
        file,
        limit,
        Some(byte_offset),
        None,
    )
}

fn find_tests_resolved(
    root: &Path,
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
    limit: usize,
    byte_offset: Option<usize>,
    content_cache: Option<&Arc<DashMap<String, Arc<String>>>>,
) -> Result<TestsResult, String> {
    let _ = resolve_symbol(symbol_table, symbol_name, file, byte_offset)?;

    let all_test_symbols = symbol_table.test_symbols();
    let total_test_symbols = all_test_symbols.len();
    let cache = content_cache.cloned();
    let mut tests: Vec<TestInfo> = all_test_symbols
        .into_par_iter()
        .filter_map(|symbol| {
            let source = load_file_cached(root, &symbol.file, cache.as_ref()).ok()?;
            let body = source_slice(source.as_str(), symbol.byte_range)?;
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
        total_test_symbols,
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
    list_variables_resolved(
        root,
        resolve_symbol(symbol_table, function_name, file, None)?,
    )
}

pub fn list_variables_at(
    root: &Path,
    symbol_table: &SymbolTable,
    function_name: &str,
    file: &str,
    byte_offset: usize,
) -> Result<Vec<VariableInfo>, String> {
    list_variables_resolved(
        root,
        resolve_symbol(symbol_table, function_name, file, Some(byte_offset))?,
    )
}

fn list_variables_resolved(root: &Path, symbol: Symbol) -> Result<Vec<VariableInfo>, String> {
    let function_name = symbol.name.clone();
    let file = symbol.file.clone();

    let source = std::fs::read_to_string(root.join(&file))
        .map_err(|error| format!("failed to read '{file}': {error}"))?;

    let (start, end) = bounded_range(symbol.byte_range, source.len()).ok_or_else(|| {
        format!(
            "invalid byte range {}..{} for '{}' in '{}'",
            symbol.byte_range.0, symbol.byte_range.1, symbol.name, file
        )
    })?;
    let body = source_slice(&source, symbol.byte_range).ok_or_else(|| {
        format!(
            "invalid UTF-8 slice {}..{} for '{}' in '{}'",
            symbol.byte_range.0, symbol.byte_range.1, symbol.name, file
        )
    })?;

    if symbol.language.has_tree_sitter_support() {
        return Ok(list_variables_ast(
            &source,
            symbol.language,
            start,
            end,
            &function_name,
        ));
    }

    Ok(list_variables_regex(body, symbol.language, &function_name))
}

fn resolve_symbol(
    symbol_table: &SymbolTable,
    symbol_name: &str,
    file: &str,
    byte_offset: Option<usize>,
) -> Result<Symbol, String> {
    match byte_offset {
        Some(byte_offset) => symbol_table
            .get_at(file, symbol_name, byte_offset)
            .ok_or_else(|| {
                format!("symbol '{symbol_name}' not found at byte offset {byte_offset} in '{file}'")
            }),
        None => {
            let matches = symbol_table.matching_symbols_in_file(file, symbol_name);
            match matches.len() {
                0 => Err(symbol_not_found_message(symbol_table, symbol_name, file)),
                1 => {
                    if let Some(symbol) = matches.into_iter().next() {
                        Ok(symbol)
                    } else {
                        Err(symbol_not_found_message(symbol_table, symbol_name, file))
                    }
                }
                _ => Err(ambiguous_symbol_message(symbol_name, file, &matches)),
            }
        }
    }
}

fn symbol_not_found_message(symbol_table: &SymbolTable, symbol_name: &str, file: &str) -> String {
    let available = symbol_table.symbols_in_file(file);
    if available.is_empty() {
        format!("file '{file}' has no indexed symbols")
    } else {
        let names: Vec<String> = available
            .iter()
            .map(|symbol| match &symbol.parent {
                Some(parent) => format!("{parent}.{}", symbol.name),
                None => symbol.name.clone(),
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .take(20)
            .collect();
        format!(
            "symbol '{symbol_name}' not found in '{file}'. Available symbols: {}",
            names.join(", ")
        )
    }
}

fn ambiguous_symbol_message(symbol_name: &str, file: &str, matches: &[Symbol]) -> String {
    let rendered_matches = matches
        .iter()
        .map(|symbol| match &symbol.parent {
            Some(parent) => format!("{parent}.{}:{}", symbol.name, symbol.line_range.0),
            None => format!("{}:{}", symbol.name, symbol.line_range.0),
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "symbol '{symbol_name}' is ambiguous in '{file}'. Provide `line` to disambiguate. Matches: {rendered_matches}"
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectIndex;
    use crate::ProjectIndexConfig;
    use crate::file_entry::Language;

    fn make_symbol(
        name: &str,
        parent: Option<&str>,
        byte_range: (usize, usize),
        line: usize,
    ) -> Symbol {
        Symbol {
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            kind: SymbolKind::Method,
            file: "src/lib.rs".to_string(),
            byte_range,
            line_range: (line, line),
            language: Language::Rust,
            signature: name.to_string(),
            definition: None,
            parent: parent.map(str::to_string),
        }
    }

    #[test]
    fn callers_index_matches_find_callers_for_symbol() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();
        std::fs::create_dir_all(repo_root.join("src")).expect("mkdir");
        std::fs::write(
            repo_root.join("src/main.rs"),
            "fn helper() {}\nfn run() { helper(); }\nfn main() { helper(); }\n",
        )
        .expect("write source");

        let index = ProjectIndex::new_with_config(
            repo_root.to_path_buf(),
            ProjectIndexConfig {
                persist_annotations: false,
                ..ProjectIndexConfig::default()
            },
        )
        .expect("project index");

        let direct = index
            .find_callers("helper", "src/main.rs", 64)
            .expect("direct callers");
        let indexed = index
            .build_callers_index()
            .callers_for("helper", "src/main.rs", 64);

        assert_eq!(direct.total_callers, indexed.total_callers);
        assert_eq!(direct.truncated, indexed.truncated);
        assert_eq!(direct.callers, indexed.callers);
    }

    #[test]
    fn implementation_requires_line_when_symbol_name_is_ambiguous() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        let file_path = root.join("src/lib.rs");
        std::fs::create_dir_all(file_path.parent().expect("parent")).expect("create parent");
        std::fs::write(
            &file_path,
            "impl A { fn duplicate() {} }\nimpl B { fn duplicate() {} }\n",
        )
        .expect("write source");

        let table = SymbolTable::new();
        table.insert(make_symbol("duplicate", Some("A"), (9, 25), 1));
        table.insert(make_symbol("duplicate", Some("B"), (37, 53), 2));

        let err = get_implementation(root, &table, "duplicate", "src/lib.rs")
            .expect_err("ambiguous symbol should require a line");

        assert_eq!(
            err,
            "symbol 'duplicate' is ambiguous in 'src/lib.rs'. Provide `line` to disambiguate. Matches: A.duplicate:1, B.duplicate:2"
        );
    }
}
