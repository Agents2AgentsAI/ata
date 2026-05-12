use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "treesitter")]
use crate::file_watcher::FileWatcher;
#[cfg(feature = "treesitter")]
use crate::file_watcher::Receiver as FileWatcherReceiver;
#[cfg(feature = "treesitter")]
use crate::file_watcher::WatchRegistration;
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
    #[cfg(feature = "lsp")]
    install_tracker: Arc<codex_lsp_client::InstallTracker>,
    /// Shared LSP `ServerRegistry` pool owned by `AgentControl` and cloned into
    /// every subagent of this root thread. When `add_root` fires we reuse an
    /// existing registry for the same canonical path instead of spawning new
    /// LSP servers (e.g. rust-analyzer) for every subagent.
    #[cfg(feature = "lsp")]
    shared_lsp_registry_cache: crate::agent::control::SharedLspRegistryCache,
    #[cfg(feature = "treesitter")]
    treesitter_config: Option<codex_treesitter::ProjectIndexConfig>,
    #[cfg(feature = "treesitter")]
    treesitter_indices: RwLock<HashMap<String, Arc<TreeSitterIndex>>>,
    #[cfg(feature = "treesitter")]
    file_watcher: Arc<FileWatcher>,
    #[cfg(feature = "treesitter")]
    watch_subscriber: crate::file_watcher::FileWatcherSubscriber,
    #[cfg(feature = "treesitter")]
    watch_registrations: RwLock<HashMap<String, WatchRegistration>>,
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
        #[cfg(feature = "lsp")] install_tracker: Arc<codex_lsp_client::InstallTracker>,
        #[cfg(feature = "lsp")]
        shared_lsp_registry_cache: crate::agent::control::SharedLspRegistryCache,
        #[cfg(feature = "treesitter")] treesitter_config: Option<
            codex_treesitter::ProjectIndexConfig,
        >,
    ) -> Result<Arc<Self>, String> {
        let primary_name = default_root_name(&primary_path);
        #[cfg(feature = "treesitter")]
        let file_watcher = match FileWatcher::new() {
            Ok(file_watcher) => Arc::new(file_watcher),
            Err(err) => {
                tracing::warn!("failed to initialize code-intel file watcher: {err}");
                Arc::new(FileWatcher::noop())
            }
        };
        #[cfg(feature = "treesitter")]
        let (watch_subscriber, watch_rx) = file_watcher.add_subscriber();

        let state = Arc::new(Self {
            primary_root: primary_name.clone(),
            roots: RwLock::new(Vec::new()),
            #[cfg(feature = "lsp")]
            lsp_server_configs,
            #[cfg(feature = "lsp")]
            lsp_registries: RwLock::new(HashMap::new()),
            #[cfg(feature = "lsp")]
            install_confirm: std::sync::RwLock::new(None),
            #[cfg(feature = "lsp")]
            install_tracker,
            #[cfg(feature = "lsp")]
            shared_lsp_registry_cache,
            #[cfg(feature = "treesitter")]
            treesitter_config,
            #[cfg(feature = "treesitter")]
            treesitter_indices: RwLock::new(HashMap::new()),
            #[cfg(feature = "treesitter")]
            file_watcher,
            #[cfg(feature = "treesitter")]
            watch_subscriber,
            #[cfg(feature = "treesitter")]
            watch_registrations: RwLock::new(HashMap::new()),
        });
        state.add_root(primary_name, primary_path).await?;
        #[cfg(feature = "treesitter")]
        if state.treesitter_config.is_some() {
            state.start_file_watcher_listener(watch_rx);
        }
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

    #[cfg(feature = "treesitter")]
    fn start_file_watcher_listener(self: &Arc<Self>, mut rx: FileWatcherReceiver) {
        let weak_state = Arc::downgrade(self);
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let Some(state) = weak_state.upgrade() else {
                    break;
                };
                for path in event.paths {
                    state.reindex_file(&path).await;
                }
            }
        });
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

        // Reuse a `ServerRegistry` already spawned for this root by a sibling
        // subagent — otherwise every spawn_agent() fires up another
        // rust-analyzer / pyright / etc. per workspace root.
        #[cfg(feature = "lsp")]
        let lsp_registry = if let Some(servers) = &self.lsp_server_configs
            && !servers.is_empty()
        {
            let mut cache = self.shared_lsp_registry_cache.write().await;
            if let Some(existing) = cache.get(&root.path) {
                Some(Arc::clone(existing))
            } else {
                let callback = self
                    .install_confirm
                    .read()
                    .ok()
                    .and_then(|guard| guard.clone());
                let registry = Arc::new(codex_lsp_client::ServerRegistry::with_install_tracker(
                    servers.clone(),
                    root.path.clone(),
                    callback,
                    Some(Arc::clone(&self.install_tracker)),
                ));
                cache.insert(root.path.clone(), Arc::clone(&registry));
                Some(registry)
            }
        } else {
            None
        };

        #[cfg(feature = "treesitter")]
        if let Some(index) = treesitter_index {
            self.treesitter_indices
                .write()
                .await
                .insert(root.name.clone(), index);
            let registration =
                self.watch_subscriber
                    .register_paths(vec![crate::file_watcher::WatchPath {
                        path: root.path.clone(),
                        recursive: true,
                    }]);
            self.watch_registrations
                .write()
                .await
                .insert(root.name.clone(), registration);
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

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "Holds the shared-LSP-registry write lock across `registry.shutdown_all().await` intentionally: shutdown must observe an atomic 'last user is leaving' transition. The cache is module-private and no other code path tries to acquire it after this point, so the long hold cannot deadlock."
    )]
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
        #[cfg(feature = "treesitter")]
        self.watch_registrations.write().await.remove(name);

        #[cfg(feature = "lsp")]
        let removed_registry = self.lsp_registries.write().await.remove(name);
        #[cfg(feature = "lsp")]
        if let Some(registry) = removed_registry {
            // Only shut the LSP servers down if this session was the last user
            // of the shared registry — sibling subagents may still be using it.
            // `Arc::strong_count == 2` means just this local binding + the
            // shared cache entry (this session's handle has already been
            // removed from `self.lsp_registries` above).
            let mut cache = self.shared_lsp_registry_cache.write().await;
            if Arc::strong_count(&registry) <= 2 {
                cache.retain(|_, cached| !Arc::ptr_eq(cached, &registry));
                drop(cache);
                registry.shutdown_all().await;
            }
        }

        Ok(())
    }

    pub async fn roots(&self) -> Vec<ProjectRoot> {
        self.roots.read().await.clone()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "Snapshots LSP + treesitter registries by holding their read locks across `try_ready().await` on the inner index state. The awaited future never re-enters these RwLocks, so there is no deadlock; releasing and re-locking per-entry would race with concurrent `add_root` mutations."
    )]
    pub async fn root_statuses(&self) -> Vec<RootStatus> {
        let roots = self.roots().await;
        #[cfg(feature = "lsp")]
        let lsp_registries = self.lsp_registries.read().await;
        #[cfg(feature = "treesitter")]
        let treesitter_indices = self.treesitter_indices.read().await;

        let mut statuses = Vec::with_capacity(roots.len());
        for root in roots {
            #[cfg(feature = "treesitter")]
            let has_treesitter = match treesitter_indices.get(&root.name) {
                Some(idx) => idx.try_ready().await.is_some(),
                None => false,
            };

            statuses.push(RootStatus {
                is_primary: root.name == self.primary_root,
                #[cfg(feature = "lsp")]
                has_lsp: lsp_registries.contains_key(&root.name),
                #[cfg(feature = "treesitter")]
                has_treesitter,
                name: root.name,
                path: root.path,
            });
        }
        statuses
    }

    async fn resolve_root(
        &self,
        root_name: Option<&str>,
        file: Option<&Path>,
    ) -> Option<ProjectRoot> {
        let canonical_file = file.map(canonicalize_query_path);
        if let Some(root_name) = root_name {
            let roots = self.roots.read().await;
            let root = roots.iter().find(|root| root.name == root_name).cloned();
            // When both root name and file are provided, enforce that the file
            // actually lives inside the requested root.
            if let (Some(root), Some(file)) = (&root, canonical_file.as_ref())
                && !file.starts_with(&root.path)
            {
                return None;
            }
            return root;
        }

        if let Some(file) = canonical_file.as_deref() {
            return self.root_for_file(file).await;
        }

        let roots = self.roots.read().await;
        roots
            .iter()
            .find(|root| root.name == self.primary_root)
            .cloned()
    }

    async fn root_for_file(&self, file: &Path) -> Option<ProjectRoot> {
        let file = canonicalize_query_path(file);
        let roots = self.roots.read().await;
        roots
            .iter()
            .filter(|root| file.starts_with(&root.path))
            .max_by_key(|root| root.path.components().count())
            .cloned()
    }

    #[cfg(feature = "treesitter")]
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "Holds the treesitter index map read lock across `wait_ready().await` so a concurrent `add_root` can't remove the entry mid-wait. The awaited future only touches the per-root index state, never the outer map."
    )]
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
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "The `indices` guard is dropped explicitly before the `try_ready().await` below; clippy can't see the explicit drop and assumes the worst. No deadlock risk."
    )]
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
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "Index map read guards are dropped explicitly before the inner `wait_ready().await` calls. Clippy can't see the explicit drops; the awaited futures touch only per-root state, not the outer map."
    )]
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
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "Best-effort reindex: holds the index map read lock across `try_ready().await` to find the matching index. The awaited future doesn't re-enter the lock; concurrent `add_root` mutations would race with our index lookup anyway."
    )]
    pub async fn reindex_file(&self, file: &Path) {
        // Best-effort; don't block tool calls waiting for background index build.
        let canonical_file = canonicalize_query_path(file);
        let root = self.root_for_file(&canonical_file).await;
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

        let file = canonical_file;
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
    pub async fn shed_lsp_clients(&self, count_per_registry: usize) -> usize {
        let registries: Vec<Arc<codex_lsp_client::ServerRegistry>> = {
            let registries = self.lsp_registries.read().await;
            registries.values().cloned().collect()
        };
        let mut total = 0;
        for registry in registries {
            total += registry.shed_clients(count_per_registry).await;
        }
        total
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

    /// Collect raw LSP error diagnostics for a file.
    ///
    /// Returns `(line_1based, col_1based, message)` tuples for ERROR-severity
    /// diagnostics. Touches the file so the server re-reads it from disk.
    #[cfg(feature = "lsp")]
    pub async fn collect_lsp_errors(&self, file: &Path) -> Vec<(u32, u32, String)> {
        let Some((_, registry)) = self.lsp_registry_for_file(file, None).await else {
            return Vec::new();
        };

        let all_diags = registry.touch_file(file, true).await;
        let mut errors = Vec::new();

        for diags in all_diags.values() {
            for diag in diags {
                if diag.severity == Some(codex_lsp_client::lsp_types::DiagnosticSeverity::ERROR) {
                    let line = diag.range.start.line + 1;
                    let col = diag.range.start.character + 1;
                    errors.push((line, col, diag.message.clone()));
                }
            }
        }

        errors
    }

    /// Touch a file and return formatted LSP error diagnostics.
    ///
    /// Used by the auto-feedback path after `apply_patch`. If `baseline` is
    /// provided, only errors whose message text is NOT in the baseline set are
    /// included (i.e. only *new* errors introduced by the patch).
    #[cfg(feature = "lsp")]
    pub async fn touch_lsp_and_collect_errors(
        &self,
        file: &Path,
        baseline: Option<&std::collections::HashSet<String>>,
    ) -> String {
        /// Max errors shown in auto-feedback after a patch.
        const MAX_DIAGNOSTICS_AUTO_FEEDBACK: usize = 5;

        let raw_errors = self.collect_lsp_errors(file).await;

        let errors: Vec<String> = raw_errors
            .into_iter()
            .filter(|(_, _, msg)| baseline.as_ref().is_none_or(|bl| !bl.contains(msg)))
            .map(|(line, col, msg)| format!("ERROR [{line}:{col}] {msg}"))
            .collect();

        if errors.is_empty() {
            return String::new();
        }

        let display_path = file.display();
        let remaining = errors.len().saturating_sub(MAX_DIAGNOSTICS_AUTO_FEEDBACK);
        let header = if baseline.is_some() {
            "New errors from this patch"
        } else {
            "LSP errors detected"
        };
        let mut out = format!(
            "\n{header} in {display_path}, please fix:\n<diagnostics file=\"{display_path}\">\n"
        );
        for error in errors.iter().take(MAX_DIAGNOSTICS_AUTO_FEEDBACK) {
            out.push_str(error);
            out.push('\n');
        }
        if remaining > 0 {
            out.push_str(&format!(
                "... and {remaining} more (use `lsp.diagnostics` to see all)\n"
            ));
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

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "Drains the shared LSP registry cache under its write lock before awaiting `shutdown_all` on each retained registry. The cache is module-private and shutdown only touches per-registry state, not the cache itself, so the long hold is safe."
    )]
    pub async fn shutdown_all(&self) {
        #[cfg(feature = "lsp")]
        {
            let registries: Vec<_> = {
                let mut map = self.lsp_registries.write().await;
                map.drain().map(|(_, registry)| registry).collect()
            };
            // Only shut down LSP servers whose registry is not still held by
            // a sibling `MultiRootState` (e.g. another live subagent sharing
            // the same workspace root via `shared_lsp_registry_cache`).
            // `Arc::strong_count <= 2` means only this local binding plus the
            // shared cache entry remain — safe to tear down. Otherwise leave
            // the LSP running so siblings keep a working registry.
            let mut cache = self.shared_lsp_registry_cache.write().await;
            let mut to_shutdown = Vec::new();
            for registry in registries {
                if Arc::strong_count(&registry) <= 2 {
                    cache.retain(|_, cached| !Arc::ptr_eq(cached, &registry));
                    to_shutdown.push(registry);
                }
            }
            drop(cache);
            for registry in to_shutdown {
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

fn canonicalize_query_path(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(all(test, feature = "lsp"))]
mod tests {
    use super::MultiRootState;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[tokio::test]
    async fn new_uses_provided_install_tracker() {
        let primary_root = tempfile::tempdir().expect("create primary root");
        let tracker = Arc::new(codex_lsp_client::InstallTracker::new());
        let cache = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let state = MultiRootState::new(
            primary_root.path().to_path_buf(),
            Some(Default::default()),
            Arc::clone(&tracker),
            cache,
            #[cfg(feature = "treesitter")]
            None,
        )
        .await
        .expect("create multi-root state");

        assert_eq!(Arc::ptr_eq(&state.install_tracker, &tracker), true);
    }

    /// Regression: two sibling subagents with the same primary cwd must share
    /// the `ServerRegistry` for that root, so we do not spawn a second
    /// rust-analyzer/pyright/etc per subagent. Cf. OOM incident 2026-04-21.
    #[tokio::test]
    async fn shared_registry_cache_is_reused_across_multi_root_states() {
        let primary_root = tempfile::tempdir().expect("create primary root");
        // Provide a non-empty LSP server config so `add_root` actually builds a
        // registry. We use an arbitrary server definition that will never be
        // launched (no file opens happen in this test).
        let mut servers: HashMap<String, codex_lsp_client::LspServerConfig> = HashMap::new();
        servers.insert(
            "noop".to_string(),
            codex_lsp_client::LspServerConfig::new(
                vec![".noop".to_string()],
                vec!["/bin/true".to_string()],
                Vec::new(),
            ),
        );
        let tracker = Arc::new(codex_lsp_client::InstallTracker::new());
        let cache = Arc::new(tokio::sync::RwLock::new(HashMap::new()));

        let parent = MultiRootState::new(
            primary_root.path().to_path_buf(),
            Some(servers.clone()),
            Arc::clone(&tracker),
            Arc::clone(&cache),
            #[cfg(feature = "treesitter")]
            None,
        )
        .await
        .expect("create parent multi-root state");

        let subagent = MultiRootState::new(
            primary_root.path().to_path_buf(),
            Some(servers),
            Arc::clone(&tracker),
            Arc::clone(&cache),
            #[cfg(feature = "treesitter")]
            None,
        )
        .await
        .expect("create subagent multi-root state");

        let parent_registry = {
            let registries = parent.lsp_registries.read().await;
            registries
                .values()
                .next()
                .cloned()
                .expect("parent has a registry")
        };
        let subagent_registry = {
            let registries = subagent.lsp_registries.read().await;
            registries
                .values()
                .next()
                .cloned()
                .expect("subagent has a registry")
        };
        assert!(
            Arc::ptr_eq(&parent_registry, &subagent_registry),
            "subagent must reuse parent's ServerRegistry for the same root",
        );
        assert_eq!(cache.read().await.len(), 1);
    }

    /// Regression: a subagent's `shutdown_all` must NOT tear down the shared
    /// `ServerRegistry` while the parent (or another sibling) still holds it.
    /// Without the `Arc::strong_count` gate, each subagent's `Op::Shutdown`
    /// killed the parent's rust-analyzer, causing spawn/kill churn and
    /// eventually stacking orphaned processes. Cf. OOM incident 2026-04-21.
    #[tokio::test]
    async fn subagent_shutdown_leaves_parent_registry_alive() {
        let primary_root = tempfile::tempdir().expect("create primary root");
        let mut servers: HashMap<String, codex_lsp_client::LspServerConfig> = HashMap::new();
        servers.insert(
            "noop".to_string(),
            codex_lsp_client::LspServerConfig::new(
                vec![".noop".to_string()],
                vec!["/bin/true".to_string()],
                Vec::new(),
            ),
        );
        let tracker = Arc::new(codex_lsp_client::InstallTracker::new());
        let cache = Arc::new(tokio::sync::RwLock::new(HashMap::new()));

        let parent = MultiRootState::new(
            primary_root.path().to_path_buf(),
            Some(servers.clone()),
            Arc::clone(&tracker),
            Arc::clone(&cache),
            #[cfg(feature = "treesitter")]
            None,
        )
        .await
        .expect("create parent multi-root state");

        let subagent = MultiRootState::new(
            primary_root.path().to_path_buf(),
            Some(servers),
            Arc::clone(&tracker),
            Arc::clone(&cache),
            #[cfg(feature = "treesitter")]
            None,
        )
        .await
        .expect("create subagent multi-root state");

        // Sanity: both point at the same Arc and the cache has it.
        let parent_registry = parent
            .lsp_registries
            .read()
            .await
            .values()
            .next()
            .cloned()
            .expect("parent has a registry");
        assert_eq!(cache.read().await.len(), 1);

        // Subagent tears down — parent + cache still reference the registry,
        // so `strong_count` is 3 (parent local + subagent local + cache).
        subagent.shutdown_all().await;

        // Parent's registry must still be in the cache and still be the same
        // Arc — subagent shutdown must not have evicted or replaced it.
        assert_eq!(
            cache.read().await.len(),
            1,
            "subagent shutdown must not evict the shared registry from the cache",
        );
        let cached_registry = cache
            .read()
            .await
            .values()
            .next()
            .cloned()
            .expect("cache still has the registry");
        assert!(
            Arc::ptr_eq(&cached_registry, &parent_registry),
            "subagent shutdown must leave the same Arc in the cache",
        );
        assert!(
            !parent.lsp_registries.read().await.is_empty(),
            "parent's lsp_registries map must still contain the registry",
        );
    }
}
