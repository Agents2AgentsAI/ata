use std::path::Path;
use std::path::PathBuf;

use ignore::gitignore::Gitignore;
use ignore::gitignore::GitignoreBuilder;

use crate::config::ProjectIndexConfig;
use crate::content::GrepResult;
use crate::content::GrepScope;
use crate::content::PeekResult;
use crate::content::{self};
use crate::error::TreeSitterError;
use crate::file_entry::FileEntry;
use crate::file_entry::FileMark;
use crate::file_tree::FileTree;
use crate::ops::CallerInfo;
use crate::ops::TestInfo;
use crate::ops::VariableInfo;
use crate::ops::{self};
use crate::parser;
use crate::symbol::Symbol;
use crate::symbol_table::SymbolTable;
use crate::walker;

pub struct ProjectIndex {
    root: PathBuf,
    config: ProjectIndexConfig,
    extra_ignores: Option<Gitignore>,
    file_tree: FileTree,
    symbol_table: SymbolTable,
}

impl ProjectIndex {
    pub fn new(root: PathBuf) -> Result<Self, TreeSitterError> {
        Self::new_with_config(root, ProjectIndexConfig::default())
    }

    pub fn new_with_config(
        root: PathBuf,
        config: ProjectIndexConfig,
    ) -> Result<Self, TreeSitterError> {
        let extra_ignores = build_extra_ignores(&root, &config.ignore_patterns)?;
        let file_tree = FileTree::new();
        let symbol_table = SymbolTable::new();

        walker::scan_directory_with_config(&root, &file_tree, &config, extra_ignores.as_ref())?;
        parser::extract_all_symbols(&root, &file_tree, &symbol_table, &config)?;

        Ok(Self {
            root,
            config,
            extra_ignores,
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

    pub fn config(&self) -> &ProjectIndexConfig {
        &self.config
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
            let language = crate::file_entry::Language::from_path(path);

            if metadata.len() > self.config.max_file_size
                || !self.config.is_language_enabled(language)
                || self
                    .extra_ignores
                    .as_ref()
                    .is_some_and(|ignores| ignores.matched(path, false).is_ignore())
            {
                self.file_tree.remove(&rel_path);
                self.symbol_table.remove_file(&rel_path);
                return Ok(());
            }

            self.file_tree
                .insert(FileEntry::new(rel_path.clone(), metadata.len()));
            parser::reindex_file(
                &self.root,
                &self.symbol_table,
                &rel_path,
                language,
                &self.config,
            )?;
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

    pub fn define_symbol(
        &self,
        symbol: &str,
        rel_file: &str,
        definition: &str,
        overwrite: bool,
    ) -> Result<(), String> {
        self.symbol_table
            .set_definition(rel_file, symbol, definition, overwrite)
    }

    pub fn define_file(
        &self,
        rel_file: &str,
        definition: &str,
        overwrite: bool,
    ) -> Result<(), String> {
        self.file_tree.define_file(rel_file, definition, overwrite)
    }

    pub fn mark_file(&self, rel_file: &str, mark: FileMark) -> Result<(), String> {
        self.file_tree.mark_file(rel_file, mark)
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

impl std::fmt::Debug for ProjectIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectIndex")
            .field("root", &self.root)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

fn build_extra_ignores(
    root: &Path,
    patterns: &[String],
) -> Result<Option<Gitignore>, TreeSitterError> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GitignoreBuilder::new(root);
    for pattern in patterns {
        builder
            .add_line(None, pattern)
            .map_err(|error| TreeSitterError::InvalidIgnorePattern(error.to_string()))?;
    }

    builder
        .build()
        .map(Some)
        .map_err(|error| TreeSitterError::InvalidIgnorePattern(error.to_string()))
}
