//! Built-in LSP server configurations for common languages.

use crate::server_config::InstallConfig;
use crate::server_config::InstallMethod;
use crate::server_config::LspServerConfig;

/// Returns built-in server configurations.
pub fn builtin_servers() -> Vec<(&'static str, LspServerConfig)> {
    vec![
        rust_analyzer(),
        typescript_language_server(),
        gopls(),
        pyright(),
        clangd(),
        sourcekit_lsp(),
        yaml_language_server(),
        bash_language_server(),
        dockerfile_language_server(),
        vue_language_server(),
        svelte_language_server(),
        astro_language_server(),
        intelephense(),
        csharp_ls(),
        fsharp_ls(),
        rubocop(),
        terraform_ls(),
        lua_language_server(),
        texlab(),
        zls(),
        jdtls(),
        kotlin_lsp(),
        clojure_lsp(),
        nixd(),
        tinymist(),
    ]
}

fn rust_analyzer() -> (&'static str, LspServerConfig) {
    (
        "rust-analyzer",
        LspServerConfig {
            extensions: vec![".rs".into()],
            command: vec!["rust-analyzer".into()],
            command_candidates: Vec::new(),
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
            command_candidates: Vec::new(),
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
            command_candidates: Vec::new(),
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
            command_candidates: Vec::new(),
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
                method: InstallMethod::Npm {
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
            command_candidates: Vec::new(),
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
            command_candidates: vec![vec!["xcrun".into(), "sourcekit-lsp".into()]],
            env: Default::default(),
            root_markers: vec![
                "Package.swift".into(),
                "*.xcodeproj".into(),
                "*.xcworkspace".into(),
            ],
            initialization_options: None,
            disabled: false,
            // sourcekit-lsp ships with Xcode — no separate install.
            install: None,
        },
    )
}

fn yaml_language_server() -> (&'static str, LspServerConfig) {
    (
        "yaml-language-server",
        LspServerConfig {
            extensions: vec![".yaml".into(), ".yml".into()],
            command: vec!["yaml-language-server".into(), "--stdio".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![
                ".yamllint".into(),
                "docker-compose.yml".into(),
                "kustomization.yaml".into(),
                ".git".into(),
            ],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::Npm {
                    package: Some("yaml-language-server".into()),
                },
            }),
        },
    )
}

fn bash_language_server() -> (&'static str, LspServerConfig) {
    (
        "bash-language-server",
        LspServerConfig {
            extensions: vec![".sh".into(), ".bash".into(), ".zsh".into(), ".ksh".into()],
            command: vec!["bash-language-server".into(), "start".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![".git".into(), "package.json".into()],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::Npm {
                    package: Some("bash-language-server".into()),
                },
            }),
        },
    )
}

fn dockerfile_language_server() -> (&'static str, LspServerConfig) {
    (
        "dockerfile-language-server-nodejs",
        LspServerConfig {
            extensions: vec![".dockerfile".into()],
            command: vec!["docker-langserver".into(), "--stdio".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![
                "Dockerfile".into(),
                "docker-compose.yml".into(),
                ".git".into(),
            ],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::Npm {
                    package: Some("dockerfile-language-server-nodejs".into()),
                },
            }),
        },
    )
}

fn vue_language_server() -> (&'static str, LspServerConfig) {
    (
        "vue-language-server",
        LspServerConfig {
            extensions: vec![".vue".into()],
            command: vec!["vue-language-server".into(), "--stdio".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![
                "package.json".into(),
                "pnpm-lock.yaml".into(),
                "yarn.lock".into(),
            ],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::Npm {
                    package: Some("@vue/language-server".into()),
                },
            }),
        },
    )
}

fn svelte_language_server() -> (&'static str, LspServerConfig) {
    (
        "svelte-language-server",
        LspServerConfig {
            extensions: vec![".svelte".into()],
            command: vec!["svelteserver".into(), "--stdio".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![
                "package.json".into(),
                "svelte.config.js".into(),
                "svelte.config.ts".into(),
            ],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::Npm {
                    package: Some("svelte-language-server".into()),
                },
            }),
        },
    )
}

fn astro_language_server() -> (&'static str, LspServerConfig) {
    (
        "astro-language-server",
        LspServerConfig {
            extensions: vec![".astro".into()],
            command: vec!["astro-ls".into(), "--stdio".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![
                "astro.config.mjs".into(),
                "astro.config.ts".into(),
                "package.json".into(),
            ],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::Npm {
                    package: Some("@astrojs/language-server".into()),
                },
            }),
        },
    )
}

fn intelephense() -> (&'static str, LspServerConfig) {
    (
        "intelephense",
        LspServerConfig {
            extensions: vec![".php".into()],
            command: vec!["intelephense".into(), "--stdio".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![
                "composer.json".into(),
                "composer.lock".into(),
                ".git".into(),
            ],
            initialization_options: Some(serde_json::json!({
                "telemetry": { "enabled": false }
            })),
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::Npm {
                    package: Some("intelephense".into()),
                },
            }),
        },
    )
}

fn csharp_ls() -> (&'static str, LspServerConfig) {
    (
        "csharp-ls",
        LspServerConfig {
            extensions: vec![".cs".into()],
            command: vec!["csharp-ls".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![".sln".into(), ".csproj".into(), "global.json".into()],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::DotnetTool {
                    package: Some("csharp-ls".into()),
                },
            }),
        },
    )
}

