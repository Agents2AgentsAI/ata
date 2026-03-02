use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::error::TreeSitterError;
use crate::file_entry::FileMark;
use crate::file_tree::FileTree;
use crate::symbol_table::SymbolTable;

#[derive(Debug, Serialize, Deserialize, Default)]
struct AnnotationData {
    #[serde(default)]
    file_definitions: HashMap<String, String>,
    #[serde(default)]
    file_marks: HashMap<String, Vec<FileMark>>,
    #[serde(default)]
    symbol_definitions: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationSaveStats {
    pub path: String,
    pub persisted: bool,
    pub file_definitions: usize,
    pub file_marks: usize,
    pub symbol_definitions: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationLoadStats {
    pub path: String,
    pub loaded: bool,
    pub file_definitions_applied: usize,
    pub file_marks_applied: usize,
    pub symbol_definitions_applied: usize,
}

impl AnnotationSaveStats {
    pub fn skipped(path: &Path) -> Self {
        Self {
            path: path.display().to_string(),
            persisted: false,
            file_definitions: 0,
            file_marks: 0,
            symbol_definitions: 0,
        }
    }
}

impl AnnotationLoadStats {
    pub fn skipped(path: &Path) -> Self {
        Self {
            path: path.display().to_string(),
            loaded: false,
            file_definitions_applied: 0,
            file_marks_applied: 0,
            symbol_definitions_applied: 0,
        }
    }
}

pub fn default_annotation_store_path(root: &Path) -> PathBuf {
    root.join(".codex").join("treesitter_annotations.json")
}

pub fn save_annotations(
    path: &Path,
    file_tree: &FileTree,
    symbol_table: &SymbolTable,
) -> Result<AnnotationSaveStats, TreeSitterError> {
    let mut data = AnnotationData::default();

    for entry in file_tree.all_entries() {
        if let Some(definition) = entry.definition {
            data.file_definitions.insert(entry.rel_path.clone(), definition);
        }
        if !entry.marks.is_empty() {
            data.file_marks.insert(entry.rel_path, entry.marks);
        }
    }

    for (key, definition) in symbol_table.all_symbol_definitions() {
        data.symbol_definitions.insert(key, definition);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let serialized = serde_json::to_string_pretty(&data)?;
    std::fs::write(path, serialized)?;

    Ok(AnnotationSaveStats {
        path: path.display().to_string(),
        persisted: true,
        file_definitions: data.file_definitions.len(),
        file_marks: data.file_marks.len(),
        symbol_definitions: data.symbol_definitions.len(),
    })
}

pub fn load_annotations(
    path: &Path,
    file_tree: &FileTree,
    symbol_table: &SymbolTable,
) -> Result<AnnotationLoadStats, TreeSitterError> {
    if !path.exists() {
        return Ok(AnnotationLoadStats::skipped(path));
    }

    let content = std::fs::read_to_string(path)?;
    let data: AnnotationData = serde_json::from_str(&content)?;

    let mut file_definitions_applied = 0usize;
    let mut file_marks_applied = 0usize;
    let mut symbol_definitions_applied = 0usize;

    for (rel_path, definition) in data.file_definitions {
        if file_tree
            .define_file(&rel_path, &definition, true)
            .is_ok()
        {
            file_definitions_applied += 1;
        }
    }

    for (rel_path, marks) in data.file_marks {
        if file_tree.set_marks(&rel_path, marks).is_ok() {
            file_marks_applied += 1;
        }
    }

    for (key, definition) in data.symbol_definitions {
        if symbol_table
            .set_definition_by_key(&key, &definition, true)
            .is_ok()
        {
            symbol_definitions_applied += 1;
        }
    }

    Ok(AnnotationLoadStats {
        path: path.display().to_string(),
        loaded: true,
        file_definitions_applied,
        file_marks_applied,
        symbol_definitions_applied,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::tempdir;

    use crate::file_entry::FileEntry;
    use crate::file_entry::FileMark;
    use crate::file_entry::Language;
    use crate::symbol::Symbol;
    use crate::symbol::SymbolKind;

    use super::load_annotations;
    use super::save_annotations;
    use super::AnnotationLoadStats;
    use super::default_annotation_store_path;
    use crate::file_tree::FileTree;
    use crate::symbol_table::SymbolTable;

    #[test]
    fn round_trip_annotations() {
        let tmp = tempdir().expect("tempdir");
        let path = default_annotation_store_path(tmp.path());

        let file_tree = FileTree::new();
        file_tree.insert(FileEntry::new("src/lib.rs".to_string(), 120));
        file_tree
            .define_file("src/lib.rs", "entrypoint module", true)
            .expect("define file");
        file_tree
            .mark_file("src/lib.rs", FileMark::Docs)
            .expect("mark file");

        let symbol_table = SymbolTable::new();
        symbol_table.insert(Symbol {
            name: "run".to_string(),
            kind: SymbolKind::Function,
            file: "src/lib.rs".to_string(),
            byte_range: (0, 32),
            line_range: (1, 3),
            language: Language::Rust,
            signature: "fn run()".to_string(),
            definition: Some("main entrypoint".to_string()),
            parent: None,
        });

        let save_stats = save_annotations(Path::new(&path), &file_tree, &symbol_table)
            .expect("save annotations");
        assert!(save_stats.persisted);
        assert_eq!(save_stats.file_definitions, 1);
        assert_eq!(save_stats.file_marks, 1);
        assert_eq!(save_stats.symbol_definitions, 1);

        let new_file_tree = FileTree::new();
        new_file_tree.insert(FileEntry::new("src/lib.rs".to_string(), 120));

        let new_symbol_table = SymbolTable::new();
        new_symbol_table.insert(Symbol {
            name: "run".to_string(),
            kind: SymbolKind::Function,
            file: "src/lib.rs".to_string(),
            byte_range: (0, 32),
            line_range: (1, 3),
            language: Language::Rust,
            signature: "fn run()".to_string(),
            definition: None,
            parent: None,
        });

        let load_stats = load_annotations(Path::new(&path), &new_file_tree, &new_symbol_table)
            .expect("load annotations");
        assert!(load_stats.loaded);
        assert_eq!(load_stats.file_definitions_applied, 1);
        assert_eq!(load_stats.file_marks_applied, 1);
        assert_eq!(load_stats.symbol_definitions_applied, 1);

        let file_entry = new_file_tree.get("src/lib.rs").expect("file exists");
        assert_eq!(file_entry.definition.as_deref(), Some("entrypoint module"));
        assert!(file_entry.marks.contains(&FileMark::Docs));

        let symbol = new_symbol_table
            .get("src/lib.rs", "run")
            .expect("symbol exists");
        assert_eq!(symbol.definition.as_deref(), Some("main entrypoint"));
    }

    #[test]
    fn load_missing_file_is_non_fatal() {
        let tmp = tempdir().expect("tempdir");
        let path = default_annotation_store_path(tmp.path());
        let file_tree = FileTree::new();
        let symbol_table = SymbolTable::new();

        let stats = load_annotations(Path::new(&path), &file_tree, &symbol_table)
            .expect("missing annotation file should not fail");

        assert_eq!(
            stats.path,
            AnnotationLoadStats::skipped(Path::new(&path)).path
        );
        assert!(!stats.loaded);
    }
}
