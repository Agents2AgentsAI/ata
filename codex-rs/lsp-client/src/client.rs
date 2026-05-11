//! LSP client: spawn a server process, run the initialize handshake,
//! synchronize documents, wait for diagnostics, and dispatch queries.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering;

use lsp_types::request::GotoImplementationParams;
use lsp_types::request::GotoImplementationResponse;
use lsp_types::*;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::ChildStdin;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tokio::sync::oneshot;
use tokio_stream::StreamExt;
use tokio_util::codec::FramedRead;

use crate::error::LspError;
use crate::jsonrpc::*;
use crate::language::language_id_for_path;
use crate::server_config::LspServerConfig;

/// Timeout for the initialize handshake.
const INITIALIZE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// Timeout for shutdown request.
const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Default timeout for LSP queries (hover, definition, etc.).
const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Debounce period before returning diagnostics.
const DIAG_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);

/// Maximum wait time for diagnostics after a file sync.
const DIAG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// Skip syncing very large files to avoid excessive JSON payloads.
const MAX_SYNC_FILE_SIZE: u64 = 5 * 1024 * 1024;
const STDERR_TAIL_MAX_BYTES: usize = 8 * 1024;

type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<JsonRpcResponse>>>>;

struct ReaderContext {
    pending: PendingMap,
    diagnostics: Arc<RwLock<HashMap<String, Vec<Diagnostic>>>>,
    diag_tx: broadcast::Sender<String>,
    writer: Arc<Mutex<ChildStdin>>,
    initialization_options: Option<Value>,
    root_uri: Uri,
    server_id: String,
}

// ---------------------------------------------------------------------------
// URI helpers: lsp_types::Uri ↔ file path
// ---------------------------------------------------------------------------

fn uri_from_path(path: &Path) -> Option<Uri> {
    let url = url::Url::from_file_path(path).ok()?;
    url.as_str().parse::<Uri>().ok()
}

fn uri_from_directory(path: &Path) -> Option<Uri> {
    let url = url::Url::from_directory_path(path).ok()?;
    url.as_str().parse::<Uri>().ok()
}

pub(crate) fn path_from_uri(uri: &Uri) -> Option<std::path::PathBuf> {
    let url = url::Url::parse(uri.as_str()).ok()?;
    url.to_file_path().ok()
}

