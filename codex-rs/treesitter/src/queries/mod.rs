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

#[derive(Clone)]
pub struct LanguageConfig {
    pub language: tree_sitter::Language,
    pub symbols_query: &'static str,
    pub callers_query: &'static str,
    pub variables_query: &'static str,
    pub non_code_query: &'static str,
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
