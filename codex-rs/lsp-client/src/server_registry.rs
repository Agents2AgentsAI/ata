//! Registry that manages a pool of LSP clients, one per (server_id, root) pair.

use std::collections::HashMap;
use std::collections::HashSet;
use std::future::Future;
use std::hash::Hash;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::RwLock;

use lsp_types::request::GotoImplementationResponse;
use lsp_types::*;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinSet;
use tracing;

use crate::client::LspClient;
use crate::client::path_from_uri;
use crate::error::LspError;
use crate::root_discovery::dir_has_any_marker;
use crate::root_discovery::nearest_root;
use crate::root_discovery::refine_root;
use crate::server_config::LspServerConfig;

/// Key for de-duplicating clients: (server_id, root_path).
type ClientKey = (String, PathBuf);

const PREFLIGHT_VERSION_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);
const PREFLIGHT_OUTPUT_MAX_BYTES: usize = 8 * 1024;
const CALL_HIERARCHY_SERVER_ID_KEY: &str = "__codex_server_id";

#[derive(Debug)]
enum PreflightResult {
    Installed,
    NeedsInstall(&'static str),
    Inconclusive,
}

fn lock_unpoisoned<T>(mutex: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct SpawnCleanupGuard<'a> {
    spawning: &'a StdMutex<HashMap<ClientKey, Arc<tokio::sync::Notify>>>,
    key: ClientKey,
    notify: Arc<tokio::sync::Notify>,
}

impl<'a> SpawnCleanupGuard<'a> {
    fn new(
        spawning: &'a StdMutex<HashMap<ClientKey, Arc<tokio::sync::Notify>>>,
        key: ClientKey,
        notify: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self {
            spawning,
            key,
            notify,
        }
    }
}

impl Drop for SpawnCleanupGuard<'_> {
    fn drop(&mut self) {
        lock_unpoisoned(self.spawning).remove(&self.key);
        self.notify.notify_waiters();
    }
}

fn codex_home_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME")
        && !home.trim().is_empty()
    {
        return Some(PathBuf::from(home));
    }
    dirs::home_dir().map(|h| h.join(".ata"))
}

fn managed_lsp_bin_dirs() -> Vec<PathBuf> {
    let Some(codex_home) = codex_home_dir() else {
        return Vec::new();
    };
    let dirs = vec![
        codex_home.join("lsp").join("bin"),
        codex_home.join("lsp").join("npm").join("bin"),
        codex_home.join("lsp").join("pip").join("bin"),
    ];
    #[cfg(windows)]
    {
        let mut dirs = dirs;
        dirs.push(codex_home.join("lsp").join("pip").join("Scripts"));
        dirs
    }
    #[cfg(not(windows))]
    {
        dirs
    }
}

fn program_has_path_separator(program: &str) -> bool {
    program.contains(std::path::MAIN_SEPARATOR) || program.contains('/')
}

