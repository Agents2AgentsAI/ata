//! Registry that manages a pool of LSP clients, one per (server_id, root) pair.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::future::Future;
use std::hash::Hash;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::RwLock;
use std::time::Duration;
use std::time::Instant;

use fd_lock::RwLock as FileRwLock;
use lsp_types::request::GotoImplementationResponse;
use lsp_types::*;
use serde_json::Value;
use sha1::Digest;
use sha1::Sha1;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinSet;
use tokio::time::sleep;
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
const INSTALL_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(50);
const INSTALL_LOCK_TIMEOUT: Duration = Duration::from_secs(600);
const CODE_ACTION_SERVER_ID_KEY: &str = "__codex_code_action_server_id";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegistryLimits {
    pub max_sub_roots_per_server: usize,
    pub max_lsp_clients_per_registry: usize,
}

impl Default for RegistryLimits {
    fn default() -> Self {
        Self {
            max_sub_roots_per_server: 5,
            max_lsp_clients_per_registry: 20,
        }
    }
}

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

fn managed_lsp_root() -> Option<PathBuf> {
    codex_home_dir().map(|codex_home| codex_home.join("lsp"))
}

fn managed_lsp_bin_dirs() -> Vec<PathBuf> {
    let Some(lsp_root) = managed_lsp_root() else {
        return Vec::new();
    };
    let dirs = vec![
        lsp_root.join("bin"),
        lsp_root.join("gem").join("bin"),
        lsp_root.join("npm").join("bin"),
        lsp_root.join("pip").join("bin"),
    ];
    #[cfg(windows)]
    {
        let mut dirs = dirs;
        dirs.push(lsp_root.join("pip").join("Scripts"));
        dirs
    }
    #[cfg(not(windows))]
    {
        dirs
    }
}

fn install_lock_name(server_id: &str) -> String {
    let mut sanitized = String::with_capacity(server_id.len());
    for ch in server_id.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.is_empty() {
        "server".to_string()
    } else {
        sanitized
    }
}

fn open_install_lock(
    server_id: &str,
) -> std::io::Result<Option<(PathBuf, FileRwLock<std::fs::File>)>> {
    let Some(lsp_root) = managed_lsp_root() else {
        return Ok(None);
    };
    let lock_path = lsp_root
        .join(".install-locks")
        .join(format!("{}.lock", install_lock_name(server_id)));
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    Ok(Some((lock_path, FileRwLock::new(lock_file))))
}

