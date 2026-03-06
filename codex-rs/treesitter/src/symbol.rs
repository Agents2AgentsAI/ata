use serde::Deserialize;
use serde::Serialize;

use crate::file_entry::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Constant,
    Variable,
    Type,
    Module,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    #[serde(default)]
    pub name_lower: String,
    pub kind: SymbolKind,
    pub file: String,
    pub byte_range: (usize, usize),
    pub line_range: (usize, usize),
    pub language: Language,
    pub signature: String,
    pub definition: Option<String>,
    pub parent: Option<String>,
}

impl Symbol {
    pub fn ensure_name_lower(&mut self) {
        if self.name_lower.is_empty() {
            self.name_lower = self.name.to_lowercase();
        }
    }
}
