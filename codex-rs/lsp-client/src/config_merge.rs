//! Merge built-in server configs with user overrides from TOML.

use std::collections::HashMap;

use crate::server_config::LspServerConfig;
use crate::server_config::PostRootHook;
use crate::server_config::RootStrategy;

/// Merge built-in server configurations with user overrides.
///
/// Rules:
/// - If a user config has the same server_id as a built-in, it overrides it.
/// - If the user only specifies `command` (no extensions/root_markers), the
///   built-in values for extensions and root_markers are inherited.
/// - If the user sets `disabled = true`, the server is disabled.
/// - User configs with unknown server_ids are added as custom servers
///   (they must specify extensions).
pub fn merge_configs(
    builtins: Vec<(&str, LspServerConfig)>,
    user_overrides: HashMap<String, UserServerOverride>,
) -> HashMap<String, LspServerConfig> {
    let mut result: HashMap<String, LspServerConfig> = HashMap::new();

    // Start with builtins.
    for (id, config) in builtins {
        result.insert(id.to_string(), config);
    }

    // Apply user overrides.
    for (id, user) in user_overrides {
        match user {
            UserServerOverride::DisabledOnly { disabled } => {
                if let Some(existing) = result.get_mut(&id) {
                    existing.disabled = disabled;
                }
            }
            UserServerOverride::Full(full) => {
                if let Some(existing) = result.get_mut(&id) {
                    // Override a built-in server: inherit extensions/root_markers if not specified.
                    let extensions = if full.extensions.is_empty() {
                        existing.extensions.clone()
                    } else {
                        full.extensions
                    };
                    let root_markers = if full.root_markers.is_empty() {
                        existing.root_markers.clone()
                    } else {
                        full.root_markers
                    };
                    *existing = LspServerConfig {
                        extensions,
                        command: full.command,
                        command_candidates: existing.command_candidates.clone(),
                        env: full.env,
                        root_markers,
                        initialization_options: full.initialization_options,
                        disabled: full.disabled,
                        install: existing.install.clone(), // keep built-in install config
                        root_strategy: existing.root_strategy.clone(),
                        post_root_hook: existing.post_root_hook.clone(),
                    };
                } else {
                    // Custom server: user must provide extensions.
                    result.insert(
                        id,
                        LspServerConfig {
                            extensions: full.extensions,
                            command: full.command,
                            command_candidates: Vec::new(),
                            env: full.env,
                            root_markers: full.root_markers,
                            initialization_options: full.initialization_options,
                            disabled: full.disabled,
                            install: None,
                            root_strategy: RootStrategy::default(),
                            post_root_hook: PostRootHook::default(),
                        },
                    );
                }
            }
        }
    }

    result
}

/// User override for a server: either just disable, or provide a full config.
#[derive(Debug, Clone)]
pub enum UserServerOverride {
    DisabledOnly { disabled: bool },
    Full(UserServerFull),
}

/// Full user server configuration.
#[derive(Debug, Clone)]
pub struct UserServerFull {
    pub command: Vec<String>,
    pub extensions: Vec<String>,
    pub root_markers: Vec<String>,
    pub env: HashMap<String, String>,
    pub initialization_options: Option<serde_json::Value>,
    pub disabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin_servers::builtin_servers;

    #[test]
    fn override_command_inherits_extensions() {
        let builtins = builtin_servers();
        let mut overrides = HashMap::new();
        overrides.insert(
            "rust-analyzer".to_string(),
            UserServerOverride::Full(UserServerFull {
                command: vec!["ra-multiplex".into(), "--connect".into()],
                extensions: Vec::new(), // should inherit .rs
                root_markers: Vec::new(),
                env: HashMap::new(),
                initialization_options: None,
                disabled: false,
            }),
        );

        let merged = merge_configs(builtins, overrides);
        let ra = merged.get("rust-analyzer").expect("rust-analyzer present");
        assert_eq!(ra.command, vec!["ra-multiplex", "--connect"]);
        assert_eq!(ra.extensions, vec![".rs"]);
    }

    #[test]
    fn disable_builtin() {
        let builtins = builtin_servers();
        let mut overrides = HashMap::new();
        overrides.insert(
            "gopls".to_string(),
            UserServerOverride::DisabledOnly { disabled: true },
        );

        let merged = merge_configs(builtins, overrides);
        let gopls = merged.get("gopls").expect("gopls present");
        assert!(gopls.disabled);
    }

    #[test]
    fn custom_server_added() {
        let builtins = builtin_servers();
        let mut overrides = HashMap::new();
        overrides.insert(
            "sql-lsp".to_string(),
            UserServerOverride::Full(UserServerFull {
                command: vec![
                    "sql-language-server".into(),
                    "up".into(),
                    "--method".into(),
                    "stdio".into(),
                ],
                extensions: vec![".sql".into()],
                root_markers: Vec::new(),
                env: HashMap::new(),
                initialization_options: None,
                disabled: false,
            }),
        );

        let merged = merge_configs(builtins, overrides);
        assert!(merged.contains_key("sql-lsp"));
        let sql = merged.get("sql-lsp").expect("sql-lsp");
        assert_eq!(sql.extensions, vec![".sql"]);
    }
}
