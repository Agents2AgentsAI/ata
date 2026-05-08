use super::*;

pub(super) async fn initialize_mcp_connection_manager(
    sess: &Arc<Session>,
    config: &Config,
    session_configuration: &SessionConfiguration,
    mcp_servers: &HashMap<String, McpServerConfig>,
    auth_statuses: HashMap<String, crate::mcp::auth::McpAuthStatusEntry>,
    tx_event: Sender<Event>,
    auth: Option<&CodexAuth>,
) -> anyhow::Result<()> {
    let sandbox_state = SandboxState {
        sandbox_policy: session_configuration.sandbox_policy.get().clone(),
        codex_linux_sandbox_exe: config.codex_linux_sandbox_exe.clone(),
        sandbox_cwd: session_configuration.cwd.clone(),
        use_linux_sandbox_bwrap: config.features.enabled(Feature::UseLinuxSandboxBwrap),
    };
    let mut required_mcp_servers: Vec<String> = mcp_servers
        .iter()
        .filter(|(_, server)| server.enabled && server.required)
        .map(|(name, _)| name.clone())
        .collect();
    required_mcp_servers.sort();

    {
        let mut cancel_guard = sess.services.mcp_startup_cancellation_token.lock().await;
        cancel_guard.cancel();
        *cancel_guard = CancellationToken::new();
    }

    let (mcp_connection_manager, cancel_token) = McpConnectionManager::new(
        mcp_servers,
        config.mcp_oauth_credentials_store_mode,
        auth_statuses,
        &session_configuration.approval_policy,
        tx_event,
        sandbox_state,
        config.codex_home.clone(),
        codex_apps_tools_cache_key(auth),
    )
    .await;

    {
        let mut manager_guard = sess.services.mcp_connection_manager.write().await;
        *manager_guard = mcp_connection_manager;
    }
    {
        let mut cancel_guard = sess.services.mcp_startup_cancellation_token.lock().await;
        if cancel_guard.is_cancelled() {
            cancel_token.cancel();
        }
        *cancel_guard = cancel_token;
    }

    if required_mcp_servers.is_empty() {
        return Ok(());
    }

    let failures = sess
        .services
        .mcp_connection_manager
        .read()
        .await
        .required_startup_failures(&required_mcp_servers)
        .await;
    if failures.is_empty() {
        return Ok(());
    }

    let details = failures
        .iter()
        .map(|failure| format!("{}: {}", failure.server, failure.error))
        .collect::<Vec<_>>()
        .join("; ");
    Err(anyhow::anyhow!(
        "required MCP servers failed to initialize: {details}"
    ))
}
