//! LSP server configuration types.

use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

/// Strategy for refining the project root after initial marker-based discovery.
#[derive(Debug, Clone, Default)]
pub enum RootStrategy {
    /// Use the nearest marker match as-is (default behavior).
    #[default]
    NearestMarker,
    /// Walk up from the initial root looking for `marker_file` containing
    /// `content_pattern`. Returns the NEAREST match (not outermost).
    /// Used for Cargo `[workspace]` detection.
    WalkUpForContent {
        marker_file: &'static str,
        content_pattern: &'static str,
    },
    /// Walk up looking for `preferred_marker`. Returns the FIRST ancestor
    /// directory containing it. Used for `go.work` detection.
    PreferAncestorMarker { preferred_marker: &'static str },
}

/// Hook executed after root discovery to modify server configuration.
#[derive(Debug, Clone, Default)]
pub enum PostRootHook {
    #[default]
    None,
    /// Probe for a Python virtual environment and inject `pythonPath` into
    /// initialization options.
    PythonVenvProbe,
}

/// Configuration for a single LSP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig {
    /// File extensions this server handles (e.g. `[".rs"]`).
    pub extensions: Vec<String>,

    /// Command to spawn the server (e.g. `["rust-analyzer"]`).
    pub command: Vec<String>,

    /// Fallback command vectors attempted when `command` is unavailable.
    /// Useful for platform/toolchain alternatives, e.g. `xcrun sourcekit-lsp`.
    #[serde(default)]
    pub command_candidates: Vec<Vec<String>>,

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

    /// Strategy for refining the project root after initial marker discovery.
    #[serde(skip)]
    pub root_strategy: RootStrategy,

    /// Hook to run after root discovery (e.g. Python venv probing).
    #[serde(skip)]
    pub post_root_hook: PostRootHook,
}

impl LspServerConfig {
    /// Construct a server config with sensible defaults.
    pub fn new(extensions: Vec<String>, command: Vec<String>, root_markers: Vec<String>) -> Self {
        Self {
            extensions,
            command,
            command_candidates: Vec::new(),
            env: HashMap::new(),
            root_markers,
            initialization_options: None,
            disabled: false,
            install: None,
            root_strategy: RootStrategy::default(),
            post_root_hook: PostRootHook::default(),
        }
    }

    pub fn with_command_candidates(mut self, command_candidates: Vec<Vec<String>>) -> Self {
        self.command_candidates = command_candidates;
        self
    }

    pub fn with_initialization_options(mut self, options: serde_json::Value) -> Self {
        self.initialization_options = Some(options);
        self
    }

    pub fn with_install(mut self, method: InstallMethod) -> Self {
        self.install = Some(InstallConfig { method });
        self
    }

    pub fn with_root_strategy(mut self, strategy: RootStrategy) -> Self {
        self.root_strategy = strategy;
        self
    }

    pub fn with_post_root_hook(mut self, hook: PostRootHook) -> Self {
        self.post_root_hook = hook;
        self
    }

    /// Returns `true` if this server handles files with the given extension
    /// (including the leading dot).
    pub fn matches_extension(&self, ext: &str) -> bool {
        self.extensions.iter().any(|e| e == ext)
    }