fn sha1_short(path: &Path) -> String {
    let mut hasher = Sha1::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    format!("{digest:x}")[..12].to_string()
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

fn install_skips_preflight(config: &LspServerConfig) -> bool {
    config
        .install
        .as_ref()
        .is_some_and(|install| install.skip_preflight)
}

fn build_runtime_config(
    config: &LspServerConfig,
    resolved_command: Vec<String>,
    root: &Path,
) -> LspServerConfig {
    let mut runtime_config = config.clone();
    runtime_config.command = resolved_command;
    apply_post_root_hook(&mut runtime_config, root);
    runtime_config
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

/// Deduplicates install attempts across multiple [`ServerRegistry`] instances.
///
/// When multiple workspace roots each have their own `ServerRegistry`, they may
/// independently detect that the same global tool (e.g. `rust-analyzer` via
/// `rustup component add`) needs installing. Without coordination, each registry
/// prompts the user and runs the install command separately.
///
/// `InstallTracker` is shared (via `Arc`) across all registries and ensures that
/// only one install attempt per `server_id` is in flight at a time. Subsequent
/// requests for the same server wait for the first attempt to complete and reuse
/// its result.
pub struct InstallTracker {
    /// In-flight install attempts keyed by server_id.
    in_flight: StdMutex<HashMap<String, Arc<tokio::sync::Notify>>>,
    /// Server IDs that were successfully installed this session.
    installed: StdMutex<HashSet<String>>,
}

impl InstallTracker {
    pub fn new() -> Self {
        Self {
            in_flight: StdMutex::new(HashMap::new()),
            installed: StdMutex::new(HashSet::new()),
        }
    }

    /// Check whether `server_id` was already installed this session.
    fn is_installed(&self, server_id: &str) -> bool {
        lock_unpoisoned(&self.installed).contains(server_id)
    }

    /// Try to claim the install slot for `server_id`.
    ///
    /// Returns `Ok(notify)` if this caller should perform the install (and must
    /// call [`finish_install`] afterwards). Returns `Err(notify)` if another
    /// caller is already installing — the caller should `notified().await` and
    /// then check [`is_installed`].
    fn begin_install(
        &self,
        server_id: &str,
    ) -> Result<Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>> {
        let mut in_flight = lock_unpoisoned(&self.in_flight);
        if let Some(existing) = in_flight.get(server_id) {
            Err(existing.clone())
        } else {
            let notify = Arc::new(tokio::sync::Notify::new());
            in_flight.insert(server_id.to_string(), notify.clone());
            Ok(notify)
        }
    }

    /// Mark an install as finished and wake any waiters.
    fn finish_install(&self, server_id: &str, success: bool) {
        if success {
            lock_unpoisoned(&self.installed).insert(server_id.to_string());
        }
        if let Some(notify) = lock_unpoisoned(&self.in_flight).remove(server_id) {
            notify.notify_waiters();
        }
    }
}

impl Default for InstallTracker {
    fn default() -> Self {
        Self::new()
    }
}

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
    /// Shared tracker to deduplicate installs across registries.
    install_tracker: Option<Arc<InstallTracker>>,
    /// Configurable caps for client fan-out and sub-root discovery.
    limits: RegistryLimits,
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
        Self::new_with_limits(
            servers,
            workspace_root,
            install_confirm,
            RegistryLimits::default(),
        )
    }

    /// Create a new registry with explicit runtime limits.
    pub fn new_with_limits(
        servers: HashMap<String, LspServerConfig>,
        workspace_root: PathBuf,
        install_confirm: Option<InstallRunnerFn>,
        limits: RegistryLimits,
    ) -> Self {
        Self::with_install_tracker_and_limits(
            servers,
            workspace_root,
            install_confirm,
            None,
            limits,
        )
    }

    /// Create a new registry with a shared [`InstallTracker`] for cross-registry
    /// install deduplication.
    pub fn with_install_tracker(
        servers: HashMap<String, LspServerConfig>,
        workspace_root: PathBuf,
        install_confirm: Option<InstallRunnerFn>,
        install_tracker: Option<Arc<InstallTracker>>,
    ) -> Self {
        Self::with_install_tracker_and_limits(
            servers,
            workspace_root,
            install_confirm,
            install_tracker,
            RegistryLimits::default(),
        )
    }

    /// Create a new registry with a shared [`InstallTracker`] and explicit limits.
    pub fn with_install_tracker_and_limits(
        servers: HashMap<String, LspServerConfig>,
        workspace_root: PathBuf,
        install_confirm: Option<InstallRunnerFn>,
        install_tracker: Option<Arc<InstallTracker>>,
        limits: RegistryLimits,
    ) -> Self {
        Self {
            servers,
            clients: AsyncMutex::new(HashMap::new()),
            broken: AsyncMutex::new(HashMap::new()),
            spawning: StdMutex::new(HashMap::new()),
            workspace_root,
            install_confirm: RwLock::new(install_confirm),
            install_tracker,
            limits,
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

    /// Whether a matching client for `file` is already running.
    pub async fn has_running_client_for(&self, file: &Path) -> bool {
        let clients = self.clients.lock().await;
        self.servers.iter().any(|(server_id, config)| {
            if config.disabled || !config.matches_path(file) {
                return false;
            }
            let root = nearest_root(file, &self.workspace_root, &config.root_markers);
            let root = refine_root(&root, &self.workspace_root, &config.root_strategy);
            clients.contains_key(&(server_id.clone(), root))
        })
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
        let mut spawn_candidates = Vec::new();

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

            spawn_candidates.push((server_id.clone(), config, root, key));
        }

        let running_clients = self.clients.lock().await.len();
        if running_clients >= self.limits.max_lsp_clients_per_registry {
            tracing::warn!(
                limit = self.limits.max_lsp_clients_per_registry,
                running = running_clients,
                "LSP client cap reached during on-demand spawn"
            );
            return result;
        }

        let remaining_capacity = self.limits.max_lsp_clients_per_registry - running_clients;
        let skipped_spawns = spawn_candidates.len().saturating_sub(remaining_capacity);
        for (server_id, config, root, key) in spawn_candidates.into_iter().take(remaining_capacity)
        {
            // Try to spawn (with dedup).
            match self.spawn_client(&server_id, config, &root, &key).await {
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

        if skipped_spawns > 0 {
            tracing::warn!(
                limit = self.limits.max_lsp_clients_per_registry,
                skipped = skipped_spawns,
                "LSP client cap prevented some on-demand spawns"
            );
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
            // Skip the preflight for servers that opt out (e.g. JVM-based servers
            // where --version starts a heavy runtime and gets killed by the timeout).
            let skip_preflight = install_skips_preflight(config);
            if !skip_preflight
                && config.install.is_some()
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

            let runtime_config = build_runtime_config(config, resolved_command, root);
            let create_result = LspClient::create(server_id, &runtime_config, root).await;

            let recovery_reason = if config.install.is_some() {
                if let Err(error) = &create_result {
                    self.recovery_install_reason(error, binary.as_deref(), config, root)
                        .await
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(reason) = recovery_reason {
                if let Some(binary_name) = binary.as_deref() {
                    match &create_result {
                        Err(LspError::ProcessExitedImmediately { status, stderr }) => {
                            tracing::warn!(
                                server = %server_id,
                                binary = %binary_name,
                                %status,
                                "server exited immediately; recovery indicates missing install ({reason}); attempting auto-install. stderr: {stderr}"
                            );
                        }
                        Err(LspError::ServerExited { details }) => {
                            tracing::warn!(
                                server = %server_id,
                                binary = %binary_name,
                                "server exited during init; recovery indicates missing install ({reason}); attempting auto-install. details: {details}"
                            );
                        }
                        _ => {}
                    }
                    if let Some(recovered) = self
                        .attempt_recovery_install(server_id, config, Some(binary_name), root)
                        .await
                    {
                        recovered
                    } else {
                        create_result
                    }
                } else {
                    create_result
                }
            } else {
                create_result
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
    ///
    /// When an [`InstallTracker`] is present, this coordinates with other
    /// registries so only one install per `server_id` is attempted at a time.
    async fn try_auto_install(
        &self,
        server_id: &str,
        config: &LspServerConfig,
        binary: &str,
    ) -> bool {
        let Some(install_config) = &config.install else {
            return false;
        };

        // Fast path: another registry already installed this server.
        if let Some(tracker) = &self.install_tracker {
            if tracker.is_installed(server_id) {
                return self.binary_available(binary);
            }

            // Coordinate with other registries: only one installs at a time.
            match tracker.begin_install(server_id) {
                Ok(notify) => {
                    // We own the install slot — run the actual install below.
                    let result = self.run_install(server_id, install_config, binary).await;
                    tracker.finish_install(server_id, result);
                    // Notify is dropped via finish_install; also drop our Arc.
                    drop(notify);
                    return result;
                }
                Err(wait) => {
                    // Another registry is installing — wait for it.
                    wait.notified().await;
                    return self.binary_available(binary);
                }
            }
        }

        // No tracker (standalone registry) — install directly.
        self.run_install(server_id, install_config, binary).await
    }

    /// Actually perform the install via the runner callback.
    async fn run_install(
        &self,
        server_id: &str,
        install_config: &crate::server_config::InstallConfig,
        binary: &str,
    ) -> bool {
        let cmd = install_config.method.install_command(binary);

        let runner = self.install_confirm.read().ok().and_then(|g| g.clone());
        let Some(runner) = runner else {
            return false;
        };

        let binary_was_available = self.binary_available(binary);

        let prompt = format!("Install {server_id} via `{}`?", cmd.join(" "));
        let lock = match open_install_lock(server_id) {
            Ok(lock) => lock,
            Err(error) => {
                tracing::warn!(
                    server = %server_id,
                    "failed to prepare auto-install lock: {error}"
                );
                None
            }
        };

        if let Some((lock_path, mut install_lock)) = lock {
            let deadline = Instant::now() + INSTALL_LOCK_TIMEOUT;
            let _install_guard = loop {
                match install_lock.try_write() {
                    Ok(guard) => break Some(guard),
                    Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            tracing::warn!(
                                server = %server_id,
                                lock = %lock_path.display(),
                                timeout_secs = INSTALL_LOCK_TIMEOUT.as_secs(),
                                "timed out waiting for auto-install lock"
                            );
                            return !binary_was_available && self.binary_available(binary);
                        }
                        sleep(INSTALL_LOCK_POLL_INTERVAL).await;
                    }
                    Err(source) => {
                        tracing::warn!(
                            server = %server_id,
                            lock = %lock_path.display(),
                            "failed to acquire auto-install lock: {source}"
                        );
                        break None;
                    }
                }
            };

            if !binary_was_available && self.binary_available(binary) {
                tracing::debug!(
                    server = %server_id,
                    lock = %lock_path.display(),
                    "auto-install skipped because binary became available while waiting"
                );
                return true;
            }

            if !runner(&prompt, &cmd).await {
                return false;
            }

            return self.binary_available(binary);
        }

        if !runner(&prompt, &cmd).await {
            return false;
        }

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
        let retry_config = build_runtime_config(config, re_resolved, root);
        Some(LspClient::create(server_id, &retry_config, root).await)
    }

    async fn recovery_install_reason(
        &self,
        error: &LspError,
        binary: Option<&str>,
        config: &LspServerConfig,
        root: &Path,
    ) -> Option<&'static str> {
        let binary = binary?;
        match error {
            LspError::ProcessExitedImmediately { stderr, .. } => {
                output_indicates_missing_install(binary, stderr)
            }
            LspError::ServerExited { .. } => {
                if install_skips_preflight(config) {
                    None
                } else {
                    match self.preflight_version_check(binary, config, root).await {
                        PreflightResult::NeedsInstall(reason) => Some(reason),
                        _ => None,
                    }
                }
            }
            _ => None,
        }
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
        let clients = self.get_clients(path).await;
        for (_, client) in &clients {
            let _ = client.ensure_open(path).await;
        }
        for (_, client) in clients {
            if let Some(result) = client.document_symbol(path).await {
                return Some(result);
            }
        }
        None
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
        let canonical_root = dunce::canonicalize(&self.workspace_root)
            .unwrap_or_else(|_| self.workspace_root.clone());
        let filtered: Vec<SymbolInformation> = symbols
            .into_iter()
            .filter(|sym| {
                path_from_uri(&sym.location.uri)
                    .and_then(|p| dunce::canonicalize(&p).ok().or(Some(p)))
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

        for (server_id, config) in &all_servers {
            // Try workspace_root first, then walk subdirectories for root markers.
            let roots: Vec<PathBuf> =
                if dir_has_any_marker(&self.workspace_root, &config.root_markers) {
                    vec![self.workspace_root.clone()]
                } else {
                    let found = self.find_sub_roots(&self.workspace_root, &config.root_markers, 3);
                    if found.is_empty() {
                        continue;
                    }
                    found
                };

            if self.clients.lock().await.len() >= self.limits.max_lsp_clients_per_registry {
                tracing::warn!(
                    limit = self.limits.max_lsp_clients_per_registry,
                    "LSP client cap reached; skipping remaining servers"
                );
                break;
            }

            for root in roots {
                if !config.extensions.is_empty()
                    && !Self::has_matching_files(&root, &config.extensions, 3)
                {
                    continue;
                }

                if self.clients.lock().await.len() >= self.limits.max_lsp_clients_per_registry {
                    tracing::warn!(
                        limit = self.limits.max_lsp_clients_per_registry,
                        "LSP client cap reached while prewarming workspace roots"
                    );
                    break;
                }

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
    }

    /// Walk subdirectories of `dir` up to `max_depth` levels looking for all
    /// directories that contain any of the given root markers.
    fn find_sub_roots(&self, dir: &Path, markers: &[String], max_depth: usize) -> Vec<PathBuf> {
        let mut results = Vec::new();
        Self::walk_for_sub_roots(dir, markers, 0, max_depth, &mut results);
        if results.len() > self.limits.max_sub_roots_per_server {
            tracing::warn!(
                found = results.len(),
                limit = self.limits.max_sub_roots_per_server,
                "LSP sub-root discovery capped"
            );
            results.truncate(self.limits.max_sub_roots_per_server);
        }
        results
    }

    fn has_matching_files(dir: &Path, extensions: &[String], max_depth: usize) -> bool {
        Self::scan_for_extensions(dir, extensions, 0, max_depth)
    }

    fn scan_for_extensions(
        dir: &Path,
        extensions: &[String],
        depth: usize,
        max_depth: usize,
    ) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(file_name) = path.file_name().and_then(|name| name.to_str())
                    && extensions
                        .iter()
                        .any(|extension| !extension.starts_with('.') && extension == file_name)
                {
                    return true;
                }
                if let Some(ext) = path.extension().and_then(|extension| extension.to_str()) {
                    let dotted = format!(".{ext}");
                    if extensions.iter().any(|extension| extension == &dotted) {
                        return true;
                    }
                }
            } else if path.is_dir() && depth < max_depth {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if !name_str.starts_with('.') && name_str != "node_modules" {
                    subdirs.push(path);
                }
            }
        }
        for subdir in subdirs {
            if Self::scan_for_extensions(&subdir, extensions, depth + 1, max_depth) {
                return true;
            }
        }
        false
    }

    fn walk_for_sub_roots(
        dir: &Path,
        markers: &[String],
        depth: usize,
        max_depth: usize,
        results: &mut Vec<PathBuf>,
    ) {
        if depth > max_depth {
            return;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };
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
                results.push(path);
                // Don't recurse into a found root — it is itself a project root.
                continue;
            }
            subdirs.push(path);
        }
        for sub in subdirs {
            Self::walk_for_sub_roots(&sub, markers, depth + 1, max_depth, results);
        }
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
        let clients = self.get_clients(path).await;
        if clients.is_empty() {
            return Vec::new();
        }

        let path = path.to_path_buf();
        let mut tasks = JoinSet::new();
        for (server_id, client) in clients {
            let path = path.clone();
            let only = only.clone();
            let diagnostics = diagnostics.clone();
            tasks.spawn(async move {
                let mut actions = client.code_action(&path, range, only, diagnostics).await;
                for action in &mut actions {
                    attach_code_action_server_id(action, &server_id);
                }
                actions
            });
        }

        let mut all = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(mut items) => all.append(&mut items),
                Err(e) => tracing::debug!("code_action query task failed: {e}"),
            }
        }
        all
    }

    /// Resolve a code action (populate its `edit` field) via `codeAction/resolve`.
    ///
    /// `path` is used to route to the correct language server.
    pub async fn code_action_resolve(&self, path: &Path, action: CodeAction) -> Option<CodeAction> {
        let mut request_action = action.clone();
        if let Some(server_id) = code_action_server_id(&action) {
            clear_code_action_server_id(&mut request_action);
            let client = self.get_clients(path).await.into_iter().find_map(
                |(candidate_server_id, client)| {
                    (candidate_server_id == server_id).then_some(client)
                },
            );
            return match client {
                Some(client) => client.code_action_resolve(request_action).await,
                None => None,
            };
        }

        self.first_match(path, |client| {
            let action = request_action.clone();
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

    /// Emergency: shut down up to `count` clients to relieve FD pressure.
    /// Returns the number of clients actually shut down.
    pub async fn shed_clients(&self, count: usize) -> usize {
        let to_shed: Vec<Arc<LspClient>> = {
            let mut map = self.clients.lock().await;
            let keys: Vec<ClientKey> = map.keys().take(count).cloned().collect();
            let mut removed = Vec::with_capacity(keys.len());
            for key in &keys {
                if let Some(client) = map.remove(key) {
                    removed.push(client);
                }
            }
            removed
        };
        let shed_count = to_shed.len();
        for client in to_shed {
            client.shutdown().await;
        }
        shed_count
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
        PostRootHook::JdtlsDataDir => {
            if !config.command.iter().any(|arg| arg == "-data") {
                let data_dir = codex_home_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("lsp")
                    .join("jdtls-data")
                    .join(sha1_short(root));
                let config_dir = data_dir.join("config");

                let _ = std::fs::create_dir_all(&data_dir);
                let _ = std::fs::create_dir_all(&config_dir);
                config.command.push("-data".into());
                config.command.push(data_dir.to_string_lossy().to_string());
                config.command.push("-configuration".into());
                config
                    .command
                    .push(config_dir.to_string_lossy().to_string());
                config.command.push(format!(
                    "--jvm-arg=-Dosgi.configuration.area={}",
                    config_dir.to_string_lossy()
                ));
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

fn attach_code_action_server_id(action: &mut CodeActionOrCommand, server_id: &str) {
    let CodeActionOrCommand::CodeAction(action) = action else {
        return;
    };
    let mut data = match action.data.take() {
        Some(Value::Object(map)) => map,
        Some(other) => {
            let mut map = serde_json::Map::new();
            map.insert("value".to_string(), other);
            map
        }
        None => serde_json::Map::new(),
    };
    data.insert(
        CODE_ACTION_SERVER_ID_KEY.to_string(),
        Value::String(server_id.to_string()),
    );
    action.data = Some(Value::Object(data));
}

fn code_action_server_id(action: &CodeAction) -> Option<&str> {
    action
        .data
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|map| map.get(CODE_ACTION_SERVER_ID_KEY))
        .and_then(Value::as_str)
}

fn clear_code_action_server_id(action: &mut CodeAction) {
    let Some(Value::Object(map)) = action.data.as_mut() else {
        return;
    };
    map.remove(CODE_ACTION_SERVER_ID_KEY);
    if map.is_empty() {
        action.data = None;
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
    #[cfg(unix)]
    use std::sync::atomic::AtomicUsize;
    #[cfg(unix)]
    use std::sync::atomic::Ordering;

    #[cfg(unix)]
    static PATH_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    static ENV_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct EnvVarGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(value) => unsafe {
                    std::env::set_var(self.key, value);
                },
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }

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
    fn write_executable_at(dir: &Path, name: &str, contents: &str) -> std::path::PathBuf {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        let mut perm = f.metadata().unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm).unwrap();
        path
    }

    #[cfg(unix)]
    fn write_executable(dir: &tempfile::TempDir, name: &str, contents: &str) -> std::path::PathBuf {
        write_executable_at(dir.path(), name, contents)
    }

    #[cfg(unix)]
    struct PathGuard {
        old: String,
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
    impl Drop for PathGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::set_var("PATH", &self.old);
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_client_prompts_install_on_preflight_needs_install_and_decline() {
        let _env_lock = ENV_MUTEX.lock().await;
        let codex_home = tempfile::TempDir::new().unwrap();
        let _codex_home_guard =
            EnvVarGuard::set("CODEX_HOME", codex_home.path().to_string_lossy().as_ref());
        let _path_lock = PATH_MUTEX.lock().await;
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
echo 'unexpected invocation without --version' >&2\nexit 1\n",
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
                skip_preflight: false,
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

    #[test]
    fn apply_post_root_hook_injects_jdtls_data_dir() {
        let _env_lock = ENV_MUTEX.blocking_lock();
        let codex_home = tempfile::TempDir::new().unwrap();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("CODEX_HOME", codex_home.path().to_string_lossy().as_ref());

        let mut config = LspServerConfig::new(vec![".java".into()], vec!["jdtls".into()], vec![]);
        config.post_root_hook = crate::server_config::PostRootHook::JdtlsDataDir;

        apply_post_root_hook(&mut config, root.path());

        let data_flag = config
            .command
            .iter()
            .position(|arg| arg == "-data")
            .unwrap();
        let expected_dir = codex_home
            .path()
            .join("lsp")
            .join("jdtls-data")
            .join(sha1_short(root.path()));
        let expected_config_dir = expected_dir.join("config");
        assert_eq!(
            config.command[data_flag + 1],
            expected_dir.to_string_lossy()
        );
        assert!(config.command.contains(&"-data".to_string()));
        let config_flag = config
            .command
            .iter()
            .position(|arg| arg == "-configuration")
            .unwrap();
        assert_eq!(
            config.command[config_flag + 1],
            expected_config_dir.to_string_lossy()
        );
        let jvm_arg = config
            .command
            .iter()
            .find(|arg| arg.starts_with("--jvm-arg=-Dosgi.configuration.area="));
        assert!(jvm_arg.is_some(), "should inject -Dosgi.configuration.area");
        let config_path = &jvm_arg.unwrap()["--jvm-arg=-Dosgi.configuration.area=".len()..];
        assert!(
            config_path.ends_with("/config") || config_path.ends_with("\\config"),
            "config dir should be under data dir, got {config_path}"
        );
        assert!(
            Path::new(config_path).exists(),
            "config dir should be created"
        );
        assert!(expected_dir.exists());
        assert!(expected_config_dir.exists());
    }

    #[cfg(unix)]
    fn installable_server_config(binary: &str) -> LspServerConfig {
        installable_server_config_with_skip_preflight(binary, false)
    }

    #[cfg(unix)]
    fn installable_server_config_with_skip_preflight(
        binary: &str,
        skip_preflight: bool,
    ) -> LspServerConfig {
        let mut config = LspServerConfig::new(vec![".java".into()], vec![binary.into()], vec![]);
        config.install = Some(crate::server_config::InstallConfig {
            method: crate::server_config::InstallMethod::Brew {
                formula: Some(binary.into()),
            },
            skip_preflight,
        });
        config
    }

    #[cfg(unix)]
    fn node_missing_module_output(binary: &str) -> String {
        format!("Error: Cannot find module '{binary}'\n")
    }

    #[test]
    fn apply_post_root_hook_preserves_existing_jdtls_data_dir() {
        let root = tempfile::TempDir::new().unwrap();

        let mut config = LspServerConfig::new(
            vec![".java".into()],
            vec!["jdtls".into(), "-data".into(), "/custom/data".into()],
            vec![],
        );
        config.post_root_hook = crate::server_config::PostRootHook::JdtlsDataDir;

        apply_post_root_hook(&mut config, root.path());

        assert_eq!(
            config.command,
            vec!["jdtls", "-data", "/custom/data"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recovery_install_reason_skips_preflight_for_skip_preflight_servers() {
        let _path_lock = PATH_MUTEX.lock().await;
        let temp_bin = tempfile::TempDir::new().unwrap();
        let _guard = PathGuard::prepend(temp_bin.path());
        let root = tempfile::TempDir::new().unwrap();
        let marker = root.path().join("skip-preflight-marker");
        let binary = "test-skip-preflight-recovery";
        let script = format!(
            "#!/bin/sh\n\
if [ \"$1\" = \"--version\" ]; then\n\
  touch \"{marker}\"\n\
fi\n\
exit 1\n",
            marker = marker.display()
        );
        write_executable(&temp_bin, binary, &script);

        let config = installable_server_config_with_skip_preflight(binary, true);
        let registry = ServerRegistry::new(HashMap::new(), root.path().to_path_buf(), None);

        let reason = registry
            .recovery_install_reason(
                &LspError::ServerExited {
                    details: "server exited".into(),
                },
                Some(binary),
                &config,
                root.path(),
            )
            .await;

        assert_eq!(reason, None);
        assert!(
            !marker.exists(),
            "skip-preflight recovery should not invoke `{binary} --version`"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recovery_install_reason_runs_preflight_for_server_exited_when_allowed() {
        let _path_lock = PATH_MUTEX.lock().await;
        let temp_bin = tempfile::TempDir::new().unwrap();
        let _guard = PathGuard::prepend(temp_bin.path());
        let root = tempfile::TempDir::new().unwrap();
        let marker = root.path().join("preflight-marker");
        let binary = "test-server-exited-preflight";
        let script = format!(
            "#!/bin/sh\n\
if [ \"$1\" = \"--version\" ]; then\n\
  touch \"{marker}\"\n\
  cat <<'EOF' >&2\n\
{out}EOF\n\
  exit 1\n\
fi\n\
exit 0\n",
            marker = marker.display(),
            out = node_missing_module_output(binary)
        );
        write_executable(&temp_bin, binary, &script);

        let config = installable_server_config(binary);
        let registry = ServerRegistry::new(HashMap::new(), root.path().to_path_buf(), None);

        let reason = registry
            .recovery_install_reason(
                &LspError::ServerExited {
                    details: "server exited".into(),
                },
                Some(binary),
                &config,
                root.path(),
            )
            .await;

        assert_eq!(reason, Some("node shim: cannot find module"));
        assert!(
            marker.exists(),
            "non-skip-preflight recovery should run preflight"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_client_does_not_prompt_install_on_generic_process_exit_for_skip_preflight() {
        let _env_lock = ENV_MUTEX.lock().await;
        let codex_home = tempfile::TempDir::new().unwrap();
        let _codex_home_guard =
            EnvVarGuard::set("CODEX_HOME", codex_home.path().to_string_lossy().as_ref());
        let _path_lock = PATH_MUTEX.lock().await;
        let temp_bin = tempfile::TempDir::new().unwrap();
        let _guard = PathGuard::prepend(temp_bin.path());
        let root = tempfile::TempDir::new().unwrap();
        let binary = "test-generic-process-exit";
        write_executable(
            &temp_bin,
            binary,
            "#!/bin/sh\necho 'generic failure' >&2\nexit 1\n",
        );

        let config = installable_server_config_with_skip_preflight(binary, true);
        let called = Arc::new(AtomicUsize::new(0));
        let called2 = called.clone();
        let runner: InstallRunnerFn = Arc::new(move |_prompt, _cmd| {
            called2.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { false })
        });

        let mut servers = HashMap::new();
        let server_id = "generic-exit";
        servers.insert(server_id.to_string(), config.clone());
        let registry = ServerRegistry::new(servers, root.path().to_path_buf(), Some(runner));
        let key: ClientKey = (server_id.to_string(), root.path().to_path_buf());

        let res = registry
            .spawn_client(server_id, &config, root.path(), &key)
            .await;

        assert!(matches!(
            res,
            Err(LspError::ProcessExitedImmediately { .. })
        ));
        assert_eq!(called.load(Ordering::SeqCst), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_client_process_exit_with_missing_install_signal_attempts_install() {
        let _env_lock = ENV_MUTEX.lock().await;
        let codex_home = tempfile::TempDir::new().unwrap();
        let _codex_home_guard =
            EnvVarGuard::set("CODEX_HOME", codex_home.path().to_string_lossy().as_ref());
        let _path_lock = PATH_MUTEX.lock().await;
        let temp_bin = tempfile::TempDir::new().unwrap();
        let _guard = PathGuard::prepend(temp_bin.path());
        let root = tempfile::TempDir::new().unwrap();
        let binary = "test-process-exit-missing-install";
        let script = format!(
            "#!/bin/sh\n\
cat <<'EOF' >&2\n\
{out}EOF\n\
exit 1\n",
            out = node_missing_module_output(binary)
        );
        write_executable(&temp_bin, binary, &script);

        let config = installable_server_config_with_skip_preflight(binary, true);
        let called = Arc::new(AtomicUsize::new(0));
        let called2 = called.clone();
        let runner: InstallRunnerFn = Arc::new(move |_prompt, _cmd| {
            called2.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { false })
        });

        let mut servers = HashMap::new();
        let server_id = "process-exit-missing-install";
        servers.insert(server_id.to_string(), config.clone());
        let registry = ServerRegistry::new(servers, root.path().to_path_buf(), Some(runner));
        let key: ClientKey = (server_id.to_string(), root.path().to_path_buf());

        let res = registry
            .spawn_client(server_id, &config, root.path(), &key)
            .await;

        assert!(matches!(
            res,
            Err(LspError::ProcessExitedImmediately { .. })
        ));
        assert_eq!(called.load(Ordering::SeqCst), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recovery_retry_applies_post_root_hook() {
        let _env_lock = ENV_MUTEX.lock().await;
        let codex_home = tempfile::TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("CODEX_HOME", codex_home.path().to_string_lossy().as_ref());
        let root = tempfile::TempDir::new().unwrap();
        let broken_bin_dir = tempfile::TempDir::new().unwrap();
        let binary = "test-jdtls-retry-hook";
        let broken_script = format!(
            "#!/bin/sh\n\
cat <<'EOF' >&2\n\
{out}EOF\n\
exit 1\n",
            out = node_missing_module_output(binary)
        );
        let broken_path = write_executable(&broken_bin_dir, "broken-jdtls", &broken_script);
        let args_log = codex_home.path().join("retry-args.log");
        let managed_bin = codex_home.path().join("lsp").join("bin");

        let mut config = installable_server_config_with_skip_preflight(binary, true);
        config.command_candidates = vec![vec![broken_path.to_string_lossy().to_string()]];
        config.post_root_hook = crate::server_config::PostRootHook::JdtlsDataDir;

        let runner: InstallRunnerFn = Arc::new(move |_prompt, _cmd| {
            let managed_bin = managed_bin.clone();
            let args_log = args_log.clone();
            let binary = binary.to_string();
            Box::pin(async move {
                let script = format!(
                    "#!/bin/sh\n\
printf '%s\\n' \"$@\" > \"{log}\"\n\
exit 1\n",
                    log = args_log.display()
                );
                write_executable_at(&managed_bin, &binary, &script);
                true
            })
        });

        let mut servers = HashMap::new();
        let server_id = "retry-hook";
        servers.insert(server_id.to_string(), config.clone());
        let registry = ServerRegistry::new(servers, root.path().to_path_buf(), Some(runner));
        let key: ClientKey = (server_id.to_string(), root.path().to_path_buf());

        let res = registry
            .spawn_client(server_id, &config, root.path(), &key)
            .await;

        assert!(matches!(
            res,
            Err(LspError::ProcessExitedImmediately { .. })
        ));
        let args = std::fs::read_to_string(codex_home.path().join("retry-args.log")).unwrap();
        let expected_dir = codex_home
            .path()
            .join("lsp")
            .join("jdtls-data")
            .join(sha1_short(root.path()));
        let expected_config_dir = expected_dir.join("config");
        assert!(
            args.contains("-data\n"),
            "retry command should include -data: {args}"
        );
        assert!(
            args.contains(&format!("{}\n", expected_dir.display())),
            "retry command should use managed jdtls data dir: {args}"
        );
        assert!(
            args.contains("-configuration\n"),
            "retry command should include -configuration: {args}"
        );
        assert!(
            args.contains(&format!("{}\n", expected_config_dir.display())),
            "retry command should use managed jdtls config dir: {args}"
        );
        assert!(
            args.contains(&format!(
                "--jvm-arg=-Dosgi.configuration.area={}",
                expected_config_dir.display()
            )),
            "retry command should include the osgi config area jvm arg: {args}"
        );
    }

    #[test]
    fn prewarm_skips_server_without_matching_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Main.java"), "class Main {}\n").unwrap();

        assert!(!ServerRegistry::has_matching_files(
            tmp.path(),
            &[".rb".to_string()],
            3
        ));
    }

    #[test]
    fn prewarm_starts_server_with_matching_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Main.java"), "class Main {}\n").unwrap();

        assert!(ServerRegistry::has_matching_files(
            tmp.path(),
            &[".java".to_string()],
            3
        ));
    }

    #[test]
    fn has_matching_files_respects_depth() {
        let tmp = tempfile::TempDir::new().unwrap();
        let nested = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("Main.java"), "class Main {}\n").unwrap();

        assert!(ServerRegistry::has_matching_files(
            tmp.path(),
            &[".java".to_string()],
            3
        ));
        assert!(!ServerRegistry::has_matching_files(
            tmp.path(),
            &[".java".to_string()],
            2
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn auto_install_deduplicates_same_server() {
        let _env_lock = ENV_MUTEX.lock().await;
        let codex_home = tempfile::TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("CODEX_HOME", codex_home.path().to_string_lossy().as_ref());
        let root = tempfile::TempDir::new().unwrap();
        let binary = "test-java-lsp";
        let server_id = "jdtls";
        let config = installable_server_config(binary);
        let managed_bin = codex_home.path().join("lsp").join("bin");
        let install_calls = Arc::new(AtomicUsize::new(0));
        let install_calls_runner = install_calls.clone();
        let runner: InstallRunnerFn = Arc::new(move |_prompt, _cmd| {
            let managed_bin = managed_bin.clone();
            let binary = binary.to_string();
            let install_calls = install_calls_runner.clone();
            Box::pin(async move {
                install_calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                write_executable_at(&managed_bin, &binary, "#!/bin/sh\nexit 0\n");
                true
            })
        });

        let mut servers = HashMap::new();
        servers.insert(server_id.to_string(), config.clone());
        let registry = Arc::new(ServerRegistry::new(
            servers,
            root.path().to_path_buf(),
            Some(runner),
        ));

        let (first, second) = tokio::join!(
            registry.try_auto_install(server_id, &config, binary),
            registry.try_auto_install(server_id, &config, binary),
        );

        assert!(first);
        assert!(second);
        assert_eq!(install_calls.load(Ordering::SeqCst), 1);
        assert!(registry.binary_available(binary));
        assert!(
            codex_home
                .path()
                .join("lsp")
                .join(".install-locks")
                .join(format!("{}.lock", install_lock_name(server_id)))
                .exists()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn auto_install_waiter_retries_after_failed_install() {
        let _env_lock = ENV_MUTEX.lock().await;
        let codex_home = tempfile::TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("CODEX_HOME", codex_home.path().to_string_lossy().as_ref());
        let root = tempfile::TempDir::new().unwrap();
        let binary = "test-java-lsp-retry";
        let server_id = "jdtls";
        let config = installable_server_config(binary);
        let managed_bin = codex_home.path().join("lsp").join("bin");
        let install_calls = Arc::new(AtomicUsize::new(0));
        let install_calls_runner = install_calls.clone();
        let runner: InstallRunnerFn = Arc::new(move |_prompt, _cmd| {
            let managed_bin = managed_bin.clone();
            let binary = binary.to_string();
            let install_calls = install_calls_runner.clone();
            Box::pin(async move {
                let attempt = install_calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(75)).await;
                if attempt == 0 {
                    false
                } else {
                    write_executable_at(&managed_bin, &binary, "#!/bin/sh\nexit 0\n");
                    true
                }
            })
        });

        let mut servers = HashMap::new();
        servers.insert(server_id.to_string(), config.clone());
        let registry = Arc::new(ServerRegistry::new(
            servers,
            root.path().to_path_buf(),
            Some(runner),
        ));

        let (first, second) = tokio::join!(
            registry.try_auto_install(server_id, &config, binary),
            registry.try_auto_install(server_id, &config, binary),
        );

        assert_eq!(install_calls.load(Ordering::SeqCst), 2);
        assert_eq!([first, second].into_iter().filter(|ok| *ok).count(), 1);
        assert!(registry.binary_available(binary));
    }

    #[test]
    fn managed_bin_dirs_includes_gem() {
        let _env_lock = ENV_MUTEX.blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("CODEX_HOME", tmp.path().to_string_lossy().as_ref());

        let dirs = managed_lsp_bin_dirs();
        assert!(dirs.contains(&tmp.path().join("lsp").join("gem").join("bin")));
    }
}
