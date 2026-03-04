use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use rayon::prelude::*;
use regex::Regex;
use serde::Serialize;
use tree_sitter::StreamingIterator;

use crate::error::TreeSitterError;
use crate::file_entry::Language;
use crate::file_tree::FileTree;
use crate::queries;

struct NonCodeQueryCache {
    parser: tree_sitter::Parser,
    query: tree_sitter::Query,
}

thread_local! {
    static NON_CODE_QUERY_CACHE: RefCell<HashMap<Language, NonCodeQueryCache>> =
        RefCell::new(HashMap::new());
}

#[derive(Debug, Clone, Serialize)]
pub struct PeekResult {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    pub content: String,
}

pub fn peek(
    root: &Path,
    file_tree: &FileTree,
    file: &str,
    start_line: usize,
    line_count: usize,
) -> Result<PeekResult, TreeSitterError> {
    if line_count == 0 {
        return Err(TreeSitterError::InvalidRange {
            start: start_line,
            end: start_line,
        });
    }

    if file_tree.get(file).is_none() {
        return Err(TreeSitterError::PathOutsideRoot {
            path: root.join(file),
        });
    }

    let source = std::fs::read_to_string(root.join(file))?;
    let lines: Vec<&str> = source.lines().collect();
    let total_lines = lines.len();

    // If start_line is beyond EOF, clamp to show the last `line_count` lines.
    let start_idx = if start_line.saturating_sub(1) >= total_lines {
        total_lines.saturating_sub(line_count)
    } else {
        start_line.saturating_sub(1)
    };
    let end_idx = (start_idx + line_count).min(total_lines);

    let content = lines[start_idx..end_idx]
        .iter()
        .enumerate()
        .map(|(idx, line)| format!("{:>6} │ {}", start_idx + idx + 1, line))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(PeekResult {
        file: file.to_string(),
        start_line: start_idx + 1,
        end_line: end_idx,
        total_lines,
        content,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrepScope {
    All,
    Code,
}

impl GrepScope {
    pub fn from_input(value: Option<&str>) -> Self {
        match value {
            Some(scope) if scope.eq_ignore_ascii_case("code") => Self::Code,
            _ => Self::All,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GrepMatch {
    pub file: String,
    pub line: usize,
    pub text: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GrepResult {
    pub pattern: String,
    pub matches: Vec<GrepMatch>,
    pub total_matches: usize,
    pub truncated: bool,
}

pub fn grep(
    root: &Path,
    file_tree: &FileTree,
    pattern: &str,
    scope: GrepScope,
    max_matches: usize,
    context_lines: usize,
) -> Result<GrepResult, TreeSitterError> {
    let regex = Regex::new(pattern).map_err(|e| TreeSitterError::InvalidPattern(e.to_string()))?;

    let mut paths = file_tree.all_paths_with_language();
    if scope == GrepScope::Code {
        paths.retain(|(_, language)| language.has_tree_sitter_support());
    }
    paths.sort_by(|a, b| a.0.cmp(&b.0));
    let mut per_file = paths
        .par_iter()
        .map(|(rel_path, language)| {
            let source = match std::fs::read_to_string(root.join(rel_path)) {
                Ok(source) => source,
                Err(_) => return (rel_path.clone(), 0usize, Vec::new()),
            };

            let excluded_ranges = if scope == GrepScope::Code && language.has_tree_sitter_support()
            {
                compute_non_code_ranges(&source, *language)
            } else {
                Vec::new()
            };

            let lines: Vec<&str> = source.lines().collect();
            let line_offsets = line_offsets(&source, &lines);

            let mut file_matches = Vec::new();
            let mut file_total_matches = 0usize;
            for (line_idx, line) in lines.iter().enumerate() {
                let Some(found) = regex.find(line) else {
                    continue;
                };

                if scope == GrepScope::Code && !excluded_ranges.is_empty() {
                    let byte_offset = line_offsets[line_idx] + found.start();
                    if is_in_excluded_range(byte_offset, &excluded_ranges) {
                        continue;
                    }
                }

                file_total_matches += 1;
                if file_matches.len() >= max_matches {
                    continue;
                }

                let context_start = line_idx.saturating_sub(context_lines);
                let context_end = (line_idx + context_lines + 1).min(lines.len());

                file_matches.push(GrepMatch {
                    file: rel_path.clone(),
                    line: line_idx + 1,
                    text: line.to_string(),
                    context_before: lines[context_start..line_idx]
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    context_after: lines[(line_idx + 1)..context_end]
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                });
            }

            (rel_path.clone(), file_total_matches, file_matches)
        })
        .collect::<Vec<_>>();
    per_file.sort_by(|a, b| a.0.cmp(&b.0));

    let mut matches = Vec::new();
    let mut total_matches = 0usize;
    for (_, file_total_matches, file_matches) in per_file {
        total_matches += file_total_matches;
        for matched in file_matches {
            if matches.len() < max_matches {
                matches.push(matched);
            }
        }
    }

    Ok(GrepResult {
        pattern: pattern.to_string(),
        truncated: total_matches > max_matches,
        matches,
        total_matches,
    })
}

pub(crate) fn line_offsets(source: &str, lines: &[&str]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(lines.len());
    let mut offset = 0usize;
    let bytes = source.as_bytes();
    for line in lines {
        offsets.push(offset);
        offset += line.len();
        if bytes.get(offset) == Some(&b'\r') {
            offset += 1;
        }
        if bytes.get(offset) == Some(&b'\n') {
            offset += 1;
        }
    }
    offsets
}

pub(crate) fn compute_non_code_ranges(source: &str, language: Language) -> Vec<(usize, usize)> {
    let Some(config) = queries::get_language_config(language) else {
        return Vec::new();
    };

    NON_CODE_QUERY_CACHE
        .with(|cache| {
            let mut cache = cache.borrow_mut();
            if let std::collections::hash_map::Entry::Vacant(e) = cache.entry(language) {
                let mut parser = tree_sitter::Parser::new();
                parser.set_language(&config.language).ok()?;
                let query =
                    tree_sitter::Query::new(&config.language, config.non_code_query).ok()?;
                e.insert(NonCodeQueryCache { parser, query });
            }

            let entry = cache.get_mut(&language)?;
            let tree = entry.parser.parse(source, None)?;

            let mut ranges = Vec::new();
            let mut cursor = tree_sitter::QueryCursor::new();
            let mut matches = cursor.matches(&entry.query, tree.root_node(), source.as_bytes());
            while let Some(match_) = matches.next() {
                for capture in match_.captures {
                    ranges.push((capture.node.start_byte(), capture.node.end_byte()));
                }
            }

            ranges.sort_by_key(|(start, _)| *start);
            Some(ranges)
        })
        .unwrap_or_default()
}

pub(crate) fn is_in_excluded_range(offset: usize, ranges: &[(usize, usize)]) -> bool {
    ranges
        .binary_search_by(|(start, end)| {
            if offset < *start {
                std::cmp::Ordering::Greater
            } else if offset >= *end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}
