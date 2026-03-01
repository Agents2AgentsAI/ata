//! LSP server configuration types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Configuration for a single LSP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig {
    /// File extensions this server handles (e.g. `[".rs"]`).
    pub extensions: Vec<String>,

    /// Command to spawn the server (e.g. `["rust-analyzer"]`).
    pub command: Vec<String>,

    /// Environment variables to set for the server process.
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Marker files/patterns used for root discovery (e.g. `["Cargo.toml"]`).
    #[serde(default)]
    pub root_markers: Vec<String>,

    /// Extra initialization options sent during the `initialize` handshake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initialization_options: Option<serde_json::Value>,

    /// When `true`, this server is skipped.
    #[serde(default)]
    pub disabled: bool,

    /// Optional install configuration for auto-installing the server binary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<InstallConfig>,
}

impl LspServerConfig {
    /// Returns `true` if this server handles files with the given extension
    /// (including the leading dot).
    pub fn matches_extension(&self, ext: &str) -> bool {
        self.extensions.iter().any(|e| e == ext)
    }

    /// Returns `true` if this server handles the given file path.
    pub fn matches_path(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| {
                let dotted = format!(".{ext}");
                self.matches_extension(&dotted)
            })
            .unwrap_or(false)
    }

    /// Returns the binary name (first element of `command`).
    pub fn binary_name(&self) -> Option<&str> {
        self.command.first().map(|s| s.as_str())
    }
}

/// Install configuration for an LSP server binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallConfig {
    /// The installation method to use.
    pub method: InstallMethod,
}

/// Supported installation methods.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InstallMethod {
    Cargo {
        #[serde(default)]
        package: Option<String>,
    },
    Npm {
        #[serde(default)]
        package: Option<String>,
    },
    Pip {
        #[serde(default)]
        package: Option<String>,
    },
    Go {
        package_path: String,
    },
    Brew {
        #[serde(default)]
        formula: Option<String>,
    },
    GithubRelease {
        repo: String,
    },
}

impl InstallMethod {
    /// Construct the shell command to install the binary.
    pub fn install_command(&self, binary_name: &str) -> Vec<String> {
        match self {
            InstallMethod::Cargo { package } => {
                let pkg = package.as_deref().unwrap_or(binary_name);
                vec!["cargo".into(), "install".into(), pkg.into()]
            }
            InstallMethod::Npm { package } => {
                let pkg = package.as_deref().unwrap_or(binary_name);
                vec!["npm".into(), "install".into(), "-g".into(), pkg.into()]
            }
            InstallMethod::Pip { package } => {
                let pkg = package.as_deref().unwrap_or(binary_name);
                vec!["pip".into(), "install".into(), pkg.into()]
            }
            InstallMethod::Go { package_path } => {
                vec![
                    "go".into(),
                    "install".into(),
                    format!("{package_path}@latest"),
                ]
            }
            InstallMethod::Brew { formula } => {
                let pkg = formula.as_deref().unwrap_or(binary_name);
                vec!["brew".into(), "install".into(), pkg.into()]
            }
            InstallMethod::GithubRelease { repo } => {
                // Placeholder — real implementation would download from GitHub releases.
                vec![
                    "echo".into(),
                    format!("Download {binary_name} from https://github.com/{repo}/releases"),
                ]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn test_config() -> LspServerConfig {
        LspServerConfig {
            extensions: vec![".rs".into(), ".toml".into()],
            command: vec!["rust-analyzer".into()],
            env: HashMap::new(),
            root_markers: vec!["Cargo.toml".into()],
            initialization_options: None,
            disabled: false,
            install: None,
        }
    }

    #[test]
    fn matches_extension_with_dot() {
        let c = test_config();
        assert!(c.matches_extension(".rs"));
        assert!(c.matches_extension(".toml"));
        assert!(!c.matches_extension(".py"));
        assert!(!c.matches_extension("rs")); // no dot
    }

    #[test]
    fn matches_path_extracts_extension() {
        let c = test_config();
        assert!(c.matches_path(Path::new("src/main.rs")));
        assert!(c.matches_path(Path::new("/abs/Cargo.toml")));
        assert!(!c.matches_path(Path::new("readme.md")));
    }

    #[test]
    fn matches_path_no_extension() {
        let c = test_config();
        assert!(!c.matches_path(Path::new("Makefile")));
    }

    #[test]
    fn binary_name_returns_first_command() {
        let c = test_config();
        assert_eq!(c.binary_name(), Some("rust-analyzer"));
    }

    #[test]
    fn binary_name_empty_command() {
        let c = LspServerConfig {
            command: Vec::new(),
            ..test_config()
        };
        assert_eq!(c.binary_name(), None);
    }

    #[test]
    fn install_command_cargo() {
        let m = InstallMethod::Cargo { package: None };
        assert_eq!(m.install_command("ra"), vec!["cargo", "install", "ra"]);

        let m = InstallMethod::Cargo { package: Some("rust-analyzer".into()) };
        assert_eq!(m.install_command("ra"), vec!["cargo", "install", "rust-analyzer"]);
    }

    #[test]
    fn install_command_npm() {
        let m = InstallMethod::Npm { package: Some("ts-server".into()) };
        assert_eq!(m.install_command("ts"), vec!["npm", "install", "-g", "ts-server"]);
    }

    #[test]
    fn install_command_go() {
        let m = InstallMethod::Go { package_path: "golang.org/x/tools/gopls".into() };
        assert_eq!(
            m.install_command("gopls"),
            vec!["go", "install", "golang.org/x/tools/gopls@latest"]
        );
    }

    #[test]
    fn install_command_brew() {
        let m = InstallMethod::Brew { formula: Some("llvm".into()) };
        assert_eq!(m.install_command("clangd"), vec!["brew", "install", "llvm"]);
    }
}