fn lock_unpoisoned<T>(mutex: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn append_stderr_tail(buffer: &StdMutex<String>, chunk: &str) {
    let mut stderr_tail = lock_unpoisoned(buffer);
    if !stderr_tail.is_empty() {
        stderr_tail.push('\n');
    }
    stderr_tail.push_str(chunk);
    if stderr_tail.len() > STDERR_TAIL_MAX_BYTES {
        let mut trim_at = stderr_tail.len() - STDERR_TAIL_MAX_BYTES;
        while trim_at < stderr_tail.len() && !stderr_tail.is_char_boundary(trim_at) {
            trim_at += 1;
        }
        stderr_tail.drain(..trim_at);
    }
}

fn snapshot_stderr_tail(buffer: &StdMutex<String>) -> Option<String> {
    let stderr_tail = lock_unpoisoned(buffer);
    (!stderr_tail.is_empty()).then(|| stderr_tail.clone())
}

/// A running LSP client connected to a single server process.
pub struct LspClient {
    /// Stdin writer to the server process, protected by a mutex.
    writer: Arc<Mutex<ChildStdin>>,
    /// Auto-incrementing request id.
    next_id: AtomicI64,
    /// Pending request map: id → oneshot sender for the response.
    pending: PendingMap,
    /// Current diagnostics per file URI string.
    diagnostics: Arc<RwLock<HashMap<String, Vec<Diagnostic>>>>,
    /// Broadcast channel notifying when diagnostics change (URI string).
    diag_tx: broadcast::Sender<String>,
    /// File version counter per URI string.
    file_versions: Arc<Mutex<HashMap<String, i32>>>,
    /// Server name (for logging).
    server_id: String,
    /// Root URI for this client.
    root_uri: Uri,
    /// Initialization options from the config.
    initialization_options: Option<Value>,
    /// Handle to the child process (for kill on shutdown).
    child: Arc<Mutex<Option<Child>>>,
    /// Last stderr lines seen from the server process.
    stderr_tail: Arc<StdMutex<String>>,
    /// Optional JSONL trace writer for requests/responses (debug-only).
    trace: Option<Arc<Mutex<tokio::fs::File>>>,
}

impl LspClient {
    /// Spawn a new LSP server and run the `initialize` → `initialized` handshake.
    pub async fn create(
        server_id: &str,
        config: &LspServerConfig,
        root: &Path,
    ) -> Result<Arc<Self>, LspError> {
        let root_canonical = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let root_uri = uri_from_directory(&root_canonical)
            .ok_or_else(|| LspError::SpawnFailed("invalid root path".into()))?;

        let binary = config
            .command
            .first()
            .ok_or_else(|| LspError::SpawnFailed("empty command".into()))?;
        let args = &config.command[1..];

        let mut cmd = tokio::process::Command::new(binary);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(&root_canonical)
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);

        // On Linux, request SIGTERM-on-parent-death so the kernel kills this
        // LSP if ata exits ungracefully (OOM, SIGKILL, panic). Without this,
        // `kill_on_drop` does not fire and the child is reparented to PID 1,
        // keeping ~10 GB of rust-analyzer resident per crashed session.
        #[cfg(target_os = "linux")]
        unsafe {
            let parent_pid = libc::getpid();
            cmd.pre_exec(move || {
                codex_utils_pty::process_group::set_parent_death_signal(parent_pid)
            });
        }

        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| LspError::SpawnFailed(format!("{binary}: {e}")))?;
        let pid = child.id();
        tracing::info!(
            server = server_id,
            binary,
            pid = pid.unwrap_or_default(),
            root = %root_canonical.display(),
            "spawned lsp server"
        );

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| LspError::SpawnFailed("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| LspError::SpawnFailed("no stdout".into()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| LspError::SpawnFailed("no stderr".into()))?;

        // Early-death detection: wait briefly for shims that exit immediately.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        if let Some(exit_status) = child.try_wait().map_err(LspError::Io)? {
            let mut stderr_buf = String::new();
            let _ = stderr.read_to_string(&mut stderr_buf).await;
            let stderr_msg = stderr_buf.trim().to_string();
            return Err(LspError::ProcessExitedImmediately {
                status: exit_status.to_string(),
                stderr: stderr_msg,
            });
        }

        let stderr_tail = Arc::new(StdMutex::new(String::new()));
        // Server is still running — spawn a background task to drain stderr.
        let drain_server_id = server_id.to_string();
        let drain_stderr_tail = stderr_tail.clone();
        tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                append_stderr_tail(drain_stderr_tail.as_ref(), &line);
                tracing::trace!(server = %drain_server_id, "stderr: {line}");
            }
        });

        let (diag_tx, _) = broadcast::channel(256);
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let diagnostics: Arc<RwLock<HashMap<String, Vec<Diagnostic>>>> =
            Arc::new(RwLock::new(HashMap::new()));

        let trace = open_trace_file().await;

        let client = Arc::new(Self {
            writer: Arc::new(Mutex::new(stdin)),
            next_id: AtomicI64::new(1),
            pending: pending.clone(),
            diagnostics: diagnostics.clone(),
            diag_tx: diag_tx.clone(),
            file_versions: Arc::new(Mutex::new(HashMap::new())),
            server_id: server_id.to_string(),
            root_uri: root_uri.clone(),
            initialization_options: config.initialization_options.clone(),
            child: Arc::new(Mutex::new(Some(child))),
            stderr_tail,
            trace,
        });

        // Spawn the reader task.
        let reader = BufReader::new(stdout);
        let framed = FramedRead::new(reader, crate::jsonrpc::LspCodec::new());
        let reader_context = ReaderContext {
            pending: pending.clone(),
            diagnostics: diagnostics.clone(),
            diag_tx: diag_tx.clone(),
            writer: client.writer.clone(),
            initialization_options: client.initialization_options.clone(),
            root_uri: root_uri.clone(),
            server_id: server_id.to_string(),
        };
        tokio::spawn(Self::reader_loop(framed, reader_context));

        // Run initialize handshake.
        match client.initialize_handshake().await {
            Ok(()) => Ok(client),
            Err(handshake_err) => {
                // If the server process has already exited, this is likely a
                // broken shim (e.g. rustup proxy without the component) rather
                // than a legitimate handshake failure.  Convert to
                // ProcessExitedImmediately so callers can attempt auto-install.
                let exit_status = {
                    let mut guard = client.child.lock().await;
                    if let Some(ref mut child) = *guard {
                        child.try_wait().ok().flatten()
                    } else {
                        None
                    }
                };
                if let Some(exit_status) = exit_status {
                    return Err(LspError::ProcessExitedImmediately {
                        status: exit_status.to_string(),
                        stderr: client
                            .server_exit_details(
                                format!(
                                    "server exited during initialize handshake: {handshake_err}"
                                ),
                                false,
                            )
                            .await,
                    });
                }
                Err(handshake_err)
            }
        }
    }

    async fn child_exit_status(&self) -> Option<String> {
        let mut guard = self.child.lock().await;
        guard
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten())
            .map(|status| status.to_string())
    }

    async fn server_exit_details(&self, context: String, include_status: bool) -> String {
        let mut details = context;
        if include_status && let Some(status) = self.child_exit_status().await {
            details.push_str("\nstatus=");
            details.push_str(&status);
        }
        if let Some(stderr) = snapshot_stderr_tail(self.stderr_tail.as_ref()) {
            details.push_str("\nstderr:\n");
            details.push_str(&stderr);
        }
        details
    }

    async fn server_exited_error(&self, context: String) -> LspError {
        LspError::ServerExited {
            details: self.server_exit_details(context, true).await,
        }
    }

    /// Initialize handshake: send `initialize`, wait for response, send `initialized`.
    async fn initialize_handshake(self: &Arc<Self>) -> Result<(), LspError> {
        #[allow(deprecated)]
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(self.root_uri.clone()),
            capabilities: Self::client_capabilities(),
            initialization_options: self.initialization_options.clone(),
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: self.root_uri.clone(),
                name: "workspace".into(),
            }]),
            ..Default::default()
        };

        let result = tokio::time::timeout(
            INITIALIZE_TIMEOUT,
            self.send_request(
                "initialize",
                Some(serde_json::to_value(params).map_err(LspError::Json)?),
            ),
        )
        .await
        .map_err(|_| LspError::InitializeTimeout)?
        .map_err(|e| {
            tracing::warn!(server = %self.server_id, "initialize failed: {e}");
            e
        })?;

        tracing::debug!(
            server = %self.server_id,
            "initialized (result keys: {:?})",
            result.as_object().map(|o| o.keys().collect::<Vec<_>>())
        );

        // Send `initialized` notification.
        self.send_notification(
            "initialized",
            Some(serde_json::to_value(InitializedParams {}).map_err(LspError::Json)?),
        )
        .await?;

        // Always send workspace/didChangeConfiguration. Some servers (notably
        // Pyright) only issue workspace/configuration requests in response to
        // this notification.  Without it they never fetch configuration and
        // silently block on all subsequent file operations.
        let settings = self
            .initialization_options
            .clone()
            .unwrap_or(serde_json::json!({}));
        self.send_notification(
            "workspace/didChangeConfiguration",
            Some(serde_json::json!({ "settings": settings })),
        )
        .await?;

        Ok(())
    }

    /// Minimal client capabilities for diagnostics + queries.
    fn client_capabilities() -> ClientCapabilities {
        ClientCapabilities {
            window: Some(WindowClientCapabilities {
                work_done_progress: Some(true),
                ..Default::default()
            }),
            workspace: Some(WorkspaceClientCapabilities {
                configuration: Some(true),
                workspace_edit: Some(WorkspaceEditClientCapabilities {
                    document_changes: Some(true),
                    resource_operations: Some(vec![
                        ResourceOperationKind::Create,
                        ResourceOperationKind::Rename,
                        ResourceOperationKind::Delete,
                    ]),
                    ..Default::default()
                }),
                did_change_watched_files: Some(DidChangeWatchedFilesClientCapabilities {
                    dynamic_registration: Some(true),
                    relative_pattern_support: Some(false),
                }),
                workspace_folders: Some(true),
                ..Default::default()
            }),
            text_document: Some(TextDocumentClientCapabilities {
                synchronization: Some(TextDocumentSyncClientCapabilities {
                    dynamic_registration: Some(false),
                    will_save: Some(false),
                    will_save_wait_until: Some(false),
                    did_save: Some(true),
                }),
                publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                    version_support: Some(true),
                    ..Default::default()
                }),
                hover: Some(HoverClientCapabilities {
                    dynamic_registration: Some(false),
                    content_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
                }),
                definition: Some(GotoCapability {
                    dynamic_registration: Some(false),
                    link_support: Some(false),
                }),
                references: Some(DynamicRegistrationClientCapabilities {
                    dynamic_registration: Some(false),
                }),
                document_symbol: Some(DocumentSymbolClientCapabilities {
                    dynamic_registration: Some(false),
                    ..Default::default()
                }),
                implementation: Some(GotoCapability {
                    dynamic_registration: Some(false),
                    link_support: Some(false),
                }),
                code_action: Some(CodeActionClientCapabilities {
                    dynamic_registration: Some(false),
                    resolve_support: Some(CodeActionCapabilityResolveSupport {
                        properties: vec!["edit".into()],
                    }),
                    ..Default::default()
                }),
                rename: Some(RenameClientCapabilities {
                    dynamic_registration: Some(false),
                    prepare_support: Some(true),
                    ..Default::default()
                }),
                call_hierarchy: Some(CallHierarchyClientCapabilities {
                    dynamic_registration: Some(false),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    // -----------------------------------------------------------------------
    // Document sync
    // -----------------------------------------------------------------------

    /// Notify the server about a file being opened or changed (full-document sync).
    pub async fn notify_open(&self, path: &Path) -> Result<(), LspError> {
        let metadata = tokio::fs::metadata(path).await.map_err(LspError::Io)?;
        if metadata.len() > MAX_SYNC_FILE_SIZE {
            tracing::warn!(
                server = %self.server_id,
                path = %path.display(),
                file_size_bytes = metadata.len(),
                max_sync_file_size_bytes = MAX_SYNC_FILE_SIZE,
                "skipping LSP sync for oversized file"
            );
            return Ok(());
        }

        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(LspError::Io)?;
        let uri = uri_from_path(path).ok_or_else(|| {
            LspError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "bad path",
            ))
        })?;
        let uri_key = uri.as_str().to_string();
        let language_id = language_id_for_path(path).unwrap_or("plaintext");

        let mut versions = self.file_versions.lock().await;
        let is_new = !versions.contains_key(&uri_key);

        if is_new {
            versions.insert(uri_key.clone(), 0);
            drop(versions);

            // Send didChangeWatchedFiles (Created).
            self.send_file_watch_event(&uri, FileChangeType::CREATED)
                .await?;

            // Send textDocument/didOpen.
            self.send_notification(
                "textDocument/didOpen",
                Some(
                    serde_json::to_value(DidOpenTextDocumentParams {
                        text_document: TextDocumentItem {
                            uri,
                            language_id: language_id.to_string(),
                            version: 0,
                            text: content,
                        },
                    })
                    .map_err(LspError::Json)?,
                ),
            )
            .await?;
        } else {
            let version = versions.get(&uri_key).copied().unwrap_or(0) + 1;
            versions.insert(uri_key.clone(), version);
            drop(versions);

            // Send didChangeWatchedFiles (Changed).
            self.send_file_watch_event(&uri, FileChangeType::CHANGED)
                .await?;

            // Send textDocument/didChange with full content.
            self.send_notification(
                "textDocument/didChange",
                Some(
                    serde_json::to_value(DidChangeTextDocumentParams {
                        text_document: VersionedTextDocumentIdentifier { uri, version },
                        content_changes: vec![TextDocumentContentChangeEvent {
                            range: None,
                            range_length: None,
                            text: content,
                        }],
                    })
                    .map_err(LspError::Json)?,
                ),
            )
            .await?;
        }

        Ok(())
    }

    /// Ensure the server has seen a `didOpen` for this file without forcing a
    /// redundant `didChange` when the file is already synchronized.
    pub async fn ensure_open(&self, path: &Path) -> Result<(), LspError> {
        let uri = uri_from_path(path).ok_or_else(|| {
            LspError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "bad path",
            ))
        })?;
        let uri_key = uri.as_str().to_string();
        if self.file_versions.lock().await.contains_key(&uri_key) {
            return Ok(());
        }
        self.notify_open(path).await
    }

    async fn send_file_watch_event(&self, uri: &Uri, typ: FileChangeType) -> Result<(), LspError> {
        self.send_notification(
            "workspace/didChangeWatchedFiles",
            Some(
                serde_json::to_value(DidChangeWatchedFilesParams {
                    changes: vec![FileEvent {
                        uri: uri.clone(),
                        typ,
                    }],
                })
                .map_err(LspError::Json)?,
            ),
        )
        .await
    }

    // -----------------------------------------------------------------------
    // Diagnostics
    // -----------------------------------------------------------------------

    /// Wait for diagnostics to settle (debounced) with a timeout.
    pub async fn wait_for_diagnostics(&self, path: &Path) -> Vec<Diagnostic> {
        let uri = match uri_from_path(path) {
            Some(u) => u,
            None => return Vec::new(),
        };
        let uri_key = uri.as_str().to_string();
        let mut rx = self.diag_tx.subscribe();

        // Wait for diagnostics with debounce.
        let _result = tokio::time::timeout(DIAG_TIMEOUT, async {
            // Wait until we observe diagnostics for this URI.
            loop {
                match rx.recv().await {
                    Ok(changed_uri) if changed_uri == uri_key => break,
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }

            // Once observed, require a quiet period for this URI before returning.
            let mut quiet_deadline = tokio::time::Instant::now() + DIAG_DEBOUNCE;
            loop {
                let now = tokio::time::Instant::now();
                if now >= quiet_deadline {
                    return;
                }
                let remaining = quiet_deadline - now;

                match tokio::time::timeout(remaining, rx.recv()).await {
                    Ok(Ok(changed_uri)) if changed_uri == uri_key => {
                        quiet_deadline = tokio::time::Instant::now() + DIAG_DEBOUNCE;
                    }
                    Ok(Ok(_)) => continue,
                    Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                    Ok(Err(broadcast::error::RecvError::Closed)) => return,
                    Err(_) => return,
                }
            }
        })
        .await;

        // Return current diagnostics regardless.
        let diags = self.diagnostics.read().await;
        diags.get(&uri_key).cloned().unwrap_or_default()
    }

    /// Get current diagnostics for a file without waiting.
    pub async fn diagnostics_for(&self, path: &Path) -> Vec<Diagnostic> {
        let uri = match uri_from_path(path) {
            Some(u) => u,
            None => return Vec::new(),
        };
        let diags = self.diagnostics.read().await;
        diags.get(uri.as_str()).cloned().unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // LSP queries
    // -----------------------------------------------------------------------

    pub async fn hover(&self, path: &Path, line: u32, character: u32) -> Option<Hover> {
        let params = HoverParams {
            text_document_position_params: self.text_doc_pos(path, line, character)?,
            work_done_progress_params: Default::default(),
        };
        let val = self.query("textDocument/hover", params).await?;
        self.deserialize_response("textDocument/hover", val)
    }

    pub async fn definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Option<GotoDefinitionResponse> {
        let params = GotoDefinitionParams {
            text_document_position_params: self.text_doc_pos(path, line, character)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let val = self.query("textDocument/definition", params).await?;
        self.deserialize_response("textDocument/definition", val)
    }

    pub async fn references(&self, path: &Path, line: u32, character: u32) -> Vec<Location> {
        let Some(tdp) = self.text_doc_pos(path, line, character) else {
            return Vec::new();
        };
        let params = ReferenceParams {
            text_document_position: tdp,
            context: ReferenceContext {
                include_declaration: true,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let val = match self.query("textDocument/references", params).await {
            Some(v) => v,
            None => return Vec::new(),
        };
        self.deserialize_or_default("textDocument/references", val)
    }

    pub async fn document_symbol(&self, path: &Path) -> Option<DocumentSymbolResponse> {
        let uri = uri_from_path(path)?;
        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier { uri },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let val = self.query("textDocument/documentSymbol", params).await?;
        self.deserialize_response("textDocument/documentSymbol", val)
    }

    #[allow(deprecated)]
    pub async fn workspace_symbol(&self, query: &str) -> Vec<SymbolInformation> {
        let params = WorkspaceSymbolParams {
            query: query.to_string(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let val = match self.query("workspace/symbol", params).await {
            Some(v) => v,
            None => return Vec::new(),
        };

        // Try flat Vec<SymbolInformation> first (most common).
        if let Ok(symbols) = serde_json::from_value::<Vec<SymbolInformation>>(val.clone()) {
            return symbols;
        }
        // Try WorkspaceSymbolResponse enum (handles both Flat and Nested).
        if let Ok(response) = serde_json::from_value::<WorkspaceSymbolResponse>(val) {
            return match response {
                WorkspaceSymbolResponse::Flat(symbols) => symbols,
                WorkspaceSymbolResponse::Nested(symbols) => symbols
                    .into_iter()
                    .filter_map(workspace_symbol_to_info)
                    .collect(),
            };
        }
        tracing::debug!(
            server = %self.server_id,
            "workspace/symbol: failed to deserialize response"
        );
        Vec::new()
    }

    pub async fn implementation(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Option<GotoImplementationResponse> {
        let params = GotoImplementationParams {
            text_document_position_params: self.text_doc_pos(path, line, character)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let val = self.query("textDocument/implementation", params).await?;
        self.deserialize_response("textDocument/implementation", val)
    }

    pub async fn prepare_call_hierarchy(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Vec<CallHierarchyItem> {
        let Some(tdp) = self.text_doc_pos(path, line, character) else {
            return Vec::new();
        };
        let params = CallHierarchyPrepareParams {
            text_document_position_params: tdp,
            work_done_progress_params: Default::default(),
        };
        let val = match self
            .query("textDocument/prepareCallHierarchy", params)
            .await
        {
            Some(v) => v,
            None => return Vec::new(),
        };
        self.deserialize_or_default("textDocument/prepareCallHierarchy", val)
    }

    pub async fn incoming_calls(&self, item: CallHierarchyItem) -> Vec<CallHierarchyIncomingCall> {
        let params = CallHierarchyIncomingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let val = match self.query("callHierarchy/incomingCalls", params).await {
            Some(v) => v,
            None => return Vec::new(),
        };
        self.deserialize_or_default("callHierarchy/incomingCalls", val)
    }

    pub async fn outgoing_calls(&self, item: CallHierarchyItem) -> Vec<CallHierarchyOutgoingCall> {
        let params = CallHierarchyOutgoingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let val = match self.query("callHierarchy/outgoingCalls", params).await {
            Some(v) => v,
            None => return Vec::new(),
        };
        self.deserialize_or_default("callHierarchy/outgoingCalls", val)
    }

    pub async fn prepare_rename(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Option<PrepareRenameResponse> {
        let params = self.text_doc_pos(path, line, character)?;
        let val = self.query("textDocument/prepareRename", params).await?;
        if val.is_null() {
            return None;
        }
        self.deserialize_response("textDocument/prepareRename", val)
    }

    pub async fn rename(
        &self,
        path: &Path,
        line: u32,
        character: u32,
        new_name: &str,
    ) -> Option<WorkspaceEdit> {
        let params = RenameParams {
            text_document_position: self.text_doc_pos(path, line, character)?,
            new_name: new_name.to_string(),
            work_done_progress_params: Default::default(),
        };
        let val = self.query("textDocument/rename", params).await?;
        if val.is_null() {
            return None;
        }
        self.deserialize_response("textDocument/rename", val)
    }

    pub async fn code_action(
        &self,
        path: &Path,
        range: Range,
        only: Option<Vec<CodeActionKind>>,
        diagnostics: Vec<Diagnostic>,
    ) -> Vec<CodeActionOrCommand> {
        let Some(uri) = uri_from_path(path) else {
            return Vec::new();
        };

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier { uri },
            range,
            context: CodeActionContext {
                diagnostics,
                only,
                trigger_kind: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let val = match self.query("textDocument/codeAction", params).await {
            Some(v) => v,
            None => return Vec::new(),
        };
        if val.is_null() {
            return Vec::new();
        }
        self.deserialize_or_default("textDocument/codeAction", val)
    }

    /// Resolve a code action to populate its `edit` field via `codeAction/resolve`.
    pub async fn code_action_resolve(&self, action: CodeAction) -> Option<CodeAction> {
        let val = self.query("codeAction/resolve", action).await?;
        if val.is_null() {
            return None;
        }
        self.deserialize_response("codeAction/resolve", val)
    }

    // -----------------------------------------------------------------------
    // Shutdown
    // -----------------------------------------------------------------------

    /// Graceful shutdown: shutdown request (2s) → exit notification → kill.
    pub async fn shutdown(&self) {
        let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, self.send_request("shutdown", None)).await;
        let _ = self.send_notification("exit", None).await;
        if let Some(mut child) = self.child.lock().await.take() {
            let pid = child.id();
            tracing::info!(
                server = %self.server_id,
                pid = pid.unwrap_or_default(),
                root = self.root_uri.as_str(),
                "shutting down lsp server"
            );
            let _ = child.kill().await;
            #[cfg(unix)]
            if let Some(pid) = pid {
                unsafe {
                    libc::killpg(pid as i32, libc::SIGKILL);
                }
            }
            let _ = child.wait().await;
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn text_doc_pos(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Option<TextDocumentPositionParams> {
        let uri = uri_from_path(path)?;
        Some(TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: Position { line, character },
        })
    }

    async fn query<P: serde::Serialize>(&self, method: &str, params: P) -> Option<Value> {
        let value = serde_json::to_value(params).ok()?;
        match tokio::time::timeout(QUERY_TIMEOUT, self.send_request(method, Some(value))).await {
            Ok(Ok(result)) => Some(result),
            Ok(Err(e)) => {
                tracing::debug!(server = %self.server_id, method, "query error: {e}");
                None
            }
            Err(_) => {
                tracing::debug!(server = %self.server_id, method, "query timed out");
                None
            }
        }
    }

    fn deserialize_response<T: DeserializeOwned>(
        &self,
        method: &'static str,
        val: Value,
    ) -> Option<T> {
        match serde_json::from_value(val) {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                tracing::debug!(
                    server = %self.server_id,
                    method,
                    "failed to deserialize response payload: {error}"
                );
                None
            }
        }
    }

    fn deserialize_or_default<T: DeserializeOwned + Default>(
        &self,
        method: &'static str,
        val: Value,
    ) -> T {
        match serde_json::from_value(val) {
            Ok(parsed) => parsed,
            Err(error) => {
                tracing::debug!(
                    server = %self.server_id,
                    method,
                    "failed to deserialize response payload: {error}"
                );
                T::default()
            }
        }
    }

    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value, LspError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: JsonRpcId::Number(id),
            method: method.to_string(),
            params,
        };

        self.trace_event("request", method, id, request.params.as_ref(), None)
            .await;

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let body = serde_json::to_vec(&request)?;
        self.write_message(body).await?;

        let response = match rx.await {
            Ok(response) => response,
            Err(_) => {
                return Err(self
                    .server_exited_error(format!("request `{method}` did not receive a response"))
                    .await);
            }
        };

        if let Some(err) = response.error {
            self.trace_event(
                "response",
                method,
                id,
                None,
                Some(serde_json::json!({ "error": err })),
            )
            .await;
            return Err(LspError::ServerError {
                code: err.code,
                message: err.message,
            });
        }

        let result = response.result.unwrap_or(Value::Null);
        if self.trace.is_some() {
            self.trace_event("response", method, id, None, Some(result.clone()))
                .await;
        }
        Ok(result)
    }

    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<(), LspError> {
        self.trace_event("notification", method, 0, params.as_ref(), None)
            .await;
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: method.to_string(),
            params,
        };
        let body = serde_json::to_vec(&notification)?;
        self.write_message(body).await
    }

    async fn trace_event(
        &self,
        kind: &'static str,
        method: &str,
        id: i64,
        params: Option<&Value>,
        result: Option<Value>,
    ) {
        let Some(trace) = &self.trace else {
            return;
        };

        let redacted_params = params.map(|v| redact_params(method, v));
        let record = serde_json::json!({
            "kind": kind,
            "server": self.server_id,
            "method": method,
            "id": if id == 0 { Value::Null } else { Value::from(id) },
            "params": redacted_params,
            "result": result,
        });

        let mut line = match serde_json::to_string(&record) {
            Ok(s) => s,
            Err(_) => return,
        };
        line.push('\n');

        if line.len() > 1024 * 1024 {
            return;
        }

        let mut file = trace.lock().await;
        let _ = file.write_all(line.as_bytes()).await;
        let _ = file.flush().await;
    }

    async fn write_message(&self, body: Vec<u8>) -> Result<(), LspError> {
        write_framed_message(&self.writer, &body).await
    }

    /// Background task that reads messages from the server and routes them.
    async fn reader_loop(
        framed: FramedRead<BufReader<tokio::process::ChildStdout>, LspCodec>,
        context: ReaderContext,
    ) {
        tokio::pin!(framed);

        while let Some(result) = framed.next().await {
            let msg = match result {
                Ok(msg) => msg,
                Err(e) => {
                    tracing::debug!(server = %context.server_id, "reader error: {e}");
                    break;
                }
            };

            match msg {
                JsonRpcMessage::Response(resp) => {
                    if let Some(id) = resp.id.as_ref().and_then(JsonRpcId::as_i64)
                        && let Some(tx) = context.pending.lock().await.remove(&id)
                    {
                        let _ = tx.send(resp);
                    }
                }
                JsonRpcMessage::Notification(notif) => {
                    if notif.method == "textDocument/publishDiagnostics"
                        && let Some(params) = notif.params
                        && let Ok(diag_params) =
                            serde_json::from_value::<PublishDiagnosticsParams>(params)
                    {
                        let uri_key = diag_params.uri.as_str().to_string();
                        context
                            .diagnostics
                            .write()
                            .await
                            .insert(uri_key.clone(), diag_params.diagnostics);
                        let _ = context.diag_tx.send(uri_key);
                    }
                }
                JsonRpcMessage::Request(req) => {
                    let response_result = match req.method.as_str() {
                        "window/workDoneProgress/create"
                        | "client/registerCapability"
                        | "client/unregisterCapability" => Some(Value::Null),
                        "workspace/configuration" => {
                            let items = req
                                .params
                                .as_ref()
                                .and_then(|p| p.get("items"))
                                .and_then(|i| i.as_array());
                            let settings = context
                                .initialization_options
                                .clone()
                                .unwrap_or(serde_json::json!({}));
                            let results: Vec<Value> = match items {
                                Some(items) => items
                                    .iter()
                                    .map(|item| {
                                        let section = item
                                            .get("section")
                                            .and_then(|s| s.as_str())
                                            .unwrap_or("");
                                        if section.is_empty() {
                                            settings.clone()
                                        } else {
                                            // Walk dot-separated path: "python.analysis" → settings["python"]["analysis"]
                                            section.split('.').fold(settings.clone(), |acc, key| {
                                                acc.get(key)
                                                    .cloned()
                                                    .unwrap_or(serde_json::json!({}))
                                            })
                                        }
                                    })
                                    .collect(),
                                None => vec![settings],
                            };
                            Some(Value::Array(results))
                        }
                        "workspace/workspaceFolders" => Some(serde_json::json!([{
                            "uri": context.root_uri.as_str(),
                            "name": "workspace"
                        }])),
                        _ => {
                            tracing::debug!(
                                server = %context.server_id,
                                method = %req.method,
                                "unhandled server request; replying with MethodNotFound"
                            );
                            None
                        }
                    };

                    let resp = if let Some(result) = response_result {
                        JsonRpcResponse {
                            jsonrpc: "2.0".into(),
                            id: Some(req.id),
                            result: Some(result),
                            error: None,
                        }
                    } else {
                        JsonRpcResponse {
                            jsonrpc: "2.0".into(),
                            id: Some(req.id),
                            result: None,
                            error: Some(JsonRpcError {
                                code: -32601,
                                message: format!("method not found: {}", req.method),
                                data: None,
                            }),
                        }
                    };
                    if let Ok(body) = serde_json::to_vec(&resp)
                        && let Err(error) = write_framed_message(&context.writer, &body).await
                    {
                        tracing::debug!(
                            server = %context.server_id,
                            "failed to write server-request response: {error}"
                        );
                        break;
                    }
                }
            }
        }

        // Server exited — resolve all pending requests with errors.
        let mut map = context.pending.lock().await;
        for (_, tx) in map.drain() {
            let _ = tx.send(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: None,
                result: None,
                error: Some(JsonRpcError {
                    code: -1,
                    message: "server exited".into(),
                    data: None,
                }),
            });
        }
    }
}

#[allow(deprecated)]
fn workspace_symbol_to_info(sym: WorkspaceSymbol) -> Option<SymbolInformation> {
    let location = match sym.location {
        OneOf::Left(loc) => loc,
        OneOf::Right(ws_loc) => Location {
            uri: ws_loc.uri,
            range: Range::default(),
        },
    };
    Some(SymbolInformation {
        name: sym.name,
        kind: sym.kind,
        tags: sym.tags,
        container_name: sym.container_name,
        location,
        deprecated: None,
    })
}

async fn write_framed_message(
    writer: &Arc<Mutex<ChildStdin>>,
    body: &[u8],
) -> Result<(), LspError> {
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut writer = writer.lock().await;
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(body).await?;
    writer.flush().await?;
    Ok(())
}

async fn open_trace_file() -> Option<Arc<Mutex<tokio::fs::File>>> {
    let path = std::env::var("CODEX_LSP_TRACE_JSONL").ok()?;
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let path = PathBuf::from(path);
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .ok()?;
    Some(Arc::new(Mutex::new(file)))
}

fn redact_params(method: &str, params: &Value) -> Value {
    // Best-effort redaction to avoid logging entire file contents.
    if method == "textDocument/didOpen" {
        let mut v = params.clone();
        if let Some(obj) = v.as_object_mut()
            && let Some(td) = obj.get_mut("textDocument")
            && let Some(td_obj) = td.as_object_mut()
        {
            td_obj.remove("text");
        }
        return v;
    }

    if method == "textDocument/didChange" {
        let mut v = params.clone();
        if let Some(obj) = v.as_object_mut()
            && let Some(changes) = obj.get_mut("contentChanges")
            && let Some(arr) = changes.as_array_mut()
        {
            for item in arr.iter_mut() {
                if let Some(change_obj) = item.as_object_mut() {
                    change_obj.remove("text");
                }
            }
        }
        return v;
    }

    params.clone()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    #[expect(clippy::expect_used)]
    fn write_executable(dir: &tempfile::TempDir, name: &str, contents: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        let mut file = fs::File::create(&path).expect("create temporary executable file");
        file.write_all(contents.as_bytes())
            .expect("write executable contents");
        let mut permissions = file.metadata().expect("read file metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("set executable permissions");
        path
    }

    #[tokio::test]
    async fn create_captures_stderr_for_initialize_exit() {
        let root = tempfile::TempDir::new().unwrap();
        let bin_dir = tempfile::TempDir::new().unwrap();
        let script = "#!/bin/sh\n\
echo 'captured initialize stderr' >&2\n\
sleep 0.05\n\
exit 1\n";
        let binary = write_executable(&bin_dir, "fake-lsp", script);

        let config = LspServerConfig::new(
            vec![".txt".into()],
            vec![binary.to_string_lossy().to_string()],
            vec![],
        );

        let result = LspClient::create("fake-lsp", &config, root.path()).await;
        assert!(result.is_err(), "fake server should fail during initialize");
        let err = result.err().unwrap();
        match err {
            LspError::ProcessExitedImmediately { stderr, .. } => {
                assert!(
                    stderr.contains("captured initialize stderr"),
                    "stderr should include the server's stderr tail: {stderr}"
                );
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
