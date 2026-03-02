//! Registry that manages a pool of LSP clients, one per (server_id, root) pair.

use std::collections::HashMap;
use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::RwLock;

use lsp_types::request::GotoImplementationResponse;
use lsp_types::*;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tracing;

use crate::client::LspClient;
use crate::client::path_from_uri;
use crate::error::LspError;
use crate::root_discovery::dir_has_any_marker;
use crate::root_discovery::nearest_root;
use crate::server_config::LspServerConfig;

/// Key for de-duplicating clients: (server_id, root_path).
type ClientKey = (String, PathBuf);

const PREFLIGHT_VERSION_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);
const PREFLIGHT_OUTPUT_MAX_BYTES: usize = 8 * 1024;

#[derive(Debug)]
enum PreflightResult {
    Installed,
    NeedsInstall(&'static str),
    Inconclusive,
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
    clients: Mutex<HashMap<ClientKey, Arc<LspClient>>>,
    /// Servers that failed to start (never retry within session).
    broken: Mutex<HashSet<ClientKey>>,
    /// In-flight spawns for dedup.
    spawning: Mutex<HashMap<ClientKey, Arc<tokio::sync::Notify>>>,
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
            clients: Mutex::new(HashMap::new()),
            broken: Mutex::new(HashSet::new()),
            spawning: Mutex::new(HashMap::new()),
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
            if config.install.is_some() {
                // If the binary isn't on PATH at all, try installing it.
                if which::which(binary).is_err() {
                    let installed = self.try_auto_install(server_id, config, binary).await;
                    if !installed {
                        self.spawning.lock().await.remove(key);
                        notify.notify_waiters();
                        return Err(LspError::BinaryNotFound(binary.into()));
                    }
                } else {
                    // Binary exists, but may be a shim (e.g. rustup proxy) that fails
                    // because the actual component isn't installed.
                    if let PreflightResult::NeedsInstall(reason) =
                        self.preflight_version_check(binary, config, root).await
                    {
                        tracing::debug!(
                            server = %server_id,
                            binary = %binary,
                            "preflight indicates missing install ({reason}); attempting auto-install"
                        );
                        let installed = self.try_auto_install(server_id, config, binary).await;
                        if !installed {
                            self.spawning.lock().await.remove(key);
                            notify.notify_waiters();
                            return Err(LspError::BinaryNotFound(binary.into()));
                        }
                    }
                }
            }
        }

        let result = LspClient::create(server_id, config, root).await;