fn fsharp_ls() -> (&'static str, LspServerConfig) {
    (
        "fsautocomplete",
        LspServerConfig {
            extensions: vec![
                ".fs".into(),
                ".fsi".into(),
                ".fsx".into(),
                ".fsscript".into(),
            ],
            command: vec!["fsautocomplete".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![".sln".into(), ".fsproj".into(), "global.json".into()],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::DotnetTool {
                    package: Some("fsautocomplete".into()),
                },
            }),
        },
    )
}

fn rubocop() -> (&'static str, LspServerConfig) {
    (
        "rubocop",
        LspServerConfig {
            extensions: vec![
                ".rb".into(),
                ".rake".into(),
                ".gemspec".into(),
                ".ru".into(),
            ],
            command: vec!["rubocop".into(), "--lsp".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec!["Gemfile".into(), ".ruby-version".into(), ".git".into()],
            initialization_options: None,
            disabled: false,
            install: Some(InstallConfig {
                method: InstallMethod::Gem {
                    package: Some("rubocop".into()),
                },
            }),
        },
    )
}

fn terraform_ls() -> (&'static str, LspServerConfig) {
    (
        "terraform-ls",
        LspServerConfig {
            extensions: vec![".tf".into(), ".tfvars".into()],
            command: vec!["terraform-ls".into(), "serve".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![".terraform.lock.hcl".into(), "*.tf".into(), ".git".into()],
            initialization_options: Some(serde_json::json!({
                "experimentalFeatures": {
                    "prefillRequiredFields": true,
                    "validateOnSave": true
                }
            })),
            disabled: false,
            install: None,
        },
    )
}

fn lua_language_server() -> (&'static str, LspServerConfig) {
    (
        "lua-language-server",
        LspServerConfig {
            extensions: vec![".lua".into()],
            command: vec!["lua-language-server".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![
                ".luarc.json".into(),
                ".luarc.jsonc".into(),
                "stylua.toml".into(),
                ".git".into(),
            ],
            initialization_options: None,
            disabled: false,
            install: None,
        },
    )
}

fn texlab() -> (&'static str, LspServerConfig) {
    (
        "texlab",
        LspServerConfig {
            extensions: vec![".tex".into(), ".bib".into()],
            command: vec!["texlab".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![".latexmkrc".into(), "latexmkrc".into(), ".git".into()],
            initialization_options: None,
            disabled: false,
            install: None,
        },
    )
}

fn zls() -> (&'static str, LspServerConfig) {
    (
        "zls",
        LspServerConfig {
            extensions: vec![".zig".into(), ".zon".into()],
            command: vec!["zls".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec!["build.zig".into(), ".git".into()],
            initialization_options: None,
            disabled: false,
            install: None,
        },
    )
}

fn jdtls() -> (&'static str, LspServerConfig) {
    (
        "jdtls",
        LspServerConfig {
            extensions: vec![".java".into()],
            command: vec!["jdtls".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![
                "pom.xml".into(),
                "build.gradle".into(),
                "build.gradle.kts".into(),
                ".project".into(),
            ],
            initialization_options: None,
            disabled: false,
            install: None,
        },
    )
}

fn kotlin_lsp() -> (&'static str, LspServerConfig) {
    (
        "kotlin-lsp",
        LspServerConfig {
            extensions: vec![".kt".into(), ".kts".into()],
            command: vec!["kotlin-lsp".into(), "--stdio".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![
                "settings.gradle.kts".into(),
                "settings.gradle".into(),
                "build.gradle.kts".into(),
                "build.gradle".into(),
            ],
            initialization_options: None,
            disabled: false,
            install: None,
        },
    )
}

fn clojure_lsp() -> (&'static str, LspServerConfig) {
    (
        "clojure-lsp",
        LspServerConfig {
            extensions: vec![".clj".into(), ".cljs".into(), ".cljc".into(), ".edn".into()],
            command: vec!["clojure-lsp".into(), "listen".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec![
                "deps.edn".into(),
                "project.clj".into(),
                "shadow-cljs.edn".into(),
                ".git".into(),
            ],
            initialization_options: None,
            disabled: false,
            install: None,
        },
    )
}

fn nixd() -> (&'static str, LspServerConfig) {
    (
        "nixd",
        LspServerConfig {
            extensions: vec![".nix".into()],
            command: vec!["nixd".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec!["flake.nix".into(), "shell.nix".into(), ".git".into()],
            initialization_options: None,
            disabled: false,
            install: None,
        },
    )
}

fn tinymist() -> (&'static str, LspServerConfig) {
    (
        "tinymist",
        LspServerConfig {
            extensions: vec![".typ".into(), ".typc".into()],
            command: vec!["tinymist".into()],
            command_candidates: Vec::new(),
            env: Default::default(),
            root_markers: vec!["typst.toml".into(), ".git".into()],
            initialization_options: None,
            disabled: false,
            install: None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_servers_returns_many() {
        let servers = builtin_servers();
        assert!(servers.len() >= 20, "expected broad built-in coverage");
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

    #[test]
    fn pyright_uses_npm_install() {
        let (_, config) = pyright();
        let Some(install) = config.install else {
            panic!("pyright should have an install method");
        };
        assert_eq!(
            install.method.install_command("pyright-langserver"),
            vec!["npm", "install", "-g", "pyright"]
        );
    }

    #[test]
    fn sourcekit_has_xcrun_fallback() {
        let (_, config) = sourcekit_lsp();
        assert_eq!(
            config.command_candidates,
            vec![vec!["xcrun".to_string(), "sourcekit-lsp".to_string()]]
        );
    }
}