fn resolve_program_on_system_or_managed(program: &str) -> Option<PathBuf> {
    if program_has_path_separator(program) {
        let path = PathBuf::from(program);
        return path.exists().then_some(path);
    }

    if let Ok(path) = which::which(program) {
        return Some(path);
    }

    #[cfg(not(windows))]
    let names = vec![program.to_string()];
    #[cfg(windows)]
    let mut names = vec![program.to_string()];
    #[cfg(windows)]
    {
        if !program.ends_with(".exe") {
            names.push(format!("{program}.exe"));
        }
        if !program.ends_with(".cmd") {
            names.push(format!("{program}.cmd"));
        }
        if !program.ends_with(".bat") {
            names.push(format!("{program}.bat"));
        }
    }

    for dir in managed_lsp_bin_dirs() {
        for name in &names {
            let path = dir.join(name);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn resolve_command_variant(variant: &[String]) -> Option<Vec<String>> {
    let first = variant.first()?;
    let mut out = variant.to_vec();
    let resolved = resolve_program_on_system_or_managed(first)?;
    out[0] = resolved.to_string_lossy().to_string();
    Some(out)
}

/// Callback type for handling auto-install.
///
/// The callback is expected to *perform the install* (including any user
/// prompting / approvals) rather than only confirming. This allows the host
/// (codex-core) to route installs through unified_exec so it can request
/// escalated sandbox permissions when needed (e.g. writes to ~/.rustup).
pub type InstallRunnerFn = Arc<
    dyn Fn(&str, &[String]) -> std::pin::Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync,
>;

/// Manages a pool of LSP clients.
pub struct ServerRegistry {
    /// Server configurations keyed by server_id.
    servers: HashMap<String, LspServerConfig>,
    /// Active clients keyed by (server_id, root).
    clients: AsyncMutex<HashMap<ClientKey, Arc<LspClient>>>,
    /// Servers that failed to start, with reason. Entries can be cleared to allow retry
    /// (e.g. after the agent installs missing dependencies).
    broken: AsyncMutex<HashMap<ClientKey, String>>,
    /// In-flight spawns for dedup.
    spawning: StdMutex<HashMap<ClientKey, Arc<tokio::sync::Notify>>>,
    /// Workspace root directory.
    workspace_root: PathBuf,
    /// Optional install confirmation callback.
    install_confirm: RwLock<Option<InstallRunnerFn>>,
}

impl std::fmt::Debug for ServerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerRegistry")
            .field("servers", &self.servers.keys().collect::<Vec<_>>())
            .field("workspace_root", &self.workspace_root)
            .finish_non_exhaustive()
    }
}

impl ServerRegistry {
    /// Create a new registry with the given server configurations.
    pub fn new(
        servers: HashMap<String, LspServerConfig>,
        workspace_root: PathBuf,
        install_confirm: Option<InstallRunnerFn>,
    ) -> Self {
        Self {
            servers,
            clients: AsyncMutex::new(HashMap::new()),
            broken: AsyncMutex::new(HashMap::new()),
            spawning: StdMutex::new(HashMap::new()),
            workspace_root,
            install_confirm: RwLock::new(install_confirm),
        }
    }

    /// Set or clear the install confirmation callback used for auto-install.
    pub fn set_install_confirm(&self, callback: Option<InstallRunnerFn>) {
        if let Ok(mut guard) = self.install_confirm.write() {
            *guard = callback;
        }
    }

    /// Clear broken entries for servers matching `path`, allowing retry.
    pub async fn clear_broken_for_path(&self, path: &Path) {
        let matching: Vec<String> = self
            .servers
            .iter()
            .filter(|(_, c)| !c.disabled && c.matches_path(path))
            .map(|(id, _)| id.clone())
            .collect();
        self.broken
            .lock()
            .await
            .retain(|(sid, _), _| !matching.contains(sid));
    }

    /// Clear all broken entries (for workspace-wide operations).
    pub async fn clear_all_broken(&self) {
        self.broken.lock().await.clear();
    }

    /// Returns the workspace root directory for this registry.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Returns true when at least one configured server can handle this file.
    pub fn has_servers_for(&self, file: &Path) -> bool {
        self.servers
            .values()
            .any(|config| !config.disabled && config.matches_path(file))
    }

    /// Number of currently running LSP clients in this registry.
    pub async fn running_client_count(&self) -> usize {
        self.clients.lock().await.len()
    }

    fn resolve_start_command(&self, config: &LspServerConfig) -> Option<Vec<String>> {
        for variant in config.command_variants() {
            if let Some(resolved) = resolve_command_variant(variant) {
                return Some(resolved);
            }
        }
        None
    }

    fn binary_available(&self, binary: &str) -> bool {
        resolve_program_on_system_or_managed(binary).is_some()
    }

    /// Best-effort explanation for why servers are currently unavailable.
    pub async fn explain_unavailable_servers(&self, file: &Path) -> Vec<String> {
        let mut lines = Vec::new();
        for (server_id, config) in &self.servers {
            if config.disabled || !config.matches_path(file) {
                continue;
            }

            let root = nearest_root(file, &self.workspace_root, &config.root_markers);
            let root = refine_root(&root, &self.workspace_root, &config.root_strategy);
            let key: ClientKey = (server_id.clone(), root);
            if let Some(reason) = self.broken.lock().await.get(&key).cloned() {
                lines.push(format!(
                    "{server_id}: previous startup failure in this session: {reason}"
                ));
                continue;
            }

            if self.resolve_start_command(config).is_some() {
                continue;
            }

            if let Some(binary) = config.binary_name() {
                if let Some(install) = &config.install {
                    lines.push(format!(
                        "{server_id}: `{binary}` not found on PATH or managed bins; auto-install is configured via {}",
                        install.method.label()
                    ));
                } else {
                    lines.push(format!(
                        "{server_id}: `{binary}` not found on PATH or managed bins and no auto-install is configured"
                    ));
                }
            } else {
                lines.push(format!("{server_id}: empty command configuration"));
            }
        }

        lines
    }

    /// Get or spawn all applicable clients for a given file.
    /// Returns a list of (server_id, client) pairs.
    pub async fn get_clients(&self, file: &Path) -> Vec<(String, Arc<LspClient>)> {
        let mut result = Vec::new();

        for (server_id, config) in &self.servers {
            if config.disabled || !config.matches_path(file) {
                continue;
            }

            let root = nearest_root(file, &self.workspace_root, &config.root_markers);
            let root = refine_root(&root, &self.workspace_root, &config.root_strategy);
            let key: ClientKey = (server_id.clone(), root.clone());

            // Check if broken.
            if self.broken.lock().await.contains_key(&key) {
                continue;
            }

            // Check if already running.
            {
                let clients = self.clients.lock().await;
                if let Some(client) = clients.get(&key) {
                    result.push((server_id.clone(), client.clone()));
                    continue;
                }
            }

            // Try to spawn (with dedup).
            match self.spawn_client(server_id, config, &root, &key).await {
                Ok(client) => {
                    result.push((server_id.clone(), client));
                }
                Err(e) => {
                    tracing::warn!(
                        server = %server_id,
                        root = %root.display(),
                        "failed to spawn LSP client: {e}"
                    );
                    self.broken.lock().await.insert(key, e.to_string());
                }
            }
        }

        result
    }

    async fn first_match<T, F, Fut>(&self, path: &Path, mut query: F) -> Option<T>
    where
        F: FnMut(Arc<LspClient>) -> Fut,
        Fut: Future<Output = Option<T>>,
    {
        let clients = self.get_clients(path).await;
        for (_, client) in clients {
            if let Some(result) = query(client).await {
                return Some(result);
            }
        }
        None
    }

    async fn client_handles_for_path(&self, path: &Path) -> Vec<Arc<LspClient>> {
        self.get_clients(path)
            .await
            .into_iter()
            .map(|(_, client)| client)
            .collect()
    }

    async fn fan_out_all<T, F, Fut>(
        &self,
        clients: Vec<Arc<LspClient>>,
        query_name: &'static str,
        mut query: F,
    ) -> Vec<T>
    where
        T: Send + 'static,
        F: FnMut(Arc<LspClient>) -> Fut,
        Fut: Future<Output = Vec<T>> + Send + 'static,
    {
        if clients.is_empty() {
            return Vec::new();
        }

        let mut tasks = JoinSet::new();
        for client in clients {
            tasks.spawn(query(client));
        }

        let mut all = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(mut items) => all.append(&mut items),
                Err(e) => tracing::debug!("{query_name} query task failed: {e}"),
            }
        }
        all
    }

    fn dedup_by_key<T, K, F>(items: Vec<T>, mut key_fn: F) -> Vec<T>
    where
        K: Eq + Hash,
        F: FnMut(&T) -> K,
    {
        let mut seen = HashSet::with_capacity(items.len());
        let mut deduped = Vec::with_capacity(items.len());
        for item in items {
            if seen.insert(key_fn(&item)) {
                deduped.push(item);
            }
        }
        deduped
    }

    /// Eagerly start workspace-relevant servers in the background.
    pub async fn prewarm_workspace_clients(&self) {
        self.ensure_workspace_clients_started().await;
    }

    /// Spawn a client with deduplication.
    async fn spawn_client(
        &self,
        server_id: &str,
        config: &LspServerConfig,
        root: &Path,
        key: &ClientKey,
    ) -> Result<Arc<LspClient>, LspError> {
        // Check if someone else is already spawning this.
        let (notify, should_spawn) = {
            let mut spawning = lock_unpoisoned(&self.spawning);
            if let Some(existing) = spawning.get(key) {
                (existing.clone(), false)
            } else {
                let n = Arc::new(tokio::sync::Notify::new());
                spawning.insert(key.clone(), n.clone());
                (n, true)
            }
        };
        if !should_spawn {
            // Wait for the other spawn to finish.
            notify.notified().await;
            // Now check the clients map.
            let clients = self.clients.lock().await;
            if let Some(client) = clients.get(key) {
                return Ok(client.clone());
            }
            return Err(LspError::SpawnFailed("concurrent spawn failed".into()));
        }

        // Ensure `spawning` is always cleared, even if this task is cancelled.
        let _cleanup = SpawnCleanupGuard::new(&self.spawning, key.clone(), notify);

        let result: Result<Arc<LspClient>, LspError> = async {
            let binary = config.binary_name().map(ToOwned::to_owned);

            // Check command availability and auto-install when configured.
            let mut resolved_command = self.resolve_start_command(config);
            if resolved_command.is_none() && config.install.is_some() {
                let install_target = binary.clone().unwrap_or_else(|| server_id.to_string());
                let installed = self
                    .try_auto_install(server_id, config, &install_target)
                    .await;
                if !installed {
                    return Err(LspError::BinaryNotFound(install_target));
                }
                resolved_command = self.resolve_start_command(config);
            }

            let Some(resolved_command) = resolved_command else {
                return Err(LspError::BinaryNotFound(
                    binary.clone().unwrap_or_else(|| server_id.to_string()),
                ));
            };

            // Binary exists on PATH, but may be a shim (e.g. rustup proxy) that fails
            // because the actual component isn't installed.
            if config.install.is_some()
                && let Some(binary_name) = binary.as_deref()
                && which::which(binary_name).is_ok()
                && let PreflightResult::NeedsInstall(reason) = self
                    .preflight_version_check(binary_name, config, root)
                    .await
            {
                tracing::debug!(
                    server = %server_id,
                    binary = %binary_name,
                    "preflight indicates missing install ({reason}); attempting auto-install"
                );
                let installed = self.try_auto_install(server_id, config, binary_name).await;
                if !installed {
                    return Err(LspError::BinaryNotFound(binary_name.to_string()));
                }
            }

            let mut runtime_config = config.clone();
            runtime_config.command = resolved_command;
            apply_post_root_hook(&mut runtime_config, root);
            let create_result = LspClient::create(server_id, &runtime_config, root).await;

            // If the process exited immediately (broken shim), try auto-install and retry.
            match create_result {
                Err(LspError::ProcessExitedImmediately {
                    ref status,
                    ref stderr,
                }) if config.install.is_some() => {
                    tracing::warn!(
                        server = %server_id,
                        %status,
                        "server binary appears to be a broken shim, \
                         attempting auto-install. stderr: {stderr}"
                    );
                    if let Some(recovered) = self
                        .attempt_recovery_install(server_id, config, binary.as_deref(), root)
                        .await
                    {
                        recovered
                    } else {
                        create_result
                    }
                }
                // Some shims don't exit within the 30ms early-death window but still
                // terminate before responding to `initialize`.
                Err(LspError::ServerExited) if config.install.is_some() => {
                    if let Some(binary) = binary.as_deref() {
                        match self.preflight_version_check(binary, config, root).await {
                            PreflightResult::NeedsInstall(reason) => {
                                tracing::warn!(
                                    server = %server_id,
                                    binary = %binary,
                                    "server exited during init; preflight indicates missing install ({reason}); attempting auto-install"
                                );
                                if let Some(recovered) = self
                                    .attempt_recovery_install(
                                        server_id,
                                        config,
                                        Some(binary),
                                        root,
                                    )
                                    .await
                                {
                                    recovered
                                } else {
                                    create_result
                                }
                            }
                            _ => create_result,
                        }
                    } else {
                        create_result
                    }
                }
                other => other,
            }
        }
        .await;

        match result {
            Ok(client) => {
                self.clients
                    .lock()
                    .await
                    .insert(key.clone(), client.clone());
                Ok(client)
            }
            Err(e) => Err(e),
        }
    }

    /// Attempt to auto-install a server binary.
    async fn try_auto_install(
        &self,
        server_id: &str,
        config: &LspServerConfig,
        binary: &str,
    ) -> bool {
        let Some(install_config) = &config.install else {
            return false;
        };

        let cmd = install_config.method.install_command(binary);

        let runner = self.install_confirm.read().ok().and_then(|g| g.clone());
        let Some(runner) = runner else {
            // No runner callback — skip auto-install.
            return false;
        };

        let prompt = format!("Install {server_id} via `{}`?", cmd.join(" "));
        if !runner(&prompt, &cmd).await {
            return false;
        }

        // Verify the binary is now available.
        self.binary_available(binary)
    }

    async fn attempt_recovery_install(
        &self,
        server_id: &str,
        config: &LspServerConfig,
        binary: Option<&str>,
        root: &Path,
    ) -> Option<Result<Arc<LspClient>, LspError>> {
        let binary = binary?;
        if !self.try_auto_install(server_id, config, binary).await {
            return None;
        }
        let re_resolved = self.resolve_start_command(config)?;
        let mut retry_config = config.clone();
        retry_config.command = re_resolved;
        Some(LspClient::create(server_id, &retry_config, root).await)
    }

    async fn preflight_version_check(
        &self,
        binary: &str,
        config: &LspServerConfig,
        root: &Path,
    ) -> PreflightResult {
        // Best-effort: if anything looks odd (spawn fails, timeout, etc.),
        // treat it as inconclusive and proceed with normal startup.
        let mut cmd = tokio::process::Command::new(binary);
        cmd.arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(root)
            .kill_on_drop(true);

        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(_) => return PreflightResult::Inconclusive,
        };

        // Take stdout/stderr handles so we can read them after waiting.
        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();

        let status = match tokio::time::timeout(PREFLIGHT_VERSION_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(_)) => return PreflightResult::Inconclusive,
            Err(_) => {
                let _ = child.kill().await;
                return PreflightResult::Inconclusive;
            }
        };

        let mut out_stdout: Vec<u8> = Vec::new();
        let mut out_stderr: Vec<u8> = Vec::new();
        if let Some(ref mut s) = stdout {
            let _ = s.read_to_end(&mut out_stdout).await;
        }
        if let Some(ref mut s) = stderr {
            let _ = s.read_to_end(&mut out_stderr).await;
        }

        if status.success() {
            return PreflightResult::Installed;
        }

        // Combine stdout/stderr (truncated) and look for strong "missing install" signatures.
        let mut combined: Vec<u8> = Vec::new();
        combined.extend_from_slice(&out_stdout);
        if combined.len() < PREFLIGHT_OUTPUT_MAX_BYTES {
            combined.extend_from_slice(&out_stderr);
        }
        combined.truncate(PREFLIGHT_OUTPUT_MAX_BYTES);

        let combined = String::from_utf8_lossy(&combined);
        if let Some(reason) = output_indicates_missing_install(binary, &combined) {
            return PreflightResult::NeedsInstall(reason);
        }

        PreflightResult::Inconclusive
    }

    // -----------------------------------------------------------------------
    // File operations
    // -----------------------------------------------------------------------

    /// Sync a file to all applicable clients. If `wait` is true, also wait for
    /// diagnostics to settle and return them.
    pub async fn touch_file(&self, path: &Path, wait: bool) -> HashMap<String, Vec<Diagnostic>> {
        let clients = self.get_clients(path).await;
        let mut all_diags = HashMap::new();

        for (server_id, client) in &clients {
            if let Err(e) = client.notify_open(path).await {
                tracing::debug!(server = %server_id, "notify_open failed: {e}");
                continue;
            }
            if wait {
                let diags = client.wait_for_diagnostics(path).await;
                if !diags.is_empty() {
                    all_diags.insert(server_id.clone(), diags);
                }
            }
        }

        all_diags
    }

    /// Get current diagnostics for a file from all applicable clients.
    pub async fn diagnostics_for_file(&self, path: &Path) -> Vec<Diagnostic> {
        let clients = self.get_clients(path).await;
        let mut all = Vec::new();
        for (_, client) in clients {
            all.extend(client.diagnostics_for(path).await);
        }
        all
    }

    // -----------------------------------------------------------------------
    // Query dispatch: fan out to all applicable clients
    // -----------------------------------------------------------------------

    pub async fn hover(&self, path: &Path, line: u32, character: u32) -> Option<Hover> {
        self.first_match(path, |client| async move {
            client.hover(path, line, character).await
        })
        .await
    }

    pub async fn definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Option<GotoDefinitionResponse> {
        self.first_match(path, |client| async move {
            client.definition(path, line, character).await
        })
        .await
    }

    pub async fn references(&self, path: &Path, line: u32, character: u32) -> Vec<Location> {
        let clients = self.client_handles_for_path(path).await;
        let path = path.to_path_buf();
        let references = self
            .fan_out_all(clients, "references", move |client| {
                let path = path.clone();
                async move { client.references(&path, line, character).await }
            })
            .await;
        Self::dedup_by_key(references, location_key)
    }

    pub async fn document_symbol(&self, path: &Path) -> Option<DocumentSymbolResponse> {
        self.first_match(
            path,
            |client| async move { client.document_symbol(path).await },
        )
        .await
    }

    pub async fn workspace_symbol(&self, query: &str) -> Vec<SymbolInformation> {
        self.ensure_workspace_clients_started().await;
        let clients: Vec<Arc<LspClient>> = {
            let clients_map = self.clients.lock().await;
            clients_map.values().cloned().collect()
        };
        let query = query.to_string();
        let symbols: Vec<SymbolInformation> = self
            .fan_out_all(clients, "workspace_symbol", move |client| {
                let query = query.clone();
                async move { client.workspace_symbol(&query).await }
            })
            .await;

        // Filter out symbols whose location is outside the workspace root.
        // LSP servers (e.g. gopls) may return symbols from stdlib or module
        // caches that live far outside the registered project root.
        // Canonicalize to handle macOS /tmp -> /private/tmp symlinks.
        let canonical_root = std::fs::canonicalize(&self.workspace_root)
            .unwrap_or_else(|_| self.workspace_root.clone());
        let filtered: Vec<SymbolInformation> = symbols
            .into_iter()
            .filter(|sym| {
                path_from_uri(&sym.location.uri)
                    .and_then(|p| std::fs::canonicalize(&p).ok().or(Some(p)))
                    .map(|p| p.starts_with(&canonical_root))
                    .unwrap_or(false)
            })
            .collect();
        Self::dedup_by_key(filtered, symbol_info_key)
    }

    async fn ensure_workspace_clients_started(&self) {
        // Collect non-disabled servers.
        let all_servers: Vec<(String, LspServerConfig)> = self
            .servers
            .iter()
            .filter(|(_, config)| !config.disabled)
            .map(|(id, config)| (id.clone(), config.clone()))
            .collect();

        // Skip servers that already have a running client (from warmup or prior
        // file-based operations) — workspace_symbol fans out to ALL clients.
        let running_server_ids: std::collections::HashSet<String> = {
            let clients = self.clients.lock().await;
            clients.keys().map(|(sid, _)| sid.clone()).collect()
        };

        for (server_id, config) in &all_servers {
            if running_server_ids.contains(server_id) {
                continue;
            }

            // Try workspace_root first, then walk subdirectories for root markers.
            let root = if dir_has_any_marker(&self.workspace_root, &config.root_markers) {
                self.workspace_root.clone()
            } else if let Some(sub_root) =
                Self::find_sub_root(&self.workspace_root, &config.root_markers, 3)
            {
                sub_root
            } else {
                continue;
            };

            let key: ClientKey = (server_id.clone(), root.clone());

            if self.broken.lock().await.contains_key(&key) {
                continue;
            }

            {
                let clients = self.clients.lock().await;
                if clients.contains_key(&key) {
                    continue;
                }
            }

            match self.spawn_client(server_id, config, &root, &key).await {
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(
                        server = %server_id,
                        root = %root.display(),
                        "failed to spawn LSP client for workspace_symbol: {e}"
                    );
                    self.broken.lock().await.insert(key, e.to_string());
                }
            }
        }
    }

    /// Walk subdirectories of `dir` up to `max_depth` levels looking for the
    /// first directory that contains any of the given root markers.
    fn find_sub_root(dir: &Path, markers: &[String], max_depth: usize) -> Option<PathBuf> {
        Self::walk_for_sub_root(dir, markers, 0, max_depth)
    }

    fn walk_for_sub_root(
        dir: &Path,
        markers: &[String],
        depth: usize,
        max_depth: usize,
    ) -> Option<PathBuf> {
        if depth > max_depth {
            return None;
        }
        let entries = std::fs::read_dir(dir).ok()?;
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            if dir_has_any_marker(&path, markers) {
                return Some(path);
            }
            subdirs.push(path);
        }
        for sub in subdirs {
            if let Some(found) = Self::walk_for_sub_root(&sub, markers, depth + 1, max_depth) {
                return Some(found);
            }
        }
        None
    }

    pub async fn implementation(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Option<GotoImplementationResponse> {
        self.first_match(path, |client| async move {
            client.implementation(path, line, character).await
        })
        .await
    }

    pub async fn prepare_call_hierarchy(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Vec<CallHierarchyItem> {
        let clients = self.get_clients(path).await;
        if clients.is_empty() {
            return Vec::new();
        }
        let path = path.to_path_buf();
        let mut tasks = JoinSet::new();
        for (server_id, client) in clients {
            let path = path.clone();
            tasks.spawn(async move {
                let mut items = client.prepare_call_hierarchy(&path, line, character).await;
                for item in &mut items {
                    attach_call_hierarchy_server_id(item, &server_id);
                }
                items
            });
        }

        let mut items = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(mut batch) => items.append(&mut batch),
                Err(e) => tracing::debug!("prepare_call_hierarchy query task failed: {e}"),
            }
        }
        Self::dedup_by_key(items, call_hierarchy_item_key)
    }

    pub async fn incoming_calls(&self, item: CallHierarchyItem) -> Vec<CallHierarchyIncomingCall> {
        let path = match path_from_uri(&item.uri) {
            Some(p) => p,
            None => return Vec::new(),
        };
        let mut request_item = item.clone();
        let calls = if let Some(server_id) = call_hierarchy_server_id(&item) {
            clear_call_hierarchy_server_id(&mut request_item);
            let client = self.get_clients(&path).await.into_iter().find_map(
                |(candidate_server_id, client)| {
                    (candidate_server_id == server_id).then_some(client)
                },
            );
            match client {
                Some(client) => client.incoming_calls(request_item).await,
                None => Vec::new(),
            }
        } else {
            let clients = self.client_handles_for_path(&path).await;
            self.fan_out_all(clients, "incoming_calls", move |client| {
                let item = request_item.clone();
                async move { client.incoming_calls(item).await }
            })
            .await
        };
        Self::dedup_by_key(calls, incoming_call_key)
    }

    pub async fn outgoing_calls(&self, item: CallHierarchyItem) -> Vec<CallHierarchyOutgoingCall> {
        let path = match path_from_uri(&item.uri) {
            Some(p) => p,
            None => return Vec::new(),
        };
        let mut request_item = item.clone();
        let calls = if let Some(server_id) = call_hierarchy_server_id(&item) {
            clear_call_hierarchy_server_id(&mut request_item);
            let client = self.get_clients(&path).await.into_iter().find_map(
                |(candidate_server_id, client)| {
                    (candidate_server_id == server_id).then_some(client)
                },
            );
            match client {
                Some(client) => client.outgoing_calls(request_item).await,
                None => Vec::new(),
            }
        } else {
            let clients = self.client_handles_for_path(&path).await;
            self.fan_out_all(clients, "outgoing_calls", move |client| {
                let item = request_item.clone();
                async move { client.outgoing_calls(item).await }
            })
            .await
        };
        Self::dedup_by_key(calls, outgoing_call_key)
    }

    pub async fn prepare_rename(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Option<PrepareRenameResponse> {
        self.first_match(path, |client| async move {
            client.prepare_rename(path, line, character).await
        })
        .await
    }

    pub async fn rename(
        &self,
        path: &Path,
        line: u32,
        character: u32,
        new_name: &str,
    ) -> Option<WorkspaceEdit> {
        let new_name = new_name.to_string();
        let path = path.to_path_buf();
        self.first_match(&path, |client| {
            let path = path.clone();
            let new_name = new_name.clone();
            async move { client.rename(&path, line, character, &new_name).await }
        })
        .await
    }

    pub async fn code_action(
        &self,
        path: &Path,
        range: Range,
        only: Option<Vec<CodeActionKind>>,
        diagnostics: Vec<Diagnostic>,
    ) -> Vec<CodeActionOrCommand> {
        let clients = self.client_handles_for_path(path).await;

        if clients.is_empty() {
            return Vec::new();
        }
        let path = path.to_path_buf();
        self.fan_out_all(clients, "code_action", move |client| {
            let path = path.clone();
            let range = range;
            let only = only.clone();
            let diagnostics = diagnostics.clone();
            async move { client.code_action(&path, range, only, diagnostics).await }
        })
        .await
    }

    /// Resolve a code action (populate its `edit` field) via `codeAction/resolve`.
    ///
    /// `path` is used to route to the correct language server.
    pub async fn code_action_resolve(
        &self,
        path: &Path,
        action: CodeAction,
    ) -> Option<CodeAction> {
        self.first_match(path, |client| {
            let action = action.clone();
            async move { client.code_action_resolve(action).await }
        })
        .await
    }

    // -----------------------------------------------------------------------
    // Shutdown
    // -----------------------------------------------------------------------

    /// Gracefully shut down all running clients.
    pub async fn shutdown_all(&self) {
        let clients: Vec<Arc<LspClient>> = {
            let mut map = self.clients.lock().await;
            map.drain().map(|(_, c)| c).collect()
        };
        for client in clients {
            client.shutdown().await;
        }
    }
}

