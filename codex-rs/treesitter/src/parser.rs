use std::path::Path;

use tree_sitter::StreamingIterator;

use crate::config::ProjectIndexConfig;
use crate::error::TreeSitterError;
use crate::file_entry::Language;
use crate::file_tree::FileTree;
use crate::queries;
use crate::symbol::Symbol;
use crate::symbol::SymbolKind;
use crate::symbol_table::SymbolTable;

pub fn extract_symbols_from_file(
    root: &Path,
    rel_path: &str,
    language: Language,
) -> Result<Vec<Symbol>, TreeSitterError> {
    let Some(config) = queries::get_language_config(language) else {
        return Ok(Vec::new());
    };

    let abs_path = root.join(rel_path);
    let source = std::fs::read_to_string(abs_path)?;

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&config.language)?;

    let Some(tree) = parser.parse(&source, None) else {
        return Err(TreeSitterError::ParseFailed);
    };

    let query = tree_sitter::Query::new(&config.language, config.symbols_query)?;
    let capture_names: Vec<String> = query
        .capture_names()
        .iter()
        .map(ToString::to_string)
        .collect();

    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    let mut symbols = Vec::new();
    let mut current_impl_type: Option<String> = None;

    while let Some(match_) = matches.next() {
        let mut name: Option<String> = None;
        let mut kind: Option<SymbolKind> = None;
        let mut def_node: Option<tree_sitter::Node> = None;
        let mut parent: Option<String> = None;

        for capture in match_.captures {
            let capture_name = &capture_names[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name.as_str() {
                "function.name" => {
                    name = Some(text.to_string());
                    kind = Some(SymbolKind::Function);
                }
                "function.def" => {
                    def_node = Some(capture.node);
                }
                "method.name" => {
                    name = Some(text.to_string());
                    kind = Some(SymbolKind::Method);
                    parent = current_impl_type.clone();
                }
                "method.def" => {
                    def_node = Some(capture.node);
                }
                "impl.type" => {
                    current_impl_type = Some(text.to_string());
                }
                "class.name" => {
                    name = Some(text.to_string());
                    kind = Some(SymbolKind::Class);
                }
                "class.def" => {
                    def_node = Some(capture.node);
                }
                "struct.name" => {
                    name = Some(text.to_string());
                    kind = Some(SymbolKind::Struct);
                }
                "struct.def" => {
                    def_node = Some(capture.node);
                }
                "enum.name" => {
                    name = Some(text.to_string());
                    kind = Some(SymbolKind::Enum);
                }
                "enum.def" => {
                    def_node = Some(capture.node);
                }
                "trait.name" => {
                    name = Some(text.to_string());
                    kind = Some(SymbolKind::Trait);
                }
                "trait.def" => {
                    def_node = Some(capture.node);
                }
                "interface.name" => {
                    name = Some(text.to_string());
                    kind = Some(SymbolKind::Interface);
                }
                "interface.def" => {
                    def_node = Some(capture.node);
                }
                "type.name" => {
                    name = Some(text.to_string());
                    kind = Some(SymbolKind::Type);
                }
                "type.def" => {
                    def_node = Some(capture.node);
                }
                "const.name" => {
                    name = Some(text.to_string());
                    kind = Some(SymbolKind::Constant);
                }
                "const.def" => {
                    def_node = Some(capture.node);
                }
                "mod.name" => {
                    name = Some(text.to_string());
                    kind = Some(SymbolKind::Module);
                }
                "mod.def" => {
                    def_node = Some(capture.node);
                }
                _ => {}
            }
        }

        if let (Some(name), Some(kind), Some(def_node)) = (name, kind, def_node) {
            let start = def_node.start_position();
            let end = def_node.end_position();
            let byte_range = (def_node.start_byte(), def_node.end_byte());
            let line_range = (start.row + 1, end.row + 1);
            let node_text = def_node.utf8_text(source.as_bytes()).unwrap_or("");
            let signature = node_text.lines().next().unwrap_or("").to_string();

            symbols.push(Symbol {
                name,
                kind,
                file: rel_path.to_string(),
                byte_range,
                line_range,
                language,
                signature,
                definition: None,
                parent,
            });
        }
    }

    Ok(symbols)
}

pub fn extract_all_symbols(
    root: &Path,
    file_tree: &FileTree,
    symbol_table: &SymbolTable,
    config: &ProjectIndexConfig,
) -> Result<usize, TreeSitterError> {
    let paths = file_tree.all_paths_with_language();
    let mut total = 0;

    for (rel_path, language) in paths {
        if !language.has_tree_sitter_support() || !config.is_language_enabled(language) {
            continue;
        }

        match extract_symbols_from_file(root, &rel_path, language) {
            Ok(symbols) => {
                total += symbols.len();
                for symbol in symbols {
                    symbol_table.insert(symbol);
                }
            }
            Err(error) => {
                tracing::debug!("failed to parse {}: {error}", rel_path);
            }
        }
    }

    Ok(total)
}

pub fn reindex_file(
    root: &Path,
    symbol_table: &SymbolTable,
    rel_path: &str,
    language: Language,
    config: &ProjectIndexConfig,
) -> Result<(), TreeSitterError> {
    symbol_table.remove_file(rel_path);
    if !language.has_tree_sitter_support() || !config.is_language_enabled(language) {
        return Ok(());
    }

    if root.join(rel_path).is_file()
        && let Ok(symbols) = extract_symbols_from_file(root, rel_path, language)
    {
        for symbol in symbols {
            symbol_table.insert(symbol);
        }
    }

    Ok(())
}
