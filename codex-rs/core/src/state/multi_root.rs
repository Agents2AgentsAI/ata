use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "treesitter")]
use tokio::sync::Notify;
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
    install_confirm: std::sync::RwLock<Option<codex_lsp_client::server_registry::InstallRunnerFn>>,
    #[cfg(feature = "treesitter")]
    treesitter_config: Option<codex_treesitter::ProjectIndexConfig>,
    #[cfg(feature = "treesitter")]
    treesitter_indices: RwLock<HashMap<String, Arc<TreeSitterIndex>>>,
}

#[cfg(feature = "treesitter")]
#[derive(Debug, Clone)]
enum TreeSitterIndexState {
    Building,
    Ready(Arc<codex_treesitter::ProjectIndex>),
    Failed(String),
}

/// Tree-sitter index initialization is expensive on large repos. We build it in
/// the background so session startup stays fast, and await it only when a tool
/// actually needs the index.
#[cfg(feature = "treesitter")]
#[derive(Debug)]
struct TreeSitterIndex {
    state: tokio::sync::Mutex<TreeSitterIndexState>,
    notify: Notify,
}

#[cfg(feature = "treesitter")]
impl TreeSitterIndex {
    fn new() -> Self {
        Self {
            state: tokio::sync::Mutex::new(TreeSitterIndexState::Building),
            notify: Notify::new(),
        }
    }

    async fn set_ready(&self, index: codex_treesitter::ProjectIndex) {
        let mut guard = self.state.lock().await;
        *guard = TreeSitterIndexState::Ready(Arc::new(index));
        drop(guard);
        self.notify.notify_waiters();
    }

    async fn set_failed(&self, error: String) {
        let mut guard = self.state.lock().await;
        *guard = TreeSitterIndexState::Failed(error);
        drop(guard);
        self.notify.notify_waiters();
    }

    async fn wait_ready(&self) -> Result<Arc<codex_treesitter::ProjectIndex>, String> {
        loop {
            // Fast path: check current state.
            let snapshot = {
                let guard = self.state.lock().await;
                (*guard).clone()
            };
            match snapshot {
                TreeSitterIndexState::Ready(index) => return Ok(index),
                TreeSitterIndexState::Failed(error) => return Err(error),
                TreeSitterIndexState::Building => {
                    // Wait until builder notifies, then re-check.
                }
            }
            self.notify.notified().await;
        }
    }

    async fn try_ready(&self) -> Option<Arc<codex_treesitter::ProjectIndex>> {
        let guard = self.state.lock().await;
        match &*guard {
            TreeSitterIndexState::Ready(index) => Some(Arc::clone(index)),
            TreeSitterIndexState::Building | TreeSitterIndexState::Failed(_) => None,
        }
    }
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
        let treesitter_index = if let Some(config) = &self.treesitter_config {
            // Expensive: build in the background to avoid blocking session startup.
            let idx = Arc::new(TreeSitterIndex::new());
            let root_path = root.path.clone();
            let config = config.clone();
            let idx_for_task = Arc::clone(&idx);
            let root_name = root.name.clone();

            tracing::info!(
                root = %root_name,
                "scheduling tree-sitter index build in background"
            );

            tokio::spawn(async move {
                let start = std::time::Instant::now();
                let build = tokio::task::spawn_blocking(move || {
                    codex_treesitter::ProjectIndex::new_with_config(root_path, config)
                })
                .await;

                match build {
                    Ok(Ok(index)) => {
                        let files = index.file_tree().len();
                        let symbols = index.symbol_table().len();
                        tracing::info!(
                            root = %root_name,
                            files,
                            symbols,
                            elapsed_ms = start.elapsed().as_millis(),
                            "tree-sitter index built"
                        );
                        idx_for_task.set_ready(index).await;
                    }
                    Ok(Err(error)) => {
                        let msg = format!("failed to initialize TreeSitter index: {error}");
                        tracing::warn!(root = %root_name, "tree-sitter index build failed: {msg}");
                        idx_for_task.set_failed(msg).await;
                    }
                    Err(error) => {
                        let msg = format!("TreeSitter initialization task failed: {error}");
                        tracing::warn!(root = %root_name, "tree-sitter index build failed: {msg}");
                        idx_for_task.set_failed(msg).await;
                    }
                }
            });

            Some(idx)
        } else {
            None
        };

