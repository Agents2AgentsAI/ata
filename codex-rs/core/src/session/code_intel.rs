//! LSP and TreeSitter glue code for session initialization, shutdown, and
//! tool-router injection.  Extracted from `codex.rs` to keep the main session
//! orchestration file focused.

#[cfg(feature = "lsp")]
use std::path::Path;
#[cfg(any(feature = "lsp", feature = "treesitter"))]
use std::path::PathBuf;
#[cfg(any(feature = "lsp", feature = "treesitter"))]
use std::sync::Arc;

#[cfg(any(feature = "lsp", feature = "treesitter"))]
use crate::config::Config;
#[cfg(any(feature = "lsp", feature = "treesitter"))]
use crate::state::MultiRootState;
#[cfg(any(feature = "lsp", feature = "treesitter"))]
use crate::state::SessionServices;
#[cfg(any(feature = "lsp", feature = "treesitter"))]
use codex_features::Feature;

// ---------------------------------------------------------------------------
// Config builders
// ---------------------------------------------------------------------------

#[cfg(feature = "lsp")]
fn normalize_lsp_extensions(extensions: Vec<String>) -> Vec<String> {
    extensions
        .into_iter()
        .map(|ext| ext.trim().to_string())
        .filter(|ext| !ext.is_empty())
        .map(|ext| {
            if ext.starts_with('.') {
                ext
            } else {
                format!(".{ext}")
            }
        })
        .collect()
}

#[cfg(feature = "lsp")]
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum LspConfig {
    Enabled(bool),
    Servers(std::collections::HashMap<String, LspServerConfigToml>),
}

#[cfg(feature = "lsp")]
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum LspServerConfigToml {
    DisabledOnly {
        disabled: bool,
    },
    Full {
        command: Vec<String>,
        #[serde(default)]
        extensions: Vec<String>,
        #[serde(default)]
        root_markers: Vec<String>,
        #[serde(default)]
        env: std::collections::HashMap<String, String>,
        #[serde(default)]
        initialization_options: Option<serde_json::Value>,
        #[serde(default)]
        disabled: bool,
    },
}

#[cfg(feature = "lsp")]
pub(super) fn build_lsp_server_configs(
    config: &Config,
) -> std::collections::HashMap<String, codex_lsp_client::LspServerConfig> {
    let builtins = codex_lsp_client::builtin_servers::builtin_servers();
    let mut overrides = std::collections::HashMap::new();

    let lsp_config = config
        .config_layer_stack
        .effective_config()
        .get("lsp")
        .cloned()
        .and_then(|value| match value.try_into::<LspConfig>() {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                tracing::warn!("ignoring invalid `lsp` config: {error}");
                None
            }
        });

    match lsp_config.as_ref() {
        // `lsp = false` (parsed as `Enabled(false)`) disables all LSP integration.
        Some(LspConfig::Enabled(false)) => return std::collections::HashMap::new(),
        // `lsp = true` keeps builtin defaults enabled.
        Some(LspConfig::Enabled(true)) => {}
        Some(LspConfig::Servers(servers)) => {
            for (server_id, server_cfg) in servers {
                let override_cfg = match server_cfg {
                    LspServerConfigToml::DisabledOnly { disabled } => {
                        codex_lsp_client::UserServerOverride::DisabledOnly {
                            disabled: disabled.to_owned(),
                        }
                    }
                    LspServerConfigToml::Full {
                        command,
                        extensions,
                        root_markers,
                        env,
                        initialization_options,
                        disabled,
                    } => codex_lsp_client::UserServerOverride::Full(
                        codex_lsp_client::UserServerFull {
                            command: command.clone(),
                            extensions: normalize_lsp_extensions(extensions.clone()),
                            root_markers: root_markers.clone(),
                            env: env.clone(),
                            initialization_options: initialization_options.clone(),
                            disabled: disabled.to_owned(),
                        },
                    ),
                };
                overrides.insert(server_id.clone(), override_cfg);
            }
        }
        _ => {}
    }

    codex_lsp_client::merge_configs(builtins, overrides)
}

