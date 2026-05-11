use std::collections::HashSet;

use dashmap::DashMap;
use dashmap::DashSet;

#[cfg(test)]
use crate::file_entry::Language;
use crate::queries;
use crate::symbol::Symbol;

/// Thread-safe symbol table with secondary indices for fast lookups.
#[derive(Debug)]
pub struct SymbolTable {
    symbols: DashMap<String, Symbol>,
    by_name: DashMap<String, HashSet<String>>,
    by_file: DashMap<String, HashSet<String>>,
    test_keys: DashSet<String>,
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            symbols: DashMap::new(),
            by_name: DashMap::new(),
            by_file: DashMap::new(),
            test_keys: DashSet::new(),
        }
    }

    pub fn make_key(file: &str, name: &str, byte_start: usize) -> String {
        format!("{file}::{name}@{byte_start}")
    }

    pub fn insert(&self, mut symbol: Symbol) {
        symbol.ensure_name_lower();
        let key = Self::make_key(&symbol.file, &symbol.name, symbol.byte_range.0);

        self.by_name
            .entry(symbol.name.clone())
            .or_default()
            .insert(key.clone());
        self.by_file
            .entry(symbol.file.clone())
            .or_default()
            .insert(key.clone());
        if is_test_symbol(&symbol) {
            self.test_keys.insert(key.clone());
        }

        self.symbols.insert(key, symbol);
    }

    pub fn remove_file(&self, rel_path: &str) {
        if let Some((_, keys)) = self.by_file.remove(rel_path) {
            for key in keys {
                if let Some((_, symbol)) = self.symbols.remove(&key) {
                    self.test_keys.remove(&key);
                    if let Some(mut by_name_keys) = self.by_name.get_mut(&symbol.name) {
                        by_name_keys.remove(&key);
                    }
                }
            }
        }
    }

    pub fn get(&self, file: &str, name: &str) -> Option<Symbol> {
        let mut matches = self.matching_symbols_in_file(file, name);
        if matches.len() == 1 {
            matches.pop()
        } else {
            None
        }
    }

    pub fn get_at(&self, file: &str, name: &str, byte_offset: usize) -> Option<Symbol> {
        let key = self.lookup_key_at(file, name, byte_offset)?;
        self.symbols.get(&key).map(|entry| entry.value().clone())
    }

    pub fn set_definition(
        &self,
        file: &str,
        name: &str,
        definition: &str,
        overwrite: bool,
    ) -> Result<(), String> {
        let Some(keys) = self.by_file.get(file) else {
            return Err(format!("file '{file}' not found in index"));
        };

        for key in keys.iter() {
            if let Some(mut symbol) = self.symbols.get_mut(key) {
                if symbol.name != name {
                    continue;
                }
                if symbol.definition.is_some() && !overwrite {
                    return Err(format!(
                        "symbol '{name}' in '{file}' already has a definition"
                    ));
                }
                symbol.definition = Some(definition.to_string());
                return Ok(());
            }
        }

        Err(format!("symbol '{name}' not found in '{file}'"))
    }

    pub fn set_definition_at(
        &self,
        file: &str,
        name: &str,
        byte_offset: usize,
        definition: &str,
        overwrite: bool,
    ) -> Result<(), String> {
        let Some(key) = self.lookup_key_at(file, name, byte_offset) else {
            return Err(format!(
                "symbol '{name}' not found at byte offset {byte_offset} in '{file}'"
            ));
        };

        self.set_definition_by_key(&key, definition, overwrite)
    }

    pub fn set_definition_by_key(
        &self,
        key: &str,
        definition: &str,
        overwrite: bool,
    ) -> Result<(), String> {
        let Some(mut symbol) = self.symbols.get_mut(key) else {
            return Err(format!("symbol key '{key}' not found in index"));
        };
        if symbol.definition.is_some() && !overwrite {
            return Err(format!("symbol key '{key}' already has a definition"));
        }
        symbol.definition = Some(definition.to_string());
        Ok(())
    }

    pub fn symbols_in_file(&self, rel_path: &str) -> Vec<Symbol> {
        let Some(keys) = self.by_file.get(rel_path) else {
            return Vec::new();
        };
        keys.iter()
            .filter_map(|key| self.symbols.get(key).map(|entry| entry.value().clone()))
            .collect()
    }

    pub fn matching_symbols_in_file(&self, file: &str, name: &str) -> Vec<Symbol> {
        let Some(keys) = self.by_file.get(file) else {
            return Vec::new();
        };

        let mut matches: Vec<Symbol> = keys
            .iter()
            .filter_map(|key| self.symbols.get(key).map(|entry| entry.value().clone()))
            .filter(|symbol| symbol.name == name)
            .collect();
        matches.sort_by(|a, b| {
            a.line_range
                .0
                .cmp(&b.line_range.0)
                .then(a.byte_range.0.cmp(&b.byte_range.0))
        });
        matches
    }

    pub fn all_symbols(&self) -> Vec<Symbol> {
        self.symbols
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    pub fn test_symbols(&self) -> Vec<Symbol> {
        self.test_keys
            .iter()
            .filter_map(|key| {
                let key = key.key().clone();
                self.symbols.get(&key).map(|entry| entry.value().clone())
            })
            .collect()
    }

    pub fn all_symbol_definitions(&self) -> Vec<(String, String)> {
        self.symbols
            .iter()
            .filter_map(|entry| {
                entry
                    .value()
                    .definition
                    .as_ref()
                    .map(|definition| (entry.key().clone(), definition.clone()))
            })
            .collect()
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<Symbol> {
        let query = strip_kind_prefix(query);
        let query_lower = query.to_lowercase();
        let mut ranked: Vec<(Symbol, bool, bool)> = self
            .symbols
            .iter()
            .filter_map(|entry| {
                let symbol = entry.value();
                if symbol.name_lower.contains(&query_lower) {
                    let exact = symbol.name_lower == query_lower;
                    let prefix = symbol.name_lower.starts_with(&query_lower);
                    Some((symbol.clone(), exact, prefix))
                } else {
                    None
                }
            })
            .collect();

        ranked.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then(b.2.cmp(&a.2))
                .then(a.0.file.cmp(&b.0.file))
                .then(a.0.line_range.0.cmp(&b.0.line_range.0))
        });

        if ranked.len() > limit {
            ranked.truncate(limit);
        }
        ranked.into_iter().map(|(symbol, _, _)| symbol).collect()
    }

    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    fn lookup_key_at(&self, file: &str, name: &str, byte_offset: usize) -> Option<String> {
        let exact_key = Self::make_key(file, name, byte_offset);
        if self.symbols.contains_key(&exact_key) {
            return Some(exact_key);
        }

        let keys = self.by_file.get(file)?;
        let mut best_match: Option<(String, usize, usize)> = None;

        for key in keys.iter() {
            let Some(entry) = self.symbols.get(key) else {
                continue;
            };
            let symbol = entry.value();
            if symbol.name != name {
                continue;
            }

            let (start, end) = symbol.byte_range;
            let contains_offset = if start == end {
                byte_offset == start
            } else {
                start <= byte_offset && byte_offset < end
            };
            if !contains_offset {
                continue;
            }

            let span = end.saturating_sub(start);
            match &best_match {
                Some((_, best_span, best_start))
                    if span > *best_span || (span == *best_span && start <= *best_start) => {}
                _ => {
                    best_match = Some((key.clone(), span, start));
                }
            }
        }

        best_match.map(|(key, _, _)| key)
    }
}

