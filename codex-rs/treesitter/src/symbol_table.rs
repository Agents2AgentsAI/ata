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

    pub fn insert(&self, symbol: Symbol) {
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
        let keys = self.by_file.get(file)?;
        keys.iter().find_map(|key| {
            self.symbols
                .get(key)
                .filter(|entry| entry.value().name == name)
                .map(|entry| entry.value().clone())
        })
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
        let query_lower = query.to_lowercase();
        let mut ranked: Vec<(Symbol, bool, bool)> = self
            .symbols
            .iter()
            .filter_map(|entry| {
                let symbol = entry.value();
                let name_lower = symbol.name.to_lowercase();
                if name_lower.contains(&query_lower) {
                    let exact = name_lower == query_lower;
                    let prefix = name_lower.starts_with(&query_lower);
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
}

fn is_test_symbol(symbol: &Symbol) -> bool {
    queries::is_test_symbol(symbol.language, &symbol.name, &symbol.file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::SymbolKind;

    fn make_symbol(name: &str, file: &str, language: Language) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            file: file.to_string(),
            byte_range: (0, 10),
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
}
