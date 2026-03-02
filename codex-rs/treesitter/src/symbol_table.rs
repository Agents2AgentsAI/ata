use std::collections::HashSet;

use dashmap::DashMap;

use crate::symbol::Symbol;

/// Thread-safe symbol table with secondary indices for fast lookups.
#[derive(Debug)]
pub struct SymbolTable {
    symbols: DashMap<String, Symbol>,
    by_name: DashMap<String, HashSet<String>>,
    by_file: DashMap<String, HashSet<String>>,
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

        self.symbols.insert(key, symbol);
    }

    pub fn remove_file(&self, rel_path: &str) {
        if let Some((_, keys)) = self.by_file.remove(rel_path) {
            for key in keys {
                if let Some((_, symbol)) = self.symbols.remove(&key)
                    && let Some(mut by_name_keys) = self.by_name.get_mut(&symbol.name)
                {
                    by_name_keys.remove(&key);
                    if by_name_keys.is_empty() {
                        drop(by_name_keys);
                        self.by_name.remove(&symbol.name);
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

    pub fn search(&self, query: &str, limit: usize) -> Vec<Symbol> {
        let query_lower = query.to_lowercase();
        let mut out: Vec<Symbol> = self
            .symbols
            .iter()
            .filter_map(|entry| {
                let symbol = entry.value();
                let name_lower = symbol.name.to_lowercase();
                if name_lower.contains(&query_lower) {
                    Some(symbol.clone())
                } else {
                    None
                }
            })
            .collect();

        out.sort_by(|a, b| {
            let a_name = a.name.to_lowercase();
            let b_name = b.name.to_lowercase();
            let a_exact = a_name == query_lower;
            let b_exact = b_name == query_lower;
            let a_prefix = a_name.starts_with(&query_lower);
            let b_prefix = b_name.starts_with(&query_lower);

            b_exact
                .cmp(&a_exact)
                .then(b_prefix.cmp(&a_prefix))
                .then(a.file.cmp(&b.file))
                .then(a.line_range.0.cmp(&b.line_range.0))
        });

        if out.len() > limit {
            out.truncate(limit);
        }
        out
    }

    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }
}