fn is_test_symbol(symbol: &Symbol) -> bool {
    queries::is_test_symbol(symbol.language, &symbol.name, &symbol.file)
}

fn strip_kind_prefix(query: &str) -> &str {
    const PREFIXES: &[&str] = &[
        "class ",
        "def ",
        "fn ",
        "func ",
        "function ",
        "struct ",
        "enum ",
        "trait ",
        "interface ",
        "type ",
        "const ",
        "var ",
        "let ",
    ];

    for prefix in PREFIXES {
        if query.len() >= prefix.len() && query[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return query[prefix.len()..].trim_start();
        }
    }

    query
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::SymbolKind;

    fn make_symbol(name: &str, file: &str, language: Language) -> Symbol {
        make_symbol_at(name, file, language, 0, 10)
    }

    fn make_symbol_at(
        name: &str,
        file: &str,
        language: Language,
        byte_start: usize,
        byte_end: usize,
    ) -> Symbol {
        Symbol {
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            kind: SymbolKind::Function,
            file: file.to_string(),
            byte_range: (byte_start, byte_end),
            line_range: (1, 1),
            language,
            signature: name.to_string(),
            definition: None,
            parent: None,
        }
    }

    #[test]
    fn tracks_test_symbols_in_secondary_index() {
        let table = SymbolTable::new();
        table.insert(make_symbol(
            "test_works",
            "src/test_example.py",
            Language::Python,
        ));
        table.insert(make_symbol("helper", "src/main.py", Language::Python));

        let test_names: std::collections::HashSet<String> = table
            .test_symbols()
            .into_iter()
            .map(|symbol| symbol.name)
            .collect();
        assert!(test_names.contains("test_works"));
        assert!(!test_names.contains("helper"));
    }

    #[test]
    fn remove_file_clears_test_symbol_entries() {
        let table = SymbolTable::new();
        table.insert(make_symbol("TestThing", "pkg/foo_test.go", Language::Go));
        assert_eq!(table.test_symbols().len(), 1);

        table.remove_file("pkg/foo_test.go");
        assert!(table.test_symbols().is_empty());
    }

    #[test]
    fn search_ignores_kind_prefixes() {
        let table = SymbolTable::new();
        let mut symbol = make_symbol("Memory", "src/memory.py", Language::Python);
        symbol.kind = SymbolKind::Class;
        table.insert(symbol);

        let results = table.search("class Memory", 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Memory");
    }

    #[test]
    fn get_at_prefers_exact_byte_start_key_for_duplicate_names() {
        let table = SymbolTable::new();
        table.insert(make_symbol_at(
            "duplicate",
            "src/lib.rs",
            Language::Rust,
            12,
            48,
        ));
        table.insert(make_symbol_at(
            "duplicate",
            "src/lib.rs",
            Language::Rust,
            96,
            140,
        ));

        let symbol = table
            .get_at("src/lib.rs", "duplicate", 96)
            .expect("exact key lookup should resolve the later symbol");

        assert_eq!(symbol.byte_range, (96, 140));
    }

    #[test]
    fn get_at_uses_smallest_enclosing_range_when_offset_is_inside_symbol() {
        let table = SymbolTable::new();
        table.insert(make_symbol_at(
            "duplicate",
            "src/lib.rs",
            Language::Rust,
            10,
            90,
        ));
        table.insert(make_symbol_at(
            "duplicate",
            "src/lib.rs",
            Language::Rust,
            30,
            60,
        ));

        let symbol = table
            .get_at("src/lib.rs", "duplicate", 40)
            .expect("enclosing symbol lookup should resolve");

        assert_eq!(symbol.byte_range, (30, 60));
    }

    #[test]
    fn get_returns_none_when_file_contains_duplicate_symbol_names() {
        let table = SymbolTable::new();
        table.insert(make_symbol_at(
            "duplicate",
            "src/lib.rs",
            Language::Rust,
            12,
            48,
        ));
        table.insert(make_symbol_at(
            "duplicate",
            "src/lib.rs",
            Language::Rust,
            96,
            140,
        ));

        assert!(table.get("src/lib.rs", "duplicate").is_none());
    }
}
