//! Built-in LSP server configurations for common languages.

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
        cfg(&[".rs"], &["rust-analyzer"], &["Cargo.toml"]).with_install(
            InstallMethod::RustupComponent {
                component: Some("rust-analyzer".into()),
            },
        ),
    )
}

fn typescript_language_server() -> (&'static str, LspServerConfig) {
    (
        "typescript-language-server",
        cfg(
            &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"],
            &["typescript-language-server", "--stdio"],
            &["tsconfig.json", "jsconfig.json", "package.json"],
        )
        .with_install(InstallMethod::Npm {
            package: Some("typescript-language-server".into()),
        }),
    )
}

fn gopls() -> (&'static str, LspServerConfig) {
    (
        "gopls",
        cfg(&[".go"], &["gopls"], &["go.mod", "go.sum"]).with_install(InstallMethod::Go {
            package_path: "golang.org/x/tools/gopls".into(),
        }),
    )
}

fn pyright() -> (&'static str, LspServerConfig) {
    (
        "pyright",
        cfg(
            &[".py", ".pyi"],
            &["pyright-langserver", "--stdio"],
            &[
                "pyproject.toml",
                "setup.py",
                "setup.cfg",
                "requirements.txt",
                "pyrightconfig.json",
            ],
        )
        .with_install(InstallMethod::Npm {
            package: Some("pyright".into()),
        }),
    )
}

fn clangd() -> (&'static str, LspServerConfig) {
    (
        "clangd",
        cfg(
            &[".c", ".cpp", ".cc", ".cxx", ".h", ".hpp"],
            &["clangd"],
            &["compile_commands.json", ".clangd", "CMakeLists.txt"],
        )
        .with_install(InstallMethod::Brew {
            formula: Some("llvm".into()),
        }),
    )
}

fn sourcekit_lsp() -> (&'static str, LspServerConfig) {
    (
        "sourcekit-lsp",
        // sourcekit-lsp ships with Xcode — no separate install.
        cfg(
            &[".swift"],
            &["sourcekit-lsp"],
            &["Package.swift", "*.xcodeproj", "*.xcworkspace"],
        )
        .with_command_candidates(vec![v(&["xcrun", "sourcekit-lsp"])]),
    )
}

fn yaml_language_server() -> (&'static str, LspServerConfig) {
    (
        "yaml-language-server",
        cfg(
            &[".yaml", ".yml"],
            &["yaml-language-server", "--stdio"],
            &[
                ".yamllint",
                "docker-compose.yml",
                "kustomization.yaml",
                ".git",
            ],
        )
        .with_install(InstallMethod::Npm {
            package: Some("yaml-language-server".into()),
        }),
    )
}

fn bash_language_server() -> (&'static str, LspServerConfig) {
    (
        "bash-language-server",
        cfg(
            &[".sh", ".bash", ".zsh", ".ksh"],
            &["bash-language-server", "start"],
            &[".git", "package.json"],
        )
        .with_install(InstallMethod::Npm {
            package: Some("bash-language-server".into()),
        }),
    )
}

fn dockerfile_language_server() -> (&'static str, LspServerConfig) {
    (
        "dockerfile-language-server-nodejs",
        cfg(
            &[".dockerfile"],
            &["docker-langserver", "--stdio"],
            &["Dockerfile", "docker-compose.yml", ".git"],
        )
        .with_install(InstallMethod::Npm {
            package: Some("dockerfile-language-server-nodejs".into()),
        }),
    )
}

fn vue_language_server() -> (&'static str, LspServerConfig) {
    (
        "vue-language-server",
        cfg(
            &[".vue"],
            &["vue-language-server", "--stdio"],
            &["package.json", "pnpm-lock.yaml", "yarn.lock"],
        )
        .with_install(InstallMethod::Npm {
            package: Some("@vue/language-server".into()),
        }),
    )
}

fn svelte_language_server() -> (&'static str, LspServerConfig) {
    (
        "svelte-language-server",
        cfg(
            &[".svelte"],
            &["svelteserver", "--stdio"],
            &["package.json", "svelte.config.js", "svelte.config.ts"],
        )
        .with_install(InstallMethod::Npm {
            package: Some("svelte-language-server".into()),
        }),
    )
}

fn astro_language_server() -> (&'static str, LspServerConfig) {
    (
        "astro-language-server",
        cfg(
            &[".astro"],
            &["astro-ls", "--stdio"],
            &["astro.config.mjs", "astro.config.ts", "package.json"],
        )
        .with_install(InstallMethod::Npm {
            package: Some("@astrojs/language-server".into()),
        }),
    )
}