use crate::server_config::PostRootHook;

/// Apply a post-root hook to the runtime configuration.
fn apply_post_root_hook(config: &mut LspServerConfig, root: &Path) {
    match &config.post_root_hook {
        PostRootHook::None => {}
        PostRootHook::PythonVenvProbe => {
            if let Some(python_path) = probe_python_venv(root) {
                let opts = config
                    .initialization_options
                    .get_or_insert_with(|| serde_json::json!({}));
                if let Some(obj) = opts.as_object_mut() {
                    obj.entry("python").or_insert_with(|| serde_json::json!({}));
                    if let Some(python_obj) = obj.get_mut("python").and_then(|v| v.as_object_mut())
                    {
                        python_obj.entry("pythonPath".to_string()).or_insert(
                            serde_json::Value::String(python_path.to_string_lossy().to_string()),
                        );
                    }
                }
            }
        }
    }
}

/// Probe for a Python virtual environment at or above `root`.
///
/// Checks in order: `$VIRTUAL_ENV`, `<root>/.venv`, `<root>/venv`.
fn probe_python_venv(root: &Path) -> Option<PathBuf> {
    // 1. $VIRTUAL_ENV environment variable
    if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
        let venv = PathBuf::from(venv);
        if let Some(python) = find_python_in_venv(&venv) {
            return Some(python);
        }
    }
    // 2. <root>/.venv
    let dot_venv = root.join(".venv");
    if let Some(python) = find_python_in_venv(&dot_venv) {
        return Some(python);
    }
    // 3. <root>/venv
    let venv = root.join("venv");
    find_python_in_venv(&venv)
}

