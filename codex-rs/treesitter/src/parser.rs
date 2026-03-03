use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use rayon::prelude::*;
use tree_sitter::StreamingIterator;

use crate::config::ProjectIndexConfig;
use crate::error::TreeSitterError;
use crate::file_entry::Language;
use crate::file_tree::FileTree;
use crate::queries;
use crate::symbol::Symbol;
use crate::symbol::SymbolKind;
use crate::symbol_table::SymbolTable;

struct SymbolExtractCache {
    parser: tree_sitter::Parser,
    query: tree_sitter::Query,
    capture_names: Vec<String>,
}

fn symbol_kind_from_capture(capture_name: &str) -> Option<SymbolKind> {
    let kind_name = capture_name.strip_suffix(".name")?;
    match kind_name {
        "function" => Some(SymbolKind::Function),
        "method" => Some(SymbolKind::Method),
        "class" => Some(SymbolKind::Class),
        "struct" => Some(SymbolKind::Struct),
        "enum" => Some(SymbolKind::Enum),
        "trait" => Some(SymbolKind::Trait),
        "interface" => Some(SymbolKind::Interface),
        "type" => Some(SymbolKind::Type),
        "const" => Some(SymbolKind::Constant),
        "mod" => Some(SymbolKind::Module),
        _ => None,
    }
}

fn symbol_kind_priority(kind: SymbolKind) -> u8 {
    match kind {
        SymbolKind::Method => 5,
        SymbolKind::Struct | SymbolKind::Interface => 4,
        SymbolKind::Function | SymbolKind::Class | SymbolKind::Trait | SymbolKind::Enum => 3,
        SymbolKind::Constant | SymbolKind::Module | SymbolKind::Variable => 2,
        SymbolKind::Type => 1,
    }
}

thread_local! {
    static SYMBOL_EXTRACT_CACHE: RefCell<HashMap<Language, SymbolExtractCache>> =
        RefCell::new(HashMap::new());
}

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

    SYMBOL_EXTRACT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(&language) {
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&config.language)?;

            let query = tree_sitter::Query::new(&config.language, config.symbols_query)?;
            let capture_names: Vec<String> = query
                .capture_names()
                .iter()
                .map(ToString::to_string)
                .collect();

            cache.insert(
                language,
                SymbolExtractCache {
                    parser,
                    query,
                    capture_names,
                },
            );
        }

        let entry = cache
            .get_mut(&language)
            .ok_or(TreeSitterError::UnsupportedLanguage(language))?;

        let Some(tree) = entry.parser.parse(&source, None) else {
            return Err(TreeSitterError::ParseFailed);
        };

        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&entry.query, tree.root_node(), source.as_bytes());

        let mut symbols = Vec::new();

        while let Some(match_) = matches.next() {
            let mut name: Option<String> = None;
            let mut kind: Option<SymbolKind> = None;
            let mut def_node: Option<tree_sitter::Node> = None;
            let mut parent: Option<String> = None;
            let mut impl_type: Option<String> = None;
            let mut class_name: Option<String> = None;

            for capture in match_.captures {
                let capture_name = entry.capture_names[capture.index as usize].as_str();
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                if capture_name == "impl.type" {
                    impl_type = Some(text.to_string());
                    continue;
                }

                if capture_name == "class.name" {
                    class_name = Some(text.to_string());
                }

                if capture_name == "method.parent" {
                    class_name = Some(text.to_string());
                }

                if let Some(capture_kind) = symbol_kind_from_capture(capture_name) {
                    name = Some(text.to_string());
                    kind = Some(capture_kind);
                    continue;
                }

                if capture_name.ends_with(".def") {
                    def_node = Some(capture.node);
                }
            }

            if matches!(kind, Some(SymbolKind::Method)) {
                parent = impl_type.or(class_name);
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

        let mut dedup_indices: HashMap<(String, (usize, usize)), usize> = HashMap::new();
        let mut deduped: Vec<Symbol> = Vec::with_capacity(symbols.len());
        for symbol in symbols {
            let key = (symbol.name.clone(), symbol.byte_range);
            if let Some(existing_idx) = dedup_indices.get(&key).copied() {
                let existing_priority = symbol_kind_priority(deduped[existing_idx].kind);
                let new_priority = symbol_kind_priority(symbol.kind);
                if new_priority > existing_priority {
                    deduped[existing_idx] = symbol;
                }
            } else {
                dedup_indices.insert(key, deduped.len());
                deduped.push(symbol);
            }
        }

        Ok(deduped)
    })
}

