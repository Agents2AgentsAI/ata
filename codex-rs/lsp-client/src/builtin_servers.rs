//! Built-in LSP server configurations for common languages.

use crate::server_config::InstallConfig;
use crate::server_config::InstallMethod;
use crate::server_config::LspServerConfig;

/// Returns the 6 built-in server configurations (v1).
pub fn builtin_servers() -> Vec<(&'static str, LspServerConfig)> {
    vec![
        rust_analyzer(),
        typescript_language_server(),
        gopls(),
        pyright(),
        clangd(),
        sourcekit_lsp(),
    ]
}

fn rust_analyzer() -> (&'static str, LspServerConfig) {
    (
        "rust-analyzer",
        LspServerConfig {
            extensions: vec![".rs".into()],
            command: vec!["rust-analyzer".into()],
            env: Default::default(),
            root_markers: vec!["Cargo.toml".into()],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::RustupComponent {
                    component: Some("rust-analyzer".into()),
                },
            }),
        },
    )
}

fn typescript_language_server() -> (&'static str, LspServerConfig) {
    (
        "typescript-language-server",
        LspServerConfig {
            extensions: vec![
                ".ts".into(),
                ".tsx".into(),
                ".js".into(),
                ".jsx".into(),
                ".mjs".into(),
                ".cjs".into(),
            ],
            command: vec!["typescript-language-server".into(), "--stdio".into()],
            env: Default::default(),
            root_markers: vec![
                "tsconfig.json".into(),
                "jsconfig.json".into(),
                "package.json".into(),
            ],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::Npm {
                    package: Some("typescript-language-server".into()),
                },
            }),
        },
    )
}

fn gopls() -> (&'static str, LspServerConfig) {
    (
        "gopls",
        LspServerConfig {
            extensions: vec![".go".into()],
            command: vec!["gopls".into()],
            env: Default::default(),
            root_markers: vec!["go.mod".into(), "go.sum".into()],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::Go {
                    package_path: "golang.org/x/tools/gopls".into(),
                },
            }),
        },
    )
}

fn pyright() -> (&'static str, LspServerConfig) {
    (
        "pyright",
        LspServerConfig {
            extensions: vec![".py".into(), ".pyi".into()],
            command: vec!["pyright-langserver".into(), "--stdio".into()],
            env: Default::default(),
            root_markers: vec![
                "pyproject.toml".into(),
                "setup.py".into(),
                "setup.cfg".into(),
                "requirements.txt".into(),
                "pyrightconfig.json".into(),
            ],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::Pip {
                    package: Some("pyright".into()),
                },
            }),
        },
    )
}

fn clangd() -> (&'static str, LspServerConfig) {
    (
        "clangd",
        LspServerConfig {
            extensions: vec![
                ".c".into(),
                ".cpp".into(),
                ".cc".into(),
                ".cxx".into(),
                ".h".into(),
                ".hpp".into(),
            ],
            command: vec!["clangd".into()],
            env: Default::default(),
            root_markers: vec![
                "compile_commands.json".into(),
                ".clangd".into(),
                "CMakeLists.txt".into(),
            ],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::Brew {
                    formula: Some("llvm".into()),
                },
            }),
        },
    )
}

fn sourcekit_lsp() -> (&'static str, LspServerConfig) {
    (
        "sourcekit-lsp",
        LspServerConfig {
            extensions: vec![".swift".into()],
            command: vec!["sourcekit-lsp".into()],
            env: Default::default(),
            root_markers: vec!["Package.swift".into(), "*.xcodeproj".into()],
            initialization_options: None,
            disabled: false,
            // sourcekit-lsp ships with Xcode — no separate install.
            install: None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_servers_returns_six() {
        let servers = builtin_servers();
        assert_eq!(servers.len(), 6);
    }

    #[test]
    fn all_servers_have_unique_ids() {
        let servers = builtin_servers();
        let ids: Vec<&str> = servers.iter().map(|(id, _)| *id).collect();
        let unique: std::collections::HashSet<&&str> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "duplicate server IDs");
    }

    #[test]
    fn all_servers_have_at_least_one_extension() {
        for (id, config) in builtin_servers() {
            assert!(!config.extensions.is_empty(), "{id} has no extensions");
        }
    }

    #[test]
    fn all_servers_have_non_empty_command() {
        for (id, config) in builtin_servers() {
            assert!(!config.command.is_empty(), "{id} has empty command");
        }
    }

    #[test]
    fn all_servers_have_root_markers() {
        for (id, config) in builtin_servers() {
            assert!(!config.root_markers.is_empty(), "{id} has no root markers");
        }
    }

    #[test]
    fn no_servers_disabled_by_default() {
        for (id, config) in builtin_servers() {
            assert!(!config.disabled, "{id} should not be disabled by default");
        }
    }

    #[test]
    fn rust_analyzer_matches_rs_files() {
        let (_, config) = rust_analyzer();
        assert!(config.matches_extension(".rs"));
        assert!(!config.matches_extension(".py"));
    }

    #[test]
    fn rust_analyzer_uses_rustup_component_install() {
        let (_, config) = rust_analyzer();
        let Some(install) = config.install else {
            panic!("rust-analyzer should have an install method");
        };
        assert_eq!(
            install.method.install_command("rust-analyzer"),
            vec!["rustup", "component", "add", "rust-analyzer"]
        );
    }

    #[test]
    fn typescript_server_matches_all_js_ts_extensions() {
        let (_, config) = typescript_language_server();
        for ext in &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"] {
            assert!(
                config.matches_extension(ext),
                "typescript server should match {ext}"
            );
        }
        assert!(!config.matches_extension(".py"));
    }
}