        #[cfg(feature = "lsp")]
        let lsp_registry = if let Some(servers) = &self.lsp_server_configs
            && !servers.is_empty()
        {
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

        #[cfg(feature = "treesitter")]
        if let Some(index) = treesitter_index {
            self.treesitter_indices
                .write()
                .await
                .insert(root.name.clone(), index);
        }

        #[cfg(feature = "lsp")]
        if let Some(registry) = lsp_registry {
            let registry_for_prewarm = Arc::clone(&registry);
            self.lsp_registries
                .write()
                .await
                .insert(root.name.clone(), registry);

            tokio::spawn(async move {
                registry_for_prewarm.prewarm_workspace_clients().await;
            });
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
            let root = roots.iter().find(|root| root.name == root_name).cloned();
            // When both root name and file are provided, enforce that the file
            // actually lives inside the requested root.
            if let (Some(root), Some(file)) = (&root, file) {
                if !file.starts_with(&root.path) {
                    return None;
                }
            }
            return root;
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
    ) -> Result<Option<(String, Arc<codex_treesitter::ProjectIndex>)>, String> {
        let Some(root) = self.resolve_root(root_name, Some(file)).await else {
            return Ok(None);
        };
        let indices = self.treesitter_indices.read().await;
        let idx = indices.get(&root.name).cloned();
        drop(indices);

        let Some(idx) = idx else {
            return Ok(None);
        };

        let index = idx.wait_ready().await?;
        Ok(Some((root.name, index)))
    }

    /// Best-effort, non-blocking lookup of the tree-sitter index for a file.
    ///
    /// Returns `None` if:
    /// - the file is not under a registered root,
    /// - the root has no tree-sitter index,
    /// - the index is still building or has failed.
    #[cfg(feature = "treesitter")]
    pub async fn try_treesitter_index_for_file(
        &self,
        file: &Path,
        root_name: Option<&str>,
    ) -> Option<(String, Arc<codex_treesitter::ProjectIndex>)> {
        let root = self.resolve_root(root_name, Some(file)).await?;
        let indices = self.treesitter_indices.read().await;
        let idx = indices.get(&root.name).cloned();
        drop(indices);

        let idx = idx?;
        let index = idx.try_ready().await?;
        Some((root.name, index))
    }

    #[cfg(feature = "treesitter")]
    pub async fn treesitter_indices(
        &self,
        root_name: Option<&str>,
    ) -> Result<Vec<(String, Arc<codex_treesitter::ProjectIndex>)>, String> {
        if let Some(root_name) = root_name {
            let indices = self.treesitter_indices.read().await;
            let idx = indices.get(root_name).cloned();
            drop(indices);
            let Some(idx) = idx else {
                return Ok(Vec::new());
            };
            return Ok(vec![(root_name.to_string(), idx.wait_ready().await?)]);
        }

        let roots = self.roots().await;
        let indices = self.treesitter_indices.read().await;
        let idxs: Vec<(String, Arc<TreeSitterIndex>)> = roots
            .into_iter()
            .filter_map(|root| indices.get(&root.name).cloned().map(|idx| (root.name, idx)))
            .collect();
        drop(indices);

        let mut out = Vec::with_capacity(idxs.len());
        for (root_name, idx) in idxs {
            out.push((root_name, idx.wait_ready().await?));
        }
        Ok(out)
    }

    #[cfg(feature = "treesitter")]
    pub async fn reindex_file(&self, file: &Path) {
        // Best-effort; don't block tool calls waiting for background index build.
        let root = self.root_for_file(file).await;
        let Some(root) = root else {
            return;
        };

        let indices = self.treesitter_indices.read().await;
        let idx = indices.get(&root.name).cloned();
        drop(indices);

        let Some(idx) = idx else {
            return;
        };
        let Some(index) = idx.try_ready().await else {
            return;
        };

        let file = file.to_path_buf();
        let root_name = root.name.clone();
        let index = Arc::clone(&index);
        tokio::task::spawn_blocking(move || {
            if let Err(error) = index.reindex_absolute_path(&file) {
                tracing::debug!(
                    root = %root_name,
                    "tree-sitter reindex failed for {}: {error}",
                    file.display()
                );
            }
        })
        .await
        .ok();
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
        callback: Option<codex_lsp_client::server_registry::InstallRunnerFn>,
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