pub fn extract_all_symbols(
    root: &Path,
    file_tree: &FileTree,
    symbol_table: &SymbolTable,
    config: &ProjectIndexConfig,
) -> Result<usize, TreeSitterError> {
    let paths: Vec<_> = file_tree
        .all_paths_with_language()
        .into_iter()
        .filter(|(_, language)| {
            language.has_tree_sitter_support() && config.is_language_enabled(*language)
        })
        .collect();

    let total: usize = paths
        .par_iter()
        .map(
            |(rel_path, language)| match extract_symbols_from_file(root, rel_path, *language) {
                Ok(symbols) => {
                    let count = symbols.len();
                    for symbol in symbols {
                        symbol_table.insert(symbol);
                    }
                    count
                }
                Err(error) => {
                    tracing::debug!("failed to parse {}: {error}", rel_path);
                    0
                }
            },
        )
        .sum();

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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::tempdir;

    use super::*;

    fn write_source(root: &Path, rel_path: &str, source: &str) {
        let abs = root.join(rel_path);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).expect("create parent directory");
        }
        std::fs::write(abs, source).expect("write source file");
    }

    #[cfg(feature = "rust")]
    #[test]
    fn rust_method_parent_tracks_each_impl_block() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let rel_path = "src/lib.rs";
        write_source(
            root,
            rel_path,
            r#"
impl Foo {
    fn bar(&self) {}
}

impl Baz {
    fn qux(&self) {}
}

fn free() {}
"#,
        );

        let symbols =
            extract_symbols_from_file(root, rel_path, Language::Rust).expect("extract symbols");

        let by_name: HashMap<_, _> = symbols
            .iter()
            .map(|symbol| (symbol.name.as_str(), symbol))
            .collect();

        assert_eq!(
            by_name
                .get("bar")
                .and_then(|symbol| symbol.parent.as_deref()),
            Some("Foo")
        );
        assert_eq!(
            by_name
                .get("qux")
                .and_then(|symbol| symbol.parent.as_deref()),
            Some("Baz")
        );
        assert_eq!(
            by_name
                .get("free")
                .and_then(|symbol| symbol.parent.as_deref()),
            None
        );
    }

    #[cfg(feature = "python")]
    #[test]
    fn python_methods_include_enclosing_class_parent() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let rel_path = "pkg/mod.py";
        write_source(
            root,
            rel_path,
            r#"
class Greeter:
    def hello(self):
        return "hi"

def free():
    return 1
"#,
        );

        let symbols =
            extract_symbols_from_file(root, rel_path, Language::Python).expect("extract symbols");

        let by_name: HashMap<_, _> = symbols
            .iter()
            .map(|symbol| (symbol.name.as_str(), symbol))
            .collect();

        assert_eq!(
            by_name
                .get("hello")
                .and_then(|symbol| symbol.parent.as_deref()),
            Some("Greeter")
        );
        assert_eq!(
            by_name
                .get("free")
                .and_then(|symbol| symbol.parent.as_deref()),
            None
        );
    }

    #[cfg(feature = "go")]
    #[test]
    fn go_struct_and_interface_type_decls_are_not_duplicated() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let rel_path = "pkg/types.go";
        write_source(
            root,
            rel_path,
            r#"
package pkg

type Foo struct{}
type Bar interface{}
type Baz int
"#,
        );

        let symbols =
            extract_symbols_from_file(root, rel_path, Language::Go).expect("extract symbols");

        let foo = symbols.iter().filter(|symbol| symbol.name == "Foo").count();
        let bar = symbols.iter().filter(|symbol| symbol.name == "Bar").count();
        let baz = symbols.iter().filter(|symbol| symbol.name == "Baz").count();

        assert_eq!(foo, 1, "expected one symbol for struct type Foo");
        assert_eq!(bar, 1, "expected one symbol for interface type Bar");
        assert_eq!(baz, 1, "expected one symbol for alias type Baz");
    }
}