#[cfg(feature = "treesitter")]
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum TreeSitterConfig {
    Enabled(bool),
    Config(TreeSitterConfigMap),
}

#[cfg(feature = "treesitter")]
#[derive(serde::Deserialize)]
struct TreeSitterConfigMap {
    #[serde(default)]
    pub max_file_size: Option<u64>,
    #[serde(default)]
    pub ignore_patterns: Vec<String>,
    #[serde(default)]
    pub ignore_extensions: Vec<String>,
    #[serde(default)]
    pub disabled_languages: Vec<String>,
    #[serde(default)]
    pub annotation_store_path: Option<String>,
    #[serde(default = "default_true")]
    pub persist_annotations: bool,
}

#[cfg(feature = "treesitter")]
fn default_true() -> bool {
    true
}

#[cfg(feature = "treesitter")]
impl Default for TreeSitterConfigMap {
    fn default() -> Self {
        Self {
            max_file_size: None,
            ignore_patterns: Vec::new(),
            ignore_extensions: Vec::new(),
            disabled_languages: Vec::new(),
            annotation_store_path: None,
            persist_annotations: true,
        }
    }
}

#[cfg(feature = "treesitter")]
pub(super) fn build_treesitter_index_config(
    config: &Config,
) -> Option<codex_treesitter::ProjectIndexConfig> {
    let treesitter_config = config
        .config_layer_stack
        .effective_config()
        .get("treesitter")
        .cloned()
        .and_then(|value| match value.try_into::<TreeSitterConfig>() {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                tracing::warn!("ignoring invalid `treesitter` config: {error}");
                None
            }
        });

    let config_map = match treesitter_config {
        // `treesitter = false` disables indexing.
        Some(TreeSitterConfig::Enabled(false)) => return None,
        // `treesitter = true` keeps default TreeSitter settings enabled.
        Some(TreeSitterConfig::Enabled(true)) => TreeSitterConfigMap::default(),
        Some(TreeSitterConfig::Config(map)) => map,
        _ => TreeSitterConfigMap::default(),
    };

    let disabled_languages = config_map
        .disabled_languages
        .iter()
        .filter_map(|language_name| {
            let language = codex_treesitter::Language::from_name(language_name);
            if language.is_none() {
                tracing::warn!(
                    "ignoring unknown treesitter disabled language '{}'",
                    language_name
                );
            }
            language
        })
        .collect::<Vec<_>>();

    let default_config = codex_treesitter::ProjectIndexConfig::default();
    let mut treesitter_config = codex_treesitter::ProjectIndexConfig {
        max_file_size: config_map
            .max_file_size
            .unwrap_or(default_config.max_file_size),
        ignore_patterns: config_map.ignore_patterns.clone(),
        annotation_store_path: config_map.annotation_store_path.clone().map(Into::into),
        persist_annotations: config_map.persist_annotations,
        ..codex_treesitter::ProjectIndexConfig::default()
    }
    .with_disabled_languages(disabled_languages);

    treesitter_config = treesitter_config
        .with_ignore_extensions(config_map.ignore_extensions.iter().map(ToString::to_string));

    Some(treesitter_config)
}

// ---------------------------------------------------------------------------
// Multi-root state initialization
// ---------------------------------------------------------------------------