/// Find a Python binary inside a virtual environment directory.
fn find_python_in_venv(venv: &Path) -> Option<PathBuf> {
    #[cfg(unix)]
    let candidates = &[venv.join("bin/python3"), venv.join("bin/python")];
    #[cfg(windows)]
    let candidates = &[venv.join("Scripts/python.exe")];
    candidates.iter().find(|p| p.exists()).cloned()
}

fn output_indicates_missing_install(binary: &str, combined_output: &str) -> Option<&'static str> {
    // Keep this intentionally strict. We only want to auto-install when we're
    // very confident the "binary exists" but is unusable due to missing install.
    //
    // Primary target: rustup proxy for rust-analyzer.
    let bin_lc = binary.to_ascii_lowercase();
    let out_lc = combined_output.to_ascii_lowercase();
    let mentions_binary = output_mentions_binary(&bin_lc, &out_lc);

    // rustup shim patterns
    if out_lc.contains("unknown binary") && mentions_binary {
        return Some("rustup shim: unknown binary/component");
    }
    if out_lc.contains("not installed for the toolchain") && mentions_binary {
        return Some("rustup shim: component not installed");
    }
    if out_lc.contains("rustup component add") && mentions_binary {
        return Some("rustup shim: suggests rustup component add");
    }

    // node/npm global module missing patterns (best-effort)
    if out_lc.contains("cannot find module") && mentions_binary {
        return Some("node shim: cannot find module");
    }

    // python/pip missing module patterns (best-effort)
    if out_lc.contains("modulenotfounderror") && mentions_binary {
        return Some("python shim: module not found");
    }
    if out_lc.contains("no module named") && mentions_binary {
        return Some("python shim: module not found");
    }

    None
}

