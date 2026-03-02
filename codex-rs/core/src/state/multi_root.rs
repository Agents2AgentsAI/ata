use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoot {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootStatus {
    pub name: String,
    pub path: PathBuf,
    pub is_primary: bool,
    #[cfg(feature = "lsp")]
    pub has_lsp: bool,
    #[cfg(feature = "treesitter")]
    pub has_treesitter: bool,
}

pub(crate) struct MultiRootState {
    primary_root: String,
    roots: RwLock<Vec<ProjectRoot>>,
    #[cfg(feature = "lsp")]
    lsp_server_configs: Option<HashMap<String, codex_lsp_client::LspServerConfig>>,
    #[cfg(feature = "lsp")]
    lsp_registries: RwLock<HashMap<String, Arc<codex_lsp_client::ServerRegistry>>>,
    #[cfg(feature = "lsp")]
    install_confirm: std::sync::RwLock<Option<codex_lsp_client::server_registry::InstallConfirmFn>>,
    #[cfg(feature = "treesitter")]
    treesitter_config: Option<codex_treesitter::ProjectIndexConfig>,
    #[cfg(feature = "treesitter")]
    treesitter_indices: RwLock<HashMap<String, Arc<codex_treesitter::ProjectIndex>>>,
}

impl std::fmt::Debug for MultiRootState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiRootState")
            .field("primary_root", &self.primary_root)
            .finish_non_exhaustive()
    }
}

impl MultiRootState {
    pub async fn new(
        primary_path: PathBuf,
        #[cfg(feature = "lsp")] lsp_server_configs: Option<
            HashMap<String, codex_lsp_client::LspServerConfig>,
        >,
        #[cfg(feature = "treesitter")] treesitter_config: Option<
            codex_treesitter::ProjectIndexConfig,
        >,
    ) -> Result<Self, String> {
        let primary_name = default_root_name(&primary_path);
        let state = Self {
            primary_root: primary_name.clone(),
            roots: RwLock::new(Vec::new()),
            #[cfg(feature = "lsp")]
            lsp_server_configs,
            #[cfg(feature = "lsp")]
            lsp_registries: RwLock::new(HashMap::new()),
            #[cfg(feature = "lsp")]
            install_confirm: std::sync::RwLock::new(None),
            #[cfg(feature = "treesitter")]
            treesitter_config,
            #[cfg(feature = "treesitter")]
            treesitter_indices: RwLock::new(HashMap::new()),
        };
        state.add_root(primary_name, primary_path).await?;
        Ok(state)
    }

    #[cfg(feature = "lsp")]
    pub fn has_lsp(&self) -> bool {
        self.lsp_server_configs
            .as_ref()
            .is_some_and(|cfg| !cfg.is_empty())
    }

    #[cfg(feature = "treesitter")]
    pub fn has_treesitter(&self) -> bool {
        self.treesitter_config.is_some()
    }

