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
use crate::features::Feature;
#[cfg(any(feature = "lsp", feature = "treesitter"))]
use crate::state::MultiRootState;
#[cfg(any(feature = "lsp", feature = "treesitter"))]
use crate::state::SessionServices;
#[cfg(any(feature = "lsp", feature = "treesitter"))]
use crate::tools::spec::ToolsConfig;

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
pub(super) fn build_lsp_server_configs(
    config: &Config,
) -> std::collections::HashMap<String, codex_lsp_client::LspServerConfig> {
    use crate::config::types::LspConfig;
    use crate::config::types::LspServerConfigToml;

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
pub(super) fn build_treesitter_index_config(
    config: &Config,
) -> Option<codex_treesitter::ProjectIndexConfig> {
    use crate::config::types::TreeSitterConfig;
    use crate::config::types::TreeSitterConfigMap;

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

    let mut treesitter_config = codex_treesitter::ProjectIndexConfig {
        max_file_size: config_map.max_file_size,
        ignore_patterns: config_map.ignore_patterns.clone(),
        annotation_store_path: config_map.annotation_store_path.clone().map(Into::into),
        watch: config_map.watch,
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
            #[cfg(feature = "treesitter")]
            treesitter_config,
        )
        .await
        {
            Ok(state) => Some(Arc::new(state)),
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
fn lsp_toolchain_paths(codex_home: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let lsp_root = codex_home.join("lsp");
    let bin_dir = lsp_root.join("bin");
    let npm_prefix = lsp_root.join("npm");
    let npm_cache = lsp_root.join("cache").join("npm");
    let pip_prefix = lsp_root.join("pip");
    (bin_dir, npm_prefix, npm_cache, pip_prefix)
}

#[cfg(feature = "lsp")]
async fn ensure_lsp_toolchain_dirs(codex_home: &Path) {
    let (bin_dir, npm_prefix, npm_cache, pip_prefix) = lsp_toolchain_paths(codex_home);
    let _ = tokio::fs::create_dir_all(&bin_dir).await;
    let _ = tokio::fs::create_dir_all(&npm_prefix).await;
    let _ = tokio::fs::create_dir_all(&npm_cache).await;
    let _ = tokio::fs::create_dir_all(&pip_prefix).await;
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
    let (bin_dir, npm_prefix, npm_cache, pip_prefix) = lsp_toolchain_paths(codex_home);

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
            let mut out = vec![a.clone(), b.clone()];
            out.extend(rest.iter().cloned());
            out.push("--bindir".into());
            out.push(bin_dir.to_string_lossy().to_string());
            out
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

                        let mut response = tokio::select! {
                            _ = cancellation_token.cancelled() => {
                                sess.services.unified_exec_manager.release_process_id(&process_id).await;
                                return false;
                            }
                            result = sess.services.unified_exec_manager.exec_command(
                                crate::unified_exec::ExecCommandRequest {
                                    command: command.clone(),
                                    process_id: process_id.clone(),
                                    yield_time_ms: 10_000,
                                    max_output_tokens: None,
                                    workdir: None,
                                    network: None,
                                    tty: false,
                                    sandbox_permissions: crate::sandboxing::SandboxPermissions::RequireEscalated,
                                    additional_permissions: None,
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
                                        process_id: process_id.as_str(),
                                        input: "",
                                        yield_time_ms: 10_000,
                                        max_output_tokens: None,
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
                        sess.services.unified_exec_manager.release_process_id(&process_id).await;

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

#[cfg(any(feature = "lsp", feature = "treesitter"))]
pub(super) fn inject_multi_root_state(tools_config: &mut ToolsConfig, services: &SessionServices) {
    if let Some(ref multi_root_state) = services.multi_root_state {
        tools_config.multi_root_state = Some(Arc::clone(multi_root_state));
    }
}

#[cfg(all(test, feature = "lsp"))]
mod tests {
    use super::*;

    fn codex_home() -> PathBuf {
        PathBuf::from("/tmp/codex-home-test")
    }

    #[test]
    fn rewrites_npm_global_to_managed_prefix() {
        let cmd = vec![
            "npm".to_string(),
            "install".to_string(),
            "-g".to_string(),
            "pyright".to_string(),
        ];
        let rewritten = rewrite_lsp_install_command(codex_home().as_path(), &cmd);
        assert!(
            rewritten
                .windows(2)
                .any(|w| w == ["--prefix", "/tmp/codex-home-test/lsp/npm"])
        );
        assert!(
            rewritten
                .windows(2)
                .any(|w| w == ["--cache", "/tmp/codex-home-test/lsp/cache/npm"])
        );
    }

    #[test]
    fn rewrites_dotnet_tool_install_to_tool_path() {
        let cmd = vec![
            "dotnet".to_string(),
            "tool".to_string(),
            "install".to_string(),
            "csharp-ls".to_string(),
        ];
        let rewritten = rewrite_lsp_install_command(codex_home().as_path(), &cmd);
        assert!(
            rewritten
                .windows(2)
                .any(|w| w == ["--tool-path", "/tmp/codex-home-test/lsp/bin"])
        );
    }

    #[test]
    fn rewrites_gem_install_to_bindir() {
        let cmd = vec![
            "gem".to_string(),
            "install".to_string(),
            "rubocop".to_string(),
        ];
        let rewritten = rewrite_lsp_install_command(codex_home().as_path(), &cmd);
        assert!(
            rewritten
                .windows(2)
                .any(|w| w == ["--bindir", "/tmp/codex-home-test/lsp/bin"])
        );
    }

    #[test]
    fn rewrites_pip_install_to_managed_prefix() {
        let cmd = vec![
            "pip".to_string(),
            "install".to_string(),
            "python-lsp-server".to_string(),
        ];
        let rewritten = rewrite_lsp_install_command(codex_home().as_path(), &cmd);
        assert!(
            rewritten
                .windows(2)
                .any(|w| w == ["--prefix", "/tmp/codex-home-test/lsp/pip"])
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