fn output_mentions_binary(binary_lowercase: &str, output_lowercase: &str) -> bool {
    let pattern = format!(r"\b{}\b", regex::escape(binary_lowercase));
    regex::Regex::new(&pattern)
        .map(|regex| regex.is_match(output_lowercase))
        .unwrap_or_else(|_| output_lowercase.contains(binary_lowercase))
}

fn attach_call_hierarchy_server_id(item: &mut CallHierarchyItem, server_id: &str) {
    let mut data = match item.data.take() {
        Some(Value::Object(map)) => map,
        Some(other) => {
            let mut map = serde_json::Map::new();
            map.insert("value".to_string(), other);
            map
        }
        None => serde_json::Map::new(),
    };
    data.insert(
        CALL_HIERARCHY_SERVER_ID_KEY.to_string(),
        Value::String(server_id.to_string()),
    );
    item.data = Some(Value::Object(data));
}

fn call_hierarchy_server_id(item: &CallHierarchyItem) -> Option<&str> {
    item.data
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|map| map.get(CALL_HIERARCHY_SERVER_ID_KEY))
        .and_then(Value::as_str)
}

fn clear_call_hierarchy_server_id(item: &mut CallHierarchyItem) {
    let Some(Value::Object(map)) = item.data.as_mut() else {
        return;
    };
    map.remove(CALL_HIERARCHY_SERVER_ID_KEY);
    if map.is_empty() {
        item.data = None;
    }
}