#[cfg(any(feature = "lsp", feature = "treesitter"))]
pub(super) async fn init_multi_root_state(
    cwd: PathBuf,
    config: &Config,
    #[cfg(feature = "lsp")] install_tracker: Arc<codex_lsp_client::InstallTracker>,
    #[cfg(feature = "lsp")] registry_cache: crate::agent::control::SharedLspRegistryCache,
) -> Option<Arc<MultiRootState>> {
    #[cfg(feature = "lsp")]
    let lsp_server_configs = if config.features.enabled(Feature::Lsp) {
        Some(build_lsp_server_configs(config))
    } else {
        None
    };
    #[cfg(feature = "treesitter")]
    let treesitter_config = if config.features.enabled(Feature::TreeSitter) {
        build_treesitter_index_config(config)
    } else {
        None
    };

    #[cfg(feature = "lsp")]
    let lsp_enabled = lsp_server_configs
        .as_ref()
        .is_some_and(|configs| !configs.is_empty());
    #[cfg(not(feature = "lsp"))]
    let lsp_enabled = false;
    #[cfg(feature = "treesitter")]
    let treesitter_enabled = treesitter_config.is_some();
    #[cfg(not(feature = "treesitter"))]
    let treesitter_enabled = false;

    if lsp_enabled || treesitter_enabled {
        match MultiRootState::new(
            cwd,
            #[cfg(feature = "lsp")]
            lsp_server_configs,
            #[cfg(feature = "lsp")]
            install_tracker,
            #[cfg(feature = "lsp")]
            registry_cache,
            #[cfg(feature = "treesitter")]
            treesitter_config,
        )
        .await
        {
            Ok(state) => Some(state),
            Err(error) => {
                tracing::warn!("failed to initialize multi-root state: {error}");
                None
            }
        }
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// LSP auto-install callback
// ---------------------------------------------------------------------------

#[cfg(feature = "lsp")]
fn lsp_toolchain_paths(codex_home: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let lsp_root = codex_home.join("lsp");
    let bin_dir = lsp_root.join("bin");
    let npm_prefix = lsp_root.join("npm");
    let npm_cache = lsp_root.join("cache").join("npm");
    let pip_prefix = lsp_root.join("pip");
    let gem_home = lsp_root.join("gem");
    (bin_dir, npm_prefix, npm_cache, pip_prefix, gem_home)
}

#[cfg(feature = "lsp")]
async fn ensure_lsp_toolchain_dirs(codex_home: &Path) {
    let (bin_dir, npm_prefix, npm_cache, pip_prefix, gem_home) = lsp_toolchain_paths(codex_home);
    let _ = tokio::fs::create_dir_all(&bin_dir).await;
    let _ = tokio::fs::create_dir_all(&npm_prefix).await;
    let _ = tokio::fs::create_dir_all(&npm_cache).await;
    let _ = tokio::fs::create_dir_all(&pip_prefix).await;
    let _ = tokio::fs::create_dir_all(&gem_home).await;
}

#[cfg(all(unix, feature = "lsp"))]
fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if !arg.bytes().any(|b| {
        matches!(
            b,
            b' ' | b'\t'
                | b'\n'
                | b'\''
                | b'"'
                | b'\\'
                | b'|'
                | b'&'
                | b';'
                | b'<'
                | b'>'
                | b'('
                | b')'
                | b'$'
                | b'`'
                | b'!'
                | b'*'
                | b'?'
                | b'['
                | b']'
                | b'{'
                | b'}'
                | b'#'
        )
    }) {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\"'\"'"))
}

