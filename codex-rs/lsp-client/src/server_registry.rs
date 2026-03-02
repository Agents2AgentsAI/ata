//! Registry that manages a pool of LSP clients, one per (server_id, root) pair.

use std::collections::HashMap;
use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use lsp_types::request::GotoImplementationResponse;
use lsp_types::*;
use tokio::sync::Mutex;
use tracing;

use crate::client::LspClient;
use crate::client::path_from_uri;
use crate::error::LspError;
use crate::root_discovery::nearest_root;
use crate::server_config::LspServerConfig;

/// Key for de-duplicating clients: (server_id, root_path).
type ClientKey = (String, PathBuf);

/// Callback type for confirming auto-install with the user.
pub type InstallConfirmFn = Arc<
    dyn Fn(&str, &[String]) -> std::pin::Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync,
>;

/// Manages a pool of LSP clients.
pub struct ServerRegistry {
    /// Server configurations keyed by server_id.
    servers: HashMap<String, LspServerConfig>,
    /// Active clients keyed by (server_id, root).
    clients: Mutex<HashMap<ClientKey, Arc<LspClient>>>,
    /// Servers that failed to start (never retry within session).
    broken: Mutex<HashSet<ClientKey>>,
    /// In-flight spawns for dedup.
    spawning: Mutex<HashMap<ClientKey, Arc<tokio::sync::Notify>>>,
    /// Workspace root directory.
    workspace_root: PathBuf,
    /// Optional install confirmation callback.
    install_confirm: Option<InstallConfirmFn>,
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
        install_confirm: Option<InstallConfirmFn>,
    ) -> Self {
        Self {
            servers,
            clients: Mutex::new(HashMap::new()),
            broken: Mutex::new(HashSet::new()),
            spawning: Mutex::new(HashMap::new()),
            workspace_root,
            install_confirm,
        }
    }

    /// Get or spawn all applicable clients for a given file.
    /// Returns a list of (server_id, client) pairs.
    pub async fn get_clients(&self, file: &Path) -> Vec<(String, Arc<LspClient>)> {
        let ext = match file.extension().and_then(|e| e.to_str()) {
            Some(e) => format!(".{e}"),
            None => return Vec::new(),
        };

        let mut result = Vec::new();

        for (server_id, config) in &self.servers {
            if config.disabled || !config.matches_extension(&ext) {
                continue;
            }

            let root = nearest_root(file, &self.workspace_root, &config.root_markers);
            let key: ClientKey = (server_id.clone(), root.clone());

            // Check if broken.
            if self.broken.lock().await.contains(&key) {
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
                    self.broken.lock().await.insert(key);
                }
            }
        }

        result
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
        let notify = {
            let mut spawning = self.spawning.lock().await;
            if let Some(existing) = spawning.get(key) {
                let n = existing.clone();
                drop(spawning);
                // Wait for the other spawn to finish.
                n.notified().await;
                // Now check the clients map.
                let clients = self.clients.lock().await;
                if let Some(client) = clients.get(key) {
                    return Ok(client.clone());
                }
                return Err(LspError::SpawnFailed("concurrent spawn failed".into()));
            }
            let n = Arc::new(tokio::sync::Notify::new());
            spawning.insert(key.clone(), n.clone());
            n
        };

        // Check binary existence and auto-install if needed.
        if let Some(binary) = config.binary_name() {
            if which::which(binary).is_err() {
                let installed = self.try_auto_install(server_id, config, binary).await;
                if !installed {
                    self.spawning.lock().await.remove(key);
                    notify.notify_waiters();
                    return Err(LspError::BinaryNotFound(binary.into()));
                }
            }
        }

        let result = LspClient::create(server_id, config, root).await;

        // Clean up spawning entry and store result.
        self.spawning.lock().await.remove(key);

        match result {
            Ok(client) => {
                self.clients
                    .lock()
                    .await
                    .insert(key.clone(), client.clone());
                notify.notify_waiters();
                Ok(client)
            }
            Err(e) => {
                notify.notify_waiters();
                Err(e)
            }
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

        // Ask user for confirmation if callback is set.
        if let Some(confirm) = &self.install_confirm {
            let approved = confirm(
                &format!("Install {server_id} via `{}`?", cmd.join(" ")),
                &cmd,
            )
            .await;
            if !approved {
                return false;
            }
        } else {
            // No confirmation callback — skip auto-install.
            return false;
        }

        // Run the install command.
        tracing::info!(server = %server_id, "installing: {}", cmd.join(" "));
        let status = tokio::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .status()
            .await;

        match status {
            Ok(s) if s.success() => {
                // Verify the binary is now available.
                which::which(binary).is_ok()
            }
            Ok(s) => {
                tracing::warn!(
                    server = %server_id,
                    "install command exited with status {s}"
                );
                false
            }
            Err(e) => {
                tracing::warn!(server = %server_id, "install command failed: {e}");
                false
            }
        }
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
        let clients = self.get_clients(path).await;
        for (_, client) in clients {
            if let Some(result) = client.hover(path, line, character).await {
                return Some(result);
            }
        }
        None
    }

    pub async fn definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Option<GotoDefinitionResponse> {
        let clients = self.get_clients(path).await;
        for (_, client) in clients {
            if let Some(result) = client.definition(path, line, character).await {
                return Some(result);
            }
        }
        None
    }

    pub async fn references(&self, path: &Path, line: u32, character: u32) -> Vec<Location> {
        let clients = self.get_clients(path).await;
        let mut all = Vec::new();
        for (_, client) in clients {
            all.extend(client.references(path, line, character).await);
        }
        all
    }

    pub async fn document_symbol(&self, path: &Path) -> Option<DocumentSymbolResponse> {
        let clients = self.get_clients(path).await;
        for (_, client) in clients {
            if let Some(result) = client.document_symbol(path).await {
                return Some(result);
            }
        }
        None
    }

    pub async fn workspace_symbol(&self, query: &str) -> Vec<SymbolInformation> {
        let clients_map = self.clients.lock().await;
        let mut all = Vec::new();
        for client in clients_map.values() {
            all.extend(client.workspace_symbol(query).await);
        }
        all
    }

    pub async fn implementation(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Option<GotoImplementationResponse> {
        let clients = self.get_clients(path).await;
        for (_, client) in clients {
            if let Some(result) = client.implementation(path, line, character).await {
                return Some(result);
            }
        }
        None
    }

    pub async fn prepare_call_hierarchy(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Vec<CallHierarchyItem> {
        let clients = self.get_clients(path).await;
        let mut all = Vec::new();
        for (_, client) in clients {
            all.extend(client.prepare_call_hierarchy(path, line, character).await);
        }
        all
    }

    pub async fn incoming_calls(&self, item: CallHierarchyItem) -> Vec<CallHierarchyIncomingCall> {
        // Route to all clients — the item.uri tells us which server owns it.
        let path = match path_from_uri(&item.uri) {
            Some(p) => p,
            None => return Vec::new(),
        };
        let clients = self.get_clients(&path).await;
        let mut all = Vec::new();
        for (_, client) in clients {
            all.extend(client.incoming_calls(item.clone()).await);
        }
        all
    }

    pub async fn outgoing_calls(&self, item: CallHierarchyItem) -> Vec<CallHierarchyOutgoingCall> {
        let path = match path_from_uri(&item.uri) {
            Some(p) => p,
            None => return Vec::new(),
        };
        let clients = self.get_clients(&path).await;
        let mut all = Vec::new();
        for (_, client) in clients {
            all.extend(client.outgoing_calls(item.clone()).await);
        }
        all
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
