//! Built-in LSP server configurations for common languages.

use crate::server_config::{InstallConfig, InstallMethod, LspServerConfig};

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
                method: InstallMethod::Cargo {
                    package: Some("rust-analyzer".into()),
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