#[cfg(feature = "lsp")]
fn rewrite_lsp_install_command(codex_home: &Path, command: &[String]) -> Vec<String> {
    let (bin_dir, npm_prefix, npm_cache, pip_prefix, gem_home) = lsp_toolchain_paths(codex_home);

    match command {
        [a, b, c, rest @ ..] if a == "npm" && b == "install" && c == "-g" => {
            let mut out = vec![a.clone(), b.clone(), c.clone()];
            out.extend(rest.iter().cloned());
            out.push("--prefix".into());
            out.push(npm_prefix.to_string_lossy().to_string());
            out.push("--cache".into());
            out.push(npm_cache.to_string_lossy().to_string());
            out
        }
        [a, b, c, rest @ ..] if a == "dotnet" && b == "tool" && c == "install" => {
            let mut out = vec![a.clone(), b.clone(), c.clone()];
            out.extend(rest.iter().cloned());
            out.push("--tool-path".into());
            out.push(bin_dir.to_string_lossy().to_string());
            out
        }
        [a, b, rest @ ..] if a == "gem" && b == "install" => {
            #[cfg(unix)]
            {
                let mut cmd = format!(
                    "GEM_HOME={} {} {}",
                    shell_quote(&gem_home.to_string_lossy()),
                    a,
                    b
                );
                for item in rest {
                    cmd.push(' ');
                    cmd.push_str(&shell_quote(item));
                }
                cmd.push_str(" --bindir ");
                cmd.push_str(&shell_quote(&bin_dir.to_string_lossy()));
                vec!["sh".into(), "-lc".into(), cmd]
            }
            #[cfg(windows)]
            {
                let mut cmd = format!("set GEM_HOME={}&& {} {}", gem_home.display(), a, b);
                for item in rest {
                    cmd.push(' ');
                    cmd.push_str(item);
                }
                cmd.push_str(&format!(" --bindir {}", bin_dir.display()));
                vec!["cmd".into(), "/C".into(), cmd]
            }
        }
        [a, b, rest @ ..] if a == "go" && b == "install" => {
            #[cfg(unix)]
            {
                let mut cmd = format!("GOBIN={} {}", shell_quote(&bin_dir.to_string_lossy()), a);
                cmd.push(' ');
                cmd.push_str(b);
                for item in rest {
                    cmd.push(' ');
                    cmd.push_str(&shell_quote(item));
                }
                vec!["sh".into(), "-lc".into(), cmd]
            }
            #[cfg(windows)]
            {
                let mut cmd = format!("set GOBIN={}&& {} {}", bin_dir.display(), a, b);
                for item in rest {
                    cmd.push(' ');
                    cmd.push_str(item);
                }
                vec!["cmd".into(), "/C".into(), cmd]
            }
        }
        [a, b, rest @ ..] if a == "pip" && b == "install" => {
            let binary = if which::which("pip").is_err() && which::which("pip3").is_ok() {
                "pip3".to_string()
            } else {
                a.clone()
            };
            let mut out = vec![binary, b.clone()];
            out.extend(rest.iter().cloned());
            out.push("--prefix".into());
            out.push(pip_prefix.to_string_lossy().to_string());
            out
        }
        _ => command.to_vec(),
    }
}

#[cfg(feature = "lsp")]
fn install_prefix_rule(command: &[String]) -> Option<Vec<String>> {
    match command {
        [a, b, c, ..] if a == "npm" && b == "install" && c == "-g" => {
            Some(vec![a.clone(), b.clone(), c.clone()])
        }
        [a, b, c, ..] if a == "dotnet" && b == "tool" && c == "install" => {
            Some(vec![a.clone(), b.clone(), c.clone()])
        }
        [a, b, ..] => Some(vec![a.clone(), b.clone()]),
        [a] => Some(vec![a.clone()]),
        [] => None,
    }
}