fn range_key(range: &Range) -> (u32, u32, u32, u32) {
    (
        range.start.line,
        range.start.character,
        range.end.line,
        range.end.character,
    )
}

fn location_key(location: &Location) -> (String, (u32, u32, u32, u32)) {
    (
        location.uri.as_str().to_string(),
        range_key(&location.range),
    )
}

fn symbol_info_key(sym: &SymbolInformation) -> (String, (u32, u32, u32, u32)) {
    (
        sym.location.uri.as_str().to_string(),
        range_key(&sym.location.range),
    )
}

fn call_hierarchy_item_key(item: &CallHierarchyItem) -> (String, (u32, u32, u32, u32)) {
    (item.uri.as_str().to_string(), range_key(&item.range))
}

fn incoming_call_key(call: &CallHierarchyIncomingCall) -> (String, (u32, u32, u32, u32)) {
    (
        call.from.uri.as_str().to_string(),
        range_key(&call.from.range),
    )
}

fn outgoing_call_key(call: &CallHierarchyOutgoingCall) -> (String, (u32, u32, u32, u32)) {
    (call.to.uri.as_str().to_string(), range_key(&call.to.range))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    static PATH_MUTEX: Mutex<()> = Mutex::new(());

    fn rust_analyzer_like_rustup_error() -> &'static str {
        "error: Unknown binary 'rust-analyzer' in official toolchain '1.93.0-aarch64-apple-darwin'.\n\
info: 'rust-analyzer' is not installed for the toolchain '1.93.0-aarch64-apple-darwin'.\n\
help: run `rustup component add rust-analyzer`\n"
    }

    #[test]
    fn detects_rustup_missing_component() {
        let out = rust_analyzer_like_rustup_error();
        assert!(output_indicates_missing_install("rust-analyzer", out).is_some());
    }

    #[test]
    fn does_not_false_positive_on_substring_binary_names() {
        let out = "error: Unknown binary 'false' in toolchain";
        assert!(output_indicates_missing_install("ls", out).is_none());
    }

    #[test]
    fn call_hierarchy_server_id_metadata_roundtrip() {
        let uri: Uri = "file:///tmp/sample.rs".parse().expect("valid uri");
        let mut item = CallHierarchyItem {
            name: "sample".to_string(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            detail: None,
            uri,
            range: Range {
                start: Position {
                    line: 1,
                    character: 0,
                },
                end: Position {
                    line: 1,
                    character: 6,
                },
            },
            selection_range: Range {
                start: Position {
                    line: 1,
                    character: 0,
                },
                end: Position {
                    line: 1,
                    character: 6,
                },
            },
            data: Some(serde_json::json!({"foo": "bar"})),
        };

        attach_call_hierarchy_server_id(&mut item, "rust-analyzer");
        assert_eq!(
            call_hierarchy_server_id(&item),
            Some("rust-analyzer"),
            "server id should be persisted in item.data"
        );

        clear_call_hierarchy_server_id(&mut item);
        assert_eq!(call_hierarchy_server_id(&item), None);
        assert_eq!(item.data, Some(serde_json::json!({"foo": "bar"})));
    }

    #[tokio::test]
    async fn spawn_cleanup_guard_clears_entry_and_notifies_waiters() {
        let spawning: StdMutex<HashMap<ClientKey, Arc<tokio::sync::Notify>>> =
            StdMutex::new(HashMap::new());
        let key: ClientKey = ("test-server".to_string(), PathBuf::from("/tmp/workspace"));
        let notify = Arc::new(tokio::sync::Notify::new());
        lock_unpoisoned(&spawning).insert(key.clone(), notify.clone());

        let waiter = notify.notified();
        {
            let _guard = SpawnCleanupGuard::new(&spawning, key.clone(), notify.clone());
        }

        assert!(!lock_unpoisoned(&spawning).contains_key(&key));
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
                .await
                .is_ok(),
            "waiters should be notified when guard drops"
        );
    }

    #[cfg(unix)]
    fn write_executable(dir: &tempfile::TempDir, name: &str, contents: &str) -> std::path::PathBuf {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        let mut perm = f.metadata().unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm).unwrap();
        path
    }

    struct PathGuard {
        old: String,
    }

    impl PathGuard {
        fn prepend(dir: &std::path::Path) -> Self {
            let old = std::env::var("PATH").unwrap_or_default();
            let new = format!("{}:{}", dir.display(), old);
            unsafe {
                std::env::set_var("PATH", new);
            }
            Self { old }
        }
    }

    impl Drop for PathGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::set_var("PATH", &self.old);
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // Intentional: serializes tests that mutate PATH
    async fn spawn_client_prompts_install_on_preflight_needs_install_and_decline() {
        let _path_lock = PATH_MUTEX.lock().unwrap();
        let temp_bin = tempfile::TempDir::new().unwrap();
        let _guard = PathGuard::prepend(temp_bin.path());

        // Fake "rust-analyzer" that looks like a rustup proxy missing the component.
        let script = format!(
            "#!/bin/sh\n\
if [ \"$1\" = \"--version\" ]; then\n\
  cat <<'EOF'\n\
{out}\
EOF\n\
  exit 1\n\
fi\n\
sleep 60\n",
            out = rust_analyzer_like_rustup_error()
        );
        let _path = write_executable(&temp_bin, "rust-analyzer", &script);

        let root = tempfile::TempDir::new().unwrap();
        let root_path = root.path().to_path_buf();

        let server_id = "rust-analyzer";
        let config = LspServerConfig {
            extensions: vec![".rs".into()],
            command: vec!["rust-analyzer".into()],
            command_candidates: Vec::new(),
            env: HashMap::new(),
            root_markers: Vec::new(),
            initialization_options: None,
            disabled: false,
            install: Some(crate::server_config::InstallConfig {
                method: crate::server_config::InstallMethod::Cargo {
                    package: Some("rust-analyzer".into()),
                },
            }),
            root_strategy: Default::default(),
            post_root_hook: Default::default(),
        };

        let called = Arc::new(AtomicUsize::new(0));
        let called2 = called.clone();
        let runner: InstallRunnerFn = Arc::new(move |_prompt, _cmd| {
            called2.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { false })
        });

        let mut servers = HashMap::new();
        servers.insert(server_id.to_string(), config.clone());
        let registry = ServerRegistry::new(servers, root_path.clone(), Some(runner));

        let key: ClientKey = (server_id.to_string(), root_path.clone());
        let res = registry
            .spawn_client(server_id, &config, &root_path, &key)
            .await;

        assert!(matches!(res, Err(LspError::BinaryNotFound(_))));
        assert_eq!(called.load(Ordering::SeqCst), 1);
    }

    #[cfg(unix)]
    #[test]
    fn probe_python_venv_finds_dot_venv_python3() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let bin_dir = root.join(".venv/bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        // Only python3, no python
        let python3 = bin_dir.join("python3");
        std::fs::write(&python3, "").unwrap();

        let result = probe_python_venv(root);
        assert_eq!(result, Some(python3));
    }

    #[cfg(unix)]
    #[test]
    fn probe_python_venv_no_venv_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let result = probe_python_venv(tmp.path());
        assert!(result.is_none());
    }

    #[test]
    fn apply_post_root_hook_preserves_existing_init_options() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        // No venv exists, so nothing should be injected.
        let mut config = LspServerConfig::new(vec![".py".into()], vec!["pyright".into()], vec![]);
        config.initialization_options = Some(serde_json::json!({"diagnostics": true}));
        config.post_root_hook = crate::server_config::PostRootHook::PythonVenvProbe;

        apply_post_root_hook(&mut config, root);
        // Original options preserved.
        assert_eq!(
            config.initialization_options.as_ref().unwrap()["diagnostics"],
            serde_json::json!(true)
        );
    }

    #[cfg(unix)]
    #[test]
    fn apply_post_root_hook_injects_python_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let bin_dir = root.join(".venv/bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let python3 = bin_dir.join("python3");
        std::fs::write(&python3, "").unwrap();

        let mut config = LspServerConfig::new(vec![".py".into()], vec!["pyright".into()], vec![]);
        config.post_root_hook = crate::server_config::PostRootHook::PythonVenvProbe;

        apply_post_root_hook(&mut config, root);
        let opts = config.initialization_options.unwrap();
        let python_path = opts["python"]["pythonPath"].as_str().unwrap();
        assert!(python_path.contains(".venv/bin/python3"));
    }
}