    /// Returns `true` if this server handles the given file path.
    ///
    /// Extension entries (starting with `.`) are matched against the file extension.
    /// Basename entries (no leading `.`) are matched against the full filename,
    /// enabling extensionless files like `Dockerfile` and `Makefile`.
    pub fn matches_path(&self, path: &Path) -> bool {
        // Try extension match first.
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let dotted = format!(".{ext}");
            if self.matches_extension(&dotted) {
                return true;
            }
        }
        // Fall back to basename match for extensionless files or entries like "Dockerfile".
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            return self
                .extensions
                .iter()
                .any(|e| !e.starts_with('.') && e == name);
        }
        false
    }

    /// Returns the binary name (first element of `command`).
    pub fn binary_name(&self) -> Option<&str> {
        self.command.first().map(|s| s.as_str())
    }

    /// Iterate startup command variants (primary command first, then fallbacks).
    pub fn command_variants(&self) -> impl Iterator<Item = &[String]> {
        std::iter::once(self.command.as_slice())
            .chain(self.command_candidates.iter().map(std::vec::Vec::as_slice))
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
    RustupComponent {
        #[serde(default)]
        component: Option<String>,
    },
    Npm {
        #[serde(default)]
        packages: Vec<String>,
    },
    DotnetTool {
        #[serde(default)]
        package: Option<String>,
    },
    Gem {
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
}

impl InstallMethod {
    /// A short method label for diagnostics and user-facing messages.
    pub fn label(&self) -> &'static str {
        match self {
            InstallMethod::Cargo { .. } => "cargo",
            InstallMethod::RustupComponent { .. } => "rustup_component",
            InstallMethod::Npm { .. } => "npm",
            InstallMethod::DotnetTool { .. } => "dotnet_tool",
            InstallMethod::Gem { .. } => "gem",
            InstallMethod::Pip { .. } => "pip",
            InstallMethod::Go { .. } => "go",
            InstallMethod::Brew { .. } => "brew",
        }
    }

    /// Construct the shell command to install the binary.
    pub fn install_command(&self, binary_name: &str) -> Vec<String> {
        match self {
            InstallMethod::Cargo { package } => {
                let pkg = package.as_deref().unwrap_or(binary_name);
                vec!["cargo".into(), "install".into(), pkg.into()]
            }
            InstallMethod::RustupComponent { component } => {
                let comp = component.as_deref().unwrap_or(binary_name);
                vec![
                    "rustup".into(),
                    "component".into(),
                    "add".into(),
                    comp.into(),
                ]
            }
            InstallMethod::Npm { packages } => {
                let mut cmd = vec!["npm".into(), "install".into(), "-g".into()];
                if packages.is_empty() {
                    cmd.push(binary_name.into());
                } else {
                    cmd.extend(packages.iter().map(|p| p.clone()));
                }
                cmd
            }
            InstallMethod::DotnetTool { package } => {
                let pkg = package.as_deref().unwrap_or(binary_name);
                vec!["dotnet".into(), "tool".into(), "install".into(), pkg.into()]
            }
            InstallMethod::Gem { package } => {
                let pkg = package.as_deref().unwrap_or(binary_name);
                vec!["gem".into(), "install".into(), pkg.into()]
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn test_config() -> LspServerConfig {
        LspServerConfig::new(
            vec![".rs".into(), ".toml".into()],
            vec!["rust-analyzer".into()],
            vec!["Cargo.toml".into()],
        )
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
    fn matches_path_basename_entry() {
        let c = LspServerConfig::new(
            vec![".dockerfile".into(), "Dockerfile".into()],
            vec!["docker-langserver".into()],
            vec![],
        );
        // Extensionless file matched by basename
        assert!(c.matches_path(Path::new("/project/Dockerfile")));
        // Extension-based match still works
        assert!(c.matches_path(Path::new("app.dockerfile")));
        // Unrelated extensionless file does not match
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
    fn command_variants_includes_primary_then_fallbacks() {
        let mut c = test_config();
        c.command_candidates = vec![
            vec!["xcrun".into(), "sourcekit-lsp".into()],
            vec!["/opt/custom/bin/sourcekit-lsp".into()],
        ];
        let variants: Vec<Vec<String>> = c.command_variants().map(|v| v.to_vec()).collect();
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0], vec!["rust-analyzer"]);
        assert_eq!(variants[1], vec!["xcrun", "sourcekit-lsp"]);
        assert_eq!(variants[2], vec!["/opt/custom/bin/sourcekit-lsp"]);
    }

    #[test]
    fn install_command_cargo() {
        let m = InstallMethod::Cargo { package: None };
        assert_eq!(m.install_command("ra"), vec!["cargo", "install", "ra"]);

        let m = InstallMethod::Cargo {
            package: Some("rust-analyzer".into()),
        };
        assert_eq!(
            m.install_command("ra"),
            vec!["cargo", "install", "rust-analyzer"]
        );
    }

    #[test]
    fn install_command_npm() {
        let m = InstallMethod::Npm {
            packages: vec!["ts-server".into()],
        };
        assert_eq!(
            m.install_command("ts"),
            vec!["npm", "install", "-g", "ts-server"]
        );
    }

    #[test]
    fn install_command_npm_multi_package() {
        let m = InstallMethod::Npm {
            packages: vec!["typescript-language-server".into(), "typescript".into()],
        };
        assert_eq!(
            m.install_command("typescript-language-server"),
            vec![
                "npm",
                "install",
                "-g",
                "typescript-language-server",
                "typescript"
            ]
        );
    }

    #[test]
    fn install_command_npm_empty_fallback() {
        let m = InstallMethod::Npm { packages: vec![] };
        assert_eq!(
            m.install_command("my-server"),
            vec!["npm", "install", "-g", "my-server"]
        );
    }

    #[test]
    fn install_command_dotnet_tool() {
        let m = InstallMethod::DotnetTool {
            package: Some("csharp-ls".into()),
        };
        assert_eq!(
            m.install_command("csharp-ls"),
            vec!["dotnet", "tool", "install", "csharp-ls"]
        );
    }

    #[test]
    fn install_command_gem() {
        let m = InstallMethod::Gem {
            package: Some("rubocop".into()),
        };
        assert_eq!(
            m.install_command("rubocop"),
            vec!["gem", "install", "rubocop"]
        );
    }

    #[test]
    fn install_command_rustup_component_default_binary() {
        let m = InstallMethod::RustupComponent { component: None };
        assert_eq!(
            m.install_command("rust-analyzer"),
            vec!["rustup", "component", "add", "rust-analyzer"]
        );
    }

    #[test]
    fn install_command_rustup_component_explicit_component() {
        let m = InstallMethod::RustupComponent {
            component: Some("rust-analyzer".into()),
        };
        assert_eq!(
            m.install_command("ra-proxy"),
            vec!["rustup", "component", "add", "rust-analyzer"]
        );
    }

    #[test]
    fn install_command_go() {
        let m = InstallMethod::Go {
            package_path: "golang.org/x/tools/gopls".into(),
        };
        assert_eq!(
            m.install_command("gopls"),
            vec!["go", "install", "golang.org/x/tools/gopls@latest"]
        );
    }

    #[test]
    fn install_command_brew() {
        let m = InstallMethod::Brew {
            formula: Some("llvm".into()),
        };
        assert_eq!(m.install_command("clangd"), vec!["brew", "install", "llvm"]);
    }
}