#[cfg(feature = "lsp")]
pub(super) async fn setup_lsp_install_callback(sess: &Arc<super::Session>) {
    use uuid::Uuid;

    if let Some(ref multi_root_state) = sess.services.multi_root_state
        && multi_root_state.has_lsp()
    {
        let weak_sess = Arc::downgrade(sess);
        multi_root_state
            .set_install_confirm(Some(Arc::new(
                move |prompt: &str, command: &[String]| {
                    let weak_sess = weak_sess.clone();
                    let prompt = prompt.to_string();
                    let command = command.to_vec();

                    Box::pin(async move {
                        if *crate::flags::CODEX_DISABLE_LSP_DOWNLOAD {
                            tracing::info!(
                                "skipping LSP auto-install because CODEX_DISABLE_LSP_DOWNLOAD is set"
                            );
                            return false;
                        }
                        let Some(sess) = weak_sess.upgrade() else {
                            return false;
                        };
                        let Some((turn_context, cancellation_token)) =
                            sess.active_turn_context_and_cancellation_token().await
                        else {
                            tracing::debug!(
                                "skipping LSP auto-install prompt: no active turn context"
                            );
                            return false;
                        };

                        // Run the install through unified_exec so it can request
                        // escalated sandbox permissions and reuse the standard
                        // command approval UX (single prompt).
                        let process_id = sess
                            .services
                            .unified_exec_manager
                            .allocate_process_id()
                            .await;

                        ensure_lsp_toolchain_dirs(turn_context.config.codex_home.as_path()).await;
                        let original_command = command.clone();
                        let command = rewrite_lsp_install_command(
                            turn_context.config.codex_home.as_path(),
                            &original_command,
                        );

                        // Offer a stable "approve once" choice for install commands.
                        // Keep this conservative to avoid over-broad allowlisting.
                        let prefix_rule: Option<Vec<String>> =
                            install_prefix_rule(original_command.as_slice());

                        let justification =
                            Some(format!("{prompt} Command: `{}`", command.join(" ")));

                        let context = crate::unified_exec::UnifiedExecContext::new(
                            Arc::clone(&sess),
                            Arc::clone(&turn_context),
                            format!("lsp-install-{}", Uuid::new_v4()),
                        );
                        let Some(turn_environment) = turn_context.environments.primary() else {
                            sess.services
                                .unified_exec_manager
                                .release_process_id(process_id)
                                .await;
                            return false;
                        };
                        let hook_command = codex_shell_command::parse_command::shlex_join(&command);

                        let mut response = tokio::select! {
                            _ = cancellation_token.cancelled() => {
                                sess.services.unified_exec_manager.release_process_id(process_id).await;
                                return false;
                            }
                            result = sess.services.unified_exec_manager.exec_command(
                                crate::unified_exec::ExecCommandRequest {
                                    command: command.clone(),
                                    shell_type: sess.services.user_shell.shell_type.clone(),
                                    hook_command,
                                    process_id,
                                    yield_time_ms: 10_000,
                                    max_output_tokens: None,
                                    cwd: turn_environment.cwd.clone(),
                                    sandbox_cwd: turn_environment.cwd.clone(),
                                    environment: Arc::clone(&turn_environment.environment),
                                    network: turn_context.network.clone(),
                                    tty: false,
                                    sandbox_permissions: crate::sandboxing::SandboxPermissions::RequireEscalated,
                                    additional_permissions: None,
                                    additional_permissions_preapproved: false,
                                    justification,
                                    prefix_rule,
                                },
                                &context,
                            ) => match result {
                                Ok(resp) => resp,
                                Err(err) => {
                                    tracing::warn!("LSP auto-install failed to start: {err:?}");
                                    return false;
                                }
                            }
                        };

                        // If the install is long-running, poll for completion.
                        let deadline = tokio::time::Instant::now()
                            + std::time::Duration::from_secs(10 * 60);
                        while response.exit_code.is_none() && response.process_id.is_some() {
                            if tokio::time::Instant::now() >= deadline {
                                tracing::warn!("LSP auto-install timed out");
                                break;
                            }
                            let poll = tokio::select! {
                                _ = cancellation_token.cancelled() => break,
                                poll = sess.services.unified_exec_manager.write_stdin(
                                    crate::unified_exec::WriteStdinRequest {
                                        process_id,
                                        input: "",
                                        yield_time_ms: 10_000,
                                        max_output_tokens: None,
                                        truncation_policy: turn_context.truncation_policy,
                                    }
                                ) => poll
                            };
                            match poll {
                                Ok(r) => response = r,
                                Err(err) => {
                                    tracing::warn!("LSP auto-install polling failed: {err:?}");
                                    break;
                                }
                            }
                        }

                        // If the process exited during polling, the unified exec manager
                        // will have removed it from the store; release_process_id is a no-op.
                        sess.services.unified_exec_manager.release_process_id(process_id).await;

                        response.exit_code == Some(0)
                    })
                },
            )))
            .await;
    }
}

// ---------------------------------------------------------------------------
// Shutdown helper
// ---------------------------------------------------------------------------

#[cfg(any(feature = "lsp", feature = "treesitter"))]
pub(super) async fn shutdown_code_intel(services: &SessionServices) {
    if let Some(ref multi_root_state) = services.multi_root_state {
        multi_root_state.shutdown_all().await;
    }
}

// ---------------------------------------------------------------------------
// Tool-router injection
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "lsp"))]
mod tests {
    use super::*;

    fn codex_home() -> PathBuf {
        PathBuf::from("/tmp/codex-home-test")
    }