        // If the process exited immediately (broken shim), try auto-install and retry.
        let result = match result {
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
                if let Some(binary) = config.binary_name() {
                    if self.try_auto_install(server_id, config, binary).await {
                        LspClient::create(server_id, config, root).await
                    } else {
                        result
                    }
                } else {
                    result
                }
            }
            // Some shims don't exit within the 30ms early-death window but still
            // terminate before responding to `initialize`.
            Err(LspError::ServerExited) if config.install.is_some() => {
                if let Some(binary) = config.binary_name() {
                    match self.preflight_version_check(binary, config, root).await {
                        PreflightResult::NeedsInstall(reason) => {
                            tracing::warn!(
                                server = %server_id,
                                binary = %binary,
                                "server exited during init; preflight indicates missing install ({reason}); attempting auto-install"
                            );
                            if self.try_auto_install(server_id, config, binary).await {
                                LspClient::create(server_id, config, root).await
                            } else {
                                result
                            }
                        }
                        _ => result,
                    }
                } else {
                    result
                }
            }
            other => other,
        };

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
        which::which(binary).is_ok()
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
        if clients.is_empty() {
            return Vec::new();
        }

        let path = path.to_path_buf();
        let mut tasks = JoinSet::new();
        for (_, client) in clients {
            let path = path.clone();
            tasks.spawn(async move { client.references(&path, line, character).await });
        }

        let mut all = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(mut refs) => all.append(&mut refs),
                Err(e) => tracing::debug!("references query task failed: {e}"),
            }
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
        self.ensure_workspace_clients_started().await;
        let clients: Vec<Arc<LspClient>> = {
            let clients_map = self.clients.lock().await;
            clients_map.values().cloned().collect()
        };

        if clients.is_empty() {
            return Vec::new();
        }

        let mut tasks = JoinSet::new();
        for client in clients {
            let query = query.to_string();
            tasks.spawn(async move { client.workspace_symbol(&query).await });
        }

        let mut all = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(mut symbols) => all.append(&mut symbols),
                Err(e) => tracing::debug!("workspace_symbol query task failed: {e}"),
            }
        }
        all
    }

    async fn ensure_workspace_clients_started(&self) {
        let candidates: Vec<(String, LspServerConfig)> = self
            .servers
            .iter()
            .filter_map(|(server_id, config)| {
                if config.disabled {
                    return None;
                }
                if !dir_has_any_marker(&self.workspace_root, &config.root_markers) {
                    return None;
                }
                Some((server_id.clone(), config.clone()))
            })
            .collect();

        for (server_id, config) in candidates {
            let root = self.workspace_root.clone();
            let key: ClientKey = (server_id.clone(), root.clone());

            if self.broken.lock().await.contains(&key) {
                continue;
            }

            {
                let clients = self.clients.lock().await;
                if clients.contains_key(&key) {
                    continue;
                }
            }

            match self.spawn_client(&server_id, &config, &root, &key).await {
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(
                        server = %server_id,
                        root = %root.display(),
                        "failed to spawn LSP client for workspace_symbol: {e}"
                    );
                    self.broken.lock().await.insert(key);
                }
            }
        }
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
        if clients.is_empty() {
            return Vec::new();
        }

        let path = path.to_path_buf();
        let mut tasks = JoinSet::new();
        for (_, client) in clients {
            let path = path.clone();
            tasks.spawn(async move { client.prepare_call_hierarchy(&path, line, character).await });
        }

        let mut all = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(mut items) => all.append(&mut items),
                Err(e) => tracing::debug!("prepare_call_hierarchy query task failed: {e}"),
            }
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
        if clients.is_empty() {
            return Vec::new();
        }

        let mut tasks = JoinSet::new();
        for (_, client) in clients {
            let item = item.clone();
            tasks.spawn(async move { client.incoming_calls(item).await });
        }

        let mut all = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(mut calls) => all.append(&mut calls),
                Err(e) => tracing::debug!("incoming_calls query task failed: {e}"),
            }
        }
        all
    }

    pub async fn outgoing_calls(&self, item: CallHierarchyItem) -> Vec<CallHierarchyOutgoingCall> {
        let path = match path_from_uri(&item.uri) {
            Some(p) => p,
            None => return Vec::new(),
        };
        let clients = self.get_clients(&path).await;
        if clients.is_empty() {
            return Vec::new();
        }

        let mut tasks = JoinSet::new();
        for (_, client) in clients {
            let item = item.clone();
            tasks.spawn(async move { client.outgoing_calls(item).await });
        }

        let mut all = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(mut calls) => all.append(&mut calls),
                Err(e) => tracing::debug!("outgoing_calls query task failed: {e}"),
            }
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

fn output_indicates_missing_install(binary: &str, combined_output: &str) -> Option<&'static str> {
    // Keep this intentionally strict. We only want to auto-install when we're
    // very confident the "binary exists" but is unusable due to missing install.
    //
    // Primary target: rustup proxy for rust-analyzer.
    let bin_lc = binary.to_ascii_lowercase();
    let out_lc = combined_output.to_ascii_lowercase();

    // rustup shim patterns
    if out_lc.contains("unknown binary") && out_lc.contains(&bin_lc) {
        return Some("rustup shim: unknown binary/component");
    }
    if out_lc.contains("not installed for the toolchain") && out_lc.contains(&bin_lc) {
        return Some("rustup shim: component not installed");
    }
    if out_lc.contains("rustup component add") && out_lc.contains(&bin_lc) {
        return Some("rustup shim: suggests rustup component add");
    }

    // node/npm global module missing patterns (best-effort)
    if out_lc.contains("cannot find module") && out_lc.contains(&bin_lc) {
        return Some("node shim: cannot find module");
    }

    // python/pip missing module patterns (best-effort)
    if out_lc.contains("modulenotfounderror") && out_lc.contains(&bin_lc) {
        return Some("python shim: module not found");
    }
    if out_lc.contains("no module named") && out_lc.contains(&bin_lc) {
        return Some("python shim: module not found");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

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
    async fn spawn_client_prompts_install_on_preflight_needs_install_and_decline() {
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
            env: HashMap::new(),
            root_markers: Vec::new(),
            initialization_options: None,
            disabled: false,
            install: Some(crate::server_config::InstallConfig {
                method: crate::server_config::InstallMethod::Cargo {
                    package: Some("rust-analyzer".into()),
                },
            }),
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
}