    pub async fn add_root(&self, name: String, path: PathBuf) -> Result<ProjectRoot, String> {
        if name.trim().is_empty() {
            return Err("`root` must be non-empty".to_string());
        }
        if !path.is_absolute() {
            return Err("`path` must be an absolute directory path".to_string());
        }
        if !path.exists() {
            return Err(format!("root path not found: {}", path.display()));
        }
        if !path.is_dir() {
            return Err(format!("root path is not a directory: {}", path.display()));
        }

        let canonical = std::fs::canonicalize(&path)
            .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()))?;
        let root = ProjectRoot {
            name: name.trim().to_string(),
            path: canonical,
        };

        {
            let roots = self.roots.read().await;
            if roots.iter().any(|existing| existing.name == root.name) {
                return Err(format!("root '{}' already exists", root.name));
            }
            if roots.iter().any(|existing| existing.path == root.path) {
                return Err(format!(
                    "root path already registered: {}",
                    root.path.display()
                ));
            }
        }

        #[cfg(feature = "treesitter")]
        let treesitter_index = if let Some(config) = &self.treesitter_config {
            let config = config.clone();
            let root_path = root.path.clone();
            let index = tokio::task::spawn_blocking(move || {
                codex_treesitter::ProjectIndex::new_with_config(root_path, config)
            })
            .await
            .map_err(|error| format!("TreeSitter initialization task failed: {error}"))?
            .map_err(|error| format!("failed to initialize TreeSitter index: {error}"))?;
            Some(Arc::new(index))
        } else {
            None
        };

        #[cfg(feature = "lsp")]
        let lsp_registry = if let Some(servers) = &self.lsp_server_configs {
            let callback = self
                .install_confirm
                .read()
                .ok()
                .and_then(|guard| guard.clone());
            Some(Arc::new(codex_lsp_client::ServerRegistry::new(
                servers.clone(),
                root.path.clone(),
                callback,
            )))
        } else {
            None
        };

        {
            let mut roots = self.roots.write().await;
            if roots.iter().any(|existing| existing.name == root.name) {
                return Err(format!("root '{}' already exists", root.name));
            }
            if roots.iter().any(|existing| existing.path == root.path) {
                return Err(format!(
                    "root path already registered: {}",
                    root.path.display()
                ));
            }
            roots.push(root.clone());
        }

        #[cfg(feature = "treesitter")]
        if let Some(index) = treesitter_index {
            self.treesitter_indices
                .write()
                .await
                .insert(root.name.clone(), index);
        }

        #[cfg(feature = "lsp")]
        if let Some(registry) = lsp_registry {
            self.lsp_registries
                .write()
                .await
                .insert(root.name.clone(), registry);
        }

        Ok(root)
    }

    pub async fn remove_root(&self, name: &str) -> Result<(), String> {
        if name == self.primary_root {
            return Err("cannot remove the primary root".to_string());
        }

        let removed = {
            let mut roots = self.roots.write().await;
            if let Some(pos) = roots.iter().position(|root| root.name == name) {
                roots.remove(pos);
                true
            } else {
                false
            }
        };

        if !removed {
            return Err(format!("unknown root '{name}'"));
        }

        #[cfg(feature = "treesitter")]
        self.treesitter_indices.write().await.remove(name);

        #[cfg(feature = "lsp")]
        let removed_registry = self.lsp_registries.write().await.remove(name);
        #[cfg(feature = "lsp")]
        if let Some(registry) = removed_registry {
            registry.shutdown_all().await;
        }

        Ok(())
    }

    pub async fn roots(&self) -> Vec<ProjectRoot> {
        self.roots.read().await.clone()
    }

    pub async fn root_statuses(&self) -> Vec<RootStatus> {
        let roots = self.roots().await;
        #[cfg(feature = "lsp")]
        let lsp_registries = self.lsp_registries.read().await;
        #[cfg(feature = "treesitter")]
        let treesitter_indices = self.treesitter_indices.read().await;

        roots
            .into_iter()
            .map(|root| RootStatus {
                is_primary: root.name == self.primary_root,
                #[cfg(feature = "lsp")]
                has_lsp: lsp_registries.contains_key(&root.name),
                #[cfg(feature = "treesitter")]
                has_treesitter: treesitter_indices.contains_key(&root.name),
                name: root.name,
                path: root.path,
            })
            .collect()
    }

    async fn resolve_root(
        &self,
        root_name: Option<&str>,
        file: Option<&Path>,
    ) -> Option<ProjectRoot> {
        if let Some(root_name) = root_name {
            let roots = self.roots.read().await;
            return roots.iter().find(|root| root.name == root_name).cloned();
        }

        if let Some(file) = file {
            return self.root_for_file(file).await;
        }

        let roots = self.roots.read().await;
        roots
            .iter()
            .find(|root| root.name == self.primary_root)
            .cloned()
    }

    async fn root_for_file(&self, file: &Path) -> Option<ProjectRoot> {
        let roots = self.roots.read().await;
        roots
            .iter()
            .filter(|root| file.starts_with(&root.path))
            .max_by_key(|root| root.path.components().count())
            .cloned()
    }

    #[cfg(feature = "treesitter")]
    pub async fn treesitter_index_for_file(
        &self,
        file: &Path,
        root_name: Option<&str>,
    ) -> Option<(String, Arc<codex_treesitter::ProjectIndex>)> {
        let root = self.resolve_root(root_name, Some(file)).await?;
        let indices = self.treesitter_indices.read().await;
        indices
            .get(&root.name)
            .cloned()
            .map(|index| (root.name, index))
    }

    #[cfg(feature = "treesitter")]
    pub async fn treesitter_indices(
        &self,
        root_name: Option<&str>,
    ) -> Vec<(String, Arc<codex_treesitter::ProjectIndex>)> {
        if let Some(root_name) = root_name {
            let indices = self.treesitter_indices.read().await;
            return indices
                .get(root_name)
                .cloned()
                .map(|index| vec![(root_name.to_string(), index)])
                .unwrap_or_default();
        }

        let roots = self.roots().await;
        let indices = self.treesitter_indices.read().await;
        roots
            .into_iter()
            .filter_map(|root| {
                indices
                    .get(&root.name)
                    .cloned()
                    .map(|index| (root.name, index))
            })
            .collect()
    }

    #[cfg(feature = "treesitter")]
    pub async fn reindex_file(&self, file: &Path) {
        if let Some((_, index)) = self.treesitter_index_for_file(file, None).await
            && let Err(error) = index.reindex_absolute_path(file)
        {
            tracing::debug!("tree-sitter reindex failed for {}: {error}", file.display());
        }
    }

    #[cfg(feature = "lsp")]
    pub async fn lsp_registry_for_file(
        &self,
        file: &Path,
        root_name: Option<&str>,
    ) -> Option<(String, Arc<codex_lsp_client::ServerRegistry>)> {
        let root = self.resolve_root(root_name, Some(file)).await?;
        let registries = self.lsp_registries.read().await;
        registries
            .get(&root.name)
            .cloned()
            .map(|registry| (root.name, registry))
    }

    #[cfg(feature = "lsp")]
    pub async fn lsp_registries(
        &self,
        root_name: Option<&str>,
    ) -> Vec<(String, Arc<codex_lsp_client::ServerRegistry>)> {
        if let Some(root_name) = root_name {
            let registries = self.lsp_registries.read().await;
            return registries
                .get(root_name)
                .cloned()
                .map(|registry| vec![(root_name.to_string(), registry)])
                .unwrap_or_default();
        }

        let roots = self.roots().await;
        let registries = self.lsp_registries.read().await;
        roots
            .into_iter()
            .filter_map(|root| {
                registries
                    .get(&root.name)
                    .cloned()
                    .map(|registry| (root.name, registry))
            })
            .collect()
    }

    #[cfg(feature = "lsp")]
    pub async fn touch_lsp_nowait(&self, file: &Path) {
        if let Some((_, registry)) = self.lsp_registry_for_file(file, None).await {
            let _ = registry.touch_file(file, false).await;
        }
    }

    #[cfg(feature = "lsp")]
    pub async fn touch_lsp_and_collect_errors(&self, file: &Path) -> String {
        const MAX_DIAGNOSTICS_PER_FILE: usize = 20;

        let Some((_, registry)) = self.lsp_registry_for_file(file, None).await else {
            return String::new();
        };

        let all_diags = registry.touch_file(file, true).await;
        let mut errors = Vec::new();

        for diags in all_diags.values() {
            for diag in diags {
                if diag.severity == Some(codex_lsp_client::lsp_types::DiagnosticSeverity::ERROR) {
                    let line = diag.range.start.line + 1;
                    let col = diag.range.start.character + 1;
                    errors.push(format!("ERROR [{line}:{col}] {}", diag.message));
                }
            }
        }

        if errors.is_empty() {
            return String::new();
        }

        let display_path = file.display();
        let remaining = errors.len().saturating_sub(MAX_DIAGNOSTICS_PER_FILE);
        let mut out = format!(
            "\nLSP errors detected in {display_path}, please fix:\n<diagnostics file=\"{display_path}\">\n"
        );
        for error in errors.iter().take(MAX_DIAGNOSTICS_PER_FILE) {
            out.push_str(error);
            out.push('\n');
        }
        if remaining > 0 {
            out.push_str(&format!("... and {remaining} more\n"));
        }
        out.push_str("</diagnostics>");
        out
    }

    #[cfg(feature = "lsp")]
    pub async fn set_install_confirm(
        &self,
        callback: Option<codex_lsp_client::server_registry::InstallConfirmFn>,
    ) {
        if let Ok(mut guard) = self.install_confirm.write() {
            *guard = callback.clone();
        }

        let registries = self.lsp_registries.read().await;
        for registry in registries.values() {
            registry.set_install_confirm(callback.clone());
        }
    }

    pub async fn shutdown_all(&self) {
        #[cfg(feature = "lsp")]
        {
            let registries: Vec<_> = {
                let mut map = self.lsp_registries.write().await;
                map.drain().map(|(_, registry)| registry).collect()
            };
            for registry in registries {
                registry.shutdown_all().await;
            }
        }

        #[cfg(feature = "treesitter")]
        self.treesitter_indices.write().await.clear();
        self.roots.write().await.clear();
    }
}

fn default_root_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| "root".to_string())
}
