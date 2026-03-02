use std::path::Path;
use std::path::PathBuf;

use crate::content::GrepResult;
use crate::content::GrepScope;
use crate::content::PeekResult;
use crate::content::{self};
use crate::error::TreeSitterError;
use crate::file_entry::FileEntry;
use crate::file_tree::FileTree;
use crate::ops::CallerInfo;
use crate::ops::TestInfo;
use crate::ops::VariableInfo;
use crate::ops::{self};
use crate::parser;
use crate::symbol::Symbol;
use crate::symbol_table::SymbolTable;
use crate::walker;

#[derive(Debug)]
pub struct ProjectIndex {
    root: PathBuf,
    file_tree: FileTree,
    symbol_table: SymbolTable,
}

impl ProjectIndex {
    pub fn new(root: PathBuf) -> Result<Self, TreeSitterError> {
        let file_tree = FileTree::new();
        let symbol_table = SymbolTable::new();

        walker::scan_directory(&root, &file_tree)?;
        parser::extract_all_symbols(&root, &file_tree, &symbol_table)?;

        Ok(Self {
            root,
            file_tree,
            symbol_table,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn file_tree(&self) -> &FileTree {
        &self.file_tree
    }

    pub fn symbol_table(&self) -> &SymbolTable {
        &self.symbol_table
    }

    pub fn rel_path_for_absolute(&self, path: &Path) -> Result<String, TreeSitterError> {
        let rel = path
            .strip_prefix(&self.root)
            .map_err(|_| TreeSitterError::PathOutsideRoot {
                path: path.to_path_buf(),
            })?;
        Ok(rel.to_string_lossy().replace('\\', "/"))
    }

    pub fn reindex_absolute_path(&self, path: &Path) -> Result<(), TreeSitterError> {
        let rel_path = self.rel_path_for_absolute(path)?;

        if path.is_file() {
            let metadata = path.metadata()?;
            self.file_tree
                .insert(FileEntry::new(rel_path.clone(), metadata.len()));
            let language = crate::file_entry::Language::from_path(path);
            parser::reindex_file(&self.root, &self.symbol_table, &rel_path, language)?;
        } else {
            self.file_tree.remove(&rel_path);
            self.symbol_table.remove_file(&rel_path);
        }

        Ok(())
    }

    pub fn search_symbols(&self, query: &str, limit: usize) -> Vec<Symbol> {
        ops::search_symbols(&self.symbol_table, query, limit)
    }

    pub fn find_callers(
        &self,
        symbol: &str,
        rel_file: &str,
        limit: usize,
    ) -> Result<Vec<CallerInfo>, String> {
        ops::find_callers(
            &self.root,
            &self.file_tree,
            &self.symbol_table,
            symbol,
            rel_file,
            limit,
        )
    }

    pub fn find_tests(
        &self,
        symbol: &str,
        rel_file: &str,
        limit: usize,
    ) -> Result<Vec<TestInfo>, String> {
        ops::find_tests(&self.root, &self.symbol_table, symbol, rel_file, limit)
    }

    pub fn list_variables(
        &self,
        function: &str,
        rel_file: &str,
    ) -> Result<Vec<VariableInfo>, String> {
        ops::list_variables(&self.root, &self.symbol_table, function, rel_file)
    }

    pub fn implementation(&self, symbol: &str, rel_file: &str) -> Result<String, String> {
        ops::get_implementation(&self.root, &self.symbol_table, symbol, rel_file)
    }

    pub fn structure(&self, depth: usize) -> String {
        self.file_tree.render_tree(depth)
    }

    pub fn peek(
        &self,
        rel_file: &str,
        start_line: usize,
        line_count: usize,
    ) -> Result<PeekResult, TreeSitterError> {
        content::peek(
            &self.root,
            &self.file_tree,
            rel_file,
            start_line,
            line_count,
        )
    }

    pub fn grep(
        &self,
        pattern: &str,
        scope: GrepScope,
        max_matches: usize,
        context_lines: usize,
    ) -> Result<GrepResult, TreeSitterError> {
        content::grep(
            &self.root,
            &self.file_tree,
            pattern,
            scope,
            max_matches,
            context_lines,
        )
    }
}