    #[test]
    fn rewrites_npm_global_to_managed_prefix() {
        let home = codex_home();
        let expected_prefix = home.join("lsp").join("npm");
        let expected_cache = home.join("lsp").join("cache").join("npm");
        let cmd = vec![
            "npm".to_string(),
            "install".to_string(),
            "-g".to_string(),
            "pyright".to_string(),
        ];
        let rewritten = rewrite_lsp_install_command(home.as_path(), &cmd);
        assert!(
            rewritten
                .windows(2)
                .any(|w| w[0] == "--prefix" && w[1] == expected_prefix.to_string_lossy().as_ref())
        );
        assert!(
            rewritten
                .windows(2)
                .any(|w| w[0] == "--cache" && w[1] == expected_cache.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn rewrites_dotnet_tool_install_to_tool_path() {
        let home = codex_home();
        let expected_bin = home.join("lsp").join("bin");
        let cmd = vec![
            "dotnet".to_string(),
            "tool".to_string(),
            "install".to_string(),
            "csharp-ls".to_string(),
        ];
        let rewritten = rewrite_lsp_install_command(home.as_path(), &cmd);
        assert!(
            rewritten
                .windows(2)
                .any(|w| w[0] == "--tool-path" && w[1] == expected_bin.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn rewrites_gem_install_to_bindir() {
        let home = codex_home();
        let expected_bin = home.join("lsp").join("bin");
        let expected_gem_home = home.join("lsp").join("gem");
        let cmd = vec![
            "gem".to_string(),
            "install".to_string(),
            "rubocop".to_string(),
        ];
        let rewritten = rewrite_lsp_install_command(home.as_path(), &cmd);
        #[cfg(unix)]
        {
            assert_eq!(rewritten[0], "sh");
            assert_eq!(rewritten[1], "-lc");
            assert!(rewritten[2].contains("GEM_HOME="));
            assert!(rewritten[2].contains(expected_gem_home.to_string_lossy().as_ref()));
            assert!(rewritten[2].contains(" --bindir "));
            assert!(rewritten[2].contains(expected_bin.to_string_lossy().as_ref()));
        }
        #[cfg(windows)]
        {
            assert_eq!(rewritten[0], "cmd");
            assert_eq!(rewritten[1], "/C");
            assert!(rewritten[2].contains("GEM_HOME="));
            assert!(rewritten[2].contains(expected_gem_home.to_string_lossy().as_ref()));
            assert!(rewritten[2].contains(" --bindir "));
            assert!(rewritten[2].contains(expected_bin.to_string_lossy().as_ref()));
        }
    }

    #[test]
    fn lsp_toolchain_paths_include_gem_home() {
        let home = codex_home();
        let (_bin_dir, _npm_prefix, _npm_cache, _pip_prefix, gem_home) =
            lsp_toolchain_paths(home.as_path());
        assert_eq!(gem_home, home.join("lsp").join("gem"));
    }

    #[test]
    fn rewrites_pip_install_to_managed_prefix() {
        let home = codex_home();
        let expected_prefix = home.join("lsp").join("pip");
        let cmd = vec![
            "pip".to_string(),
            "install".to_string(),
            "python-lsp-server".to_string(),
        ];
        let rewritten = rewrite_lsp_install_command(home.as_path(), &cmd);
        assert!(
            rewritten
                .windows(2)
                .any(|w| w[0] == "--prefix" && w[1] == expected_prefix.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn install_prefix_rule_prefers_stable_install_prefixes() {
        let npm = vec![
            "npm".to_string(),
            "install".to_string(),
            "-g".to_string(),
            "pyright".to_string(),
        ];
        let rule = install_prefix_rule(&npm).expect("rule");
        assert_eq!(rule, vec!["npm", "install", "-g"]);

        let dotnet = vec![
            "dotnet".to_string(),
            "tool".to_string(),
            "install".to_string(),
            "csharp-ls".to_string(),
        ];
        let rule = install_prefix_rule(&dotnet).expect("rule");
        assert_eq!(rule, vec!["dotnet", "tool", "install"]);
    }
}
