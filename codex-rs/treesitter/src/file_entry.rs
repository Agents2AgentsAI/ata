use serde::Deserialize;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Go,
    Java,
    Scala,
    Other,
}

impl Language {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "rust" => Some(Self::Rust),
            "python" => Some(Self::Python),
            "typescript" | "ts" => Some(Self::TypeScript),
            "javascript" | "js" => Some(Self::JavaScript),
            "go" | "golang" => Some(Self::Go),
            "java" => Some(Self::Java),
            "scala" => Some(Self::Scala),
            "other" => Some(Self::Other),
            _ => None,
        }
    }

    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "rs" => Self::Rust,
            "py" | "pyi" => Self::Python,
            "ts" | "tsx" => Self::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "go" => Self::Go,
            "java" => Self::Java,
            "scala" | "sc" => Self::Scala,
            _ => Self::Other,
        }
    }

    pub fn from_path(path: &Path) -> Self {
        path.extension()
            .and_then(|e| e.to_str())
            .map(Self::from_extension)
            .unwrap_or(Self::Other)
    }

    pub fn has_tree_sitter_support(self) -> bool {
        matches!(
            self,
            Self::Rust
                | Self::Python
                | Self::TypeScript
                | Self::JavaScript
                | Self::Go
                | Self::Java
                | Self::Scala
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileMark {
    Test,
    Docs,
    Config,
    Generated,
    EntryPoint,
    Custom(String),
}

impl FileMark {
    pub fn from_input(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        match trimmed.to_ascii_lowercase().as_str() {
            "test" => Some(Self::Test),
            "docs" => Some(Self::Docs),
            "config" => Some(Self::Config),
            "generated" => Some(Self::Generated),
            "entrypoint" | "entry_point" => Some(Self::EntryPoint),
            _ => Some(Self::Custom(trimmed.to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub rel_path: String,
    pub size: u64,
    pub language: Language,
    pub definition: Option<String>,
    pub marks: Vec<FileMark>,
}

impl FileEntry {
    pub fn new(rel_path: String, size: u64) -> Self {
        let language = Language::from_path(Path::new(&rel_path));
        Self {
            rel_path,
            size,
            language,
            definition: None,
            marks: Vec::new(),
        }
    }
}