fn intelephense() -> (&'static str, LspServerConfig) {
    (
        "intelephense",
        cfg(
            &[".php"],
            &["intelephense", "--stdio"],
            &["composer.json", "composer.lock", ".git"],
        )
        .with_initialization_options(serde_json::json!({
            "telemetry": { "enabled": false }
        }))
        .with_install(InstallMethod::Npm {
            package: Some("intelephense".into()),
        }),
    )
}

fn csharp_ls() -> (&'static str, LspServerConfig) {
    (
        "csharp-ls",
        cfg(
            &[".cs"],
            &["csharp-ls"],
            &[".sln", ".csproj", "global.json"],
        )
        .with_install(InstallMethod::DotnetTool {
            package: Some("csharp-ls".into()),
        }),
    )
}

fn fsharp_ls() -> (&'static str, LspServerConfig) {
    (
        "fsautocomplete",
        cfg(
            &[".fs", ".fsi", ".fsx", ".fsscript"],
            &["fsautocomplete"],
            &[".sln", ".fsproj", "global.json"],
        )
        .with_install(InstallMethod::DotnetTool {
            package: Some("fsautocomplete".into()),
        }),
    )
}

fn rubocop() -> (&'static str, LspServerConfig) {
    (
        "rubocop",
        cfg(
            &[".rb", ".rake", ".gemspec", ".ru"],
            &["rubocop", "--lsp"],
            &["Gemfile", ".ruby-version", ".git"],
        )
        .with_install(InstallMethod::Gem {
            package: Some("rubocop".into()),
        }),
    )
}

fn terraform_ls() -> (&'static str, LspServerConfig) {
    (
        "terraform-ls",
        cfg(
            &[".tf", ".tfvars"],
            &["terraform-ls", "serve"],
            &[".terraform.lock.hcl", "*.tf", ".git"],
        )
        .with_initialization_options(serde_json::json!({
            "experimentalFeatures": {
                "prefillRequiredFields": true,
                "validateOnSave": true
            }
        })),
    )
}

fn lua_language_server() -> (&'static str, LspServerConfig) {
    (
        "lua-language-server",
        cfg(
            &[".lua"],
            &["lua-language-server"],
            &[".luarc.json", ".luarc.jsonc", "stylua.toml", ".git"],
        ),
    )
}

fn texlab() -> (&'static str, LspServerConfig) {
    (
        "texlab",
        cfg(
            &[".tex", ".bib"],
            &["texlab"],
            &[".latexmkrc", "latexmkrc", ".git"],
        ),
    )
}

fn zls() -> (&'static str, LspServerConfig) {
    (
        "zls",
        cfg(&[".zig", ".zon"], &["zls"], &["build.zig", ".git"]),
    )
}

fn jdtls() -> (&'static str, LspServerConfig) {
    (
        "jdtls",
        cfg(
            &[".java"],
            &["jdtls"],
            &["pom.xml", "build.gradle", "build.gradle.kts", ".project"],
        ),
    )
}

fn kotlin_lsp() -> (&'static str, LspServerConfig) {
    (
        "kotlin-lsp",
        cfg(
            &[".kt", ".kts"],
            &["kotlin-lsp", "--stdio"],
            &[
                "settings.gradle.kts",
                "settings.gradle",
                "build.gradle.kts",
                "build.gradle",
            ],
        ),
    )
}

fn clojure_lsp() -> (&'static str, LspServerConfig) {
    (
        "clojure-lsp",
        cfg(
            &[".clj", ".cljs", ".cljc", ".edn"],
            &["clojure-lsp", "listen"],
            &["deps.edn", "project.clj", "shadow-cljs.edn", ".git"],
        ),
    )
}

fn nixd() -> (&'static str, LspServerConfig) {
    (
        "nixd",
        cfg(&[".nix"], &["nixd"], &["flake.nix", "shell.nix", ".git"]),
    )
}

fn tinymist() -> (&'static str, LspServerConfig) {
    (
        "tinymist",
        cfg(&[".typ", ".typc"], &["tinymist"], &["typst.toml", ".git"]),
    )
}

fn cfg(extensions: &[&str], command: &[&str], root_markers: &[&str]) -> LspServerConfig {
    LspServerConfig::new(v(extensions), v(command), v(root_markers))
}

fn v(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
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
