#[cfg(feature = "go")]
pub mod go;
#[cfg(feature = "java")]
pub mod java;
#[cfg(feature = "python")]
pub mod python;
#[cfg(feature = "rust")]
pub mod rust;
#[cfg(feature = "scala")]
pub mod scala;
#[cfg(feature = "typescript")]
pub mod typescript;

use crate::file_entry::Language;

#[derive(Clone, Copy)]
pub struct VariableRegexPattern {
    pub regex: &'static str,
    pub capture_group: usize,
}

#[derive(Clone)]
pub struct LanguageConfig {
    pub language: tree_sitter::Language,
    pub symbols_query: &'static str,
    pub callers_query: &'static str,
    pub variables_query: &'static str,
    pub non_code_query: &'static str,
    pub definition_matcher: fn(line: &str, name: &str) -> bool,
    pub test_symbol_matcher: fn(name: &str, file: &str) -> bool,
    pub variable_regex_patterns: &'static [VariableRegexPattern],
    pub variable_name_filter: fn(name: &str) -> bool,
}

pub fn get_language_config(language: Language) -> Option<LanguageConfig> {
    match language {
        #[cfg(feature = "rust")]
        Language::Rust => Some(rust::config()),
        #[cfg(feature = "python")]
        Language::Python => Some(python::config()),
        #[cfg(feature = "typescript")]
        Language::TypeScript => Some(typescript::config()),
        #[cfg(all(feature = "javascript", feature = "typescript"))]
        Language::JavaScript => Some(typescript::javascript_config()),
        #[cfg(feature = "go")]
        Language::Go => Some(go::config()),
        #[cfg(feature = "java")]
        Language::Java => Some(java::config()),
        #[cfg(feature = "scala")]
        Language::Scala => Some(scala::config()),
        _ => None,
    }
}

pub fn is_definition_line(language: Language, line: &str, name: &str) -> bool {
    get_language_config(language).is_some_and(|config| (config.definition_matcher)(line, name))
}

pub fn is_test_symbol(language: Language, name: &str, file: &str) -> bool {
    get_language_config(language).is_some_and(|config| (config.test_symbol_matcher)(name, file))
}
