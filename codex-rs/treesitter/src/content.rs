use std::path::Path;

use regex::Regex;
use serde::Serialize;
use tree_sitter::StreamingIterator;

use crate::error::TreeSitterError;
use crate::file_entry::Language;
use crate::file_tree::FileTree;
use crate::queries;

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

    let start_idx = start_line.saturating_sub(1).min(total_lines);
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
    let regex = Regex::new(pattern).map_err(|_| TreeSitterError::ParseFailed)?;

    let mut matches = Vec::new();
    let mut total_matches = 0;

    let mut paths = file_tree.all_paths_with_language();
    paths.sort_by(|a, b| a.0.cmp(&b.0));

    for (rel_path, language) in &paths {
        let source = match std::fs::read_to_string(root.join(rel_path)) {
            Ok(source) => source,
            Err(_) => continue,
        };

        let excluded_ranges = if scope == GrepScope::Code && language.has_tree_sitter_support() {
            compute_non_code_ranges(&source, *language)
        } else {
            Vec::new()
        };

        let lines: Vec<&str> = source.lines().collect();
        let line_offsets = line_offsets(&lines);

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

            total_matches += 1;
            if matches.len() >= max_matches {
                continue;
            }

            let context_start = line_idx.saturating_sub(context_lines);
            let context_end = (line_idx + context_lines + 1).min(lines.len());

            matches.push(GrepMatch {
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
    }

    Ok(GrepResult {
        pattern: pattern.to_string(),
        truncated: total_matches > max_matches,
        matches,
        total_matches,
    })
}

fn line_offsets(lines: &[&str]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(lines.len());
    let mut offset = 0;
    for line in lines {
        offsets.push(offset);
        offset += line.len() + 1;
    }
    offsets
}

fn compute_non_code_ranges(source: &str, language: Language) -> Vec<(usize, usize)> {
    let Some(config) = queries::get_language_config(language) else {
        return Vec::new();
    };

    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&config.language).is_err() {
        return Vec::new();
    }

    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    let query_source = match language {
        Language::Rust => {
            r#"
            (line_comment) @skip
            (block_comment) @skip
            (string_literal) @skip
            (raw_string_literal) @skip
        "#
        }
        Language::Python => {
            r#"
            (comment) @skip
            (string) @skip
        "#
        }
        Language::TypeScript | Language::JavaScript => {
            r#"
            (comment) @skip
            (string) @skip
            (template_string) @skip
        "#
        }
        Language::Go => {
            r#"
            (comment) @skip
            (raw_string_literal) @skip
            (interpreted_string_literal) @skip
        "#
        }
        Language::Java => {
            r#"
            (line_comment) @skip
            (block_comment) @skip
            (string_literal) @skip
        "#
        }
        Language::Scala => {
            r#"
            (comment) @skip
            (block_comment) @skip
            (string) @skip
            (interpolated_string_expression) @skip
        "#
        }
        Language::Other => return Vec::new(),
    };

    let Ok(query) = tree_sitter::Query::new(&config.language, query_source) else {
        return Vec::new();
    };

    let mut ranges = Vec::new();
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            ranges.push((capture.node.start_byte(), capture.node.end_byte()));
        }
    }

    ranges.sort_by_key(|(start, _)| *start);
    ranges
}

fn is_in_excluded_range(offset: usize, ranges: &[(usize, usize)]) -> bool {
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
