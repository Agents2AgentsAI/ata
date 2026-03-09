use super::*;

pub(super) fn constrain_mcp_servers(
    mcp_servers: HashMap<String, McpServerConfig>,
    mcp_requirements: Option<&Sourced<BTreeMap<String, McpServerRequirement>>>,
) -> ConstraintResult<Constrained<HashMap<String, McpServerConfig>>> {
    if mcp_requirements.is_none() {
        return Ok(Constrained::allow_any(mcp_servers));
    }

    let mcp_requirements = mcp_requirements.cloned();
    Constrained::normalized(mcp_servers, move |mut servers| {
        filter_mcp_servers_by_requirements(&mut servers, mcp_requirements.as_ref());
        servers
    })
}

pub async fn load_global_mcp_servers(
    codex_home: &Path,
) -> std::io::Result<BTreeMap<String, McpServerConfig>> {
    let cli_overrides = Vec::<(String, TomlValue)>::new();
    let cwd: Option<AbsolutePathBuf> = None;
    let config_layer_stack = load_config_layers_state(
        codex_home,
        cwd,
        &cli_overrides,
        LoaderOverrides::default(),
        CloudRequirementsLoader::default(),
    )
    .await?;
    let merged_toml = config_layer_stack.effective_config();
    let Some(servers_value) = merged_toml.get("mcp_servers") else {
        return Ok(BTreeMap::new());
    };

    ensure_no_inline_bearer_tokens(servers_value)?;

    servers_value
        .clone()
        .try_into()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub(crate) fn filter_mcp_servers_by_requirements(
    mcp_servers: &mut HashMap<String, McpServerConfig>,
    mcp_requirements: Option<&Sourced<BTreeMap<String, McpServerRequirement>>>,
) {
    let Some(allowlist) = mcp_requirements else {
        return;
    };

    let source = allowlist.source.clone();
    for (name, server) in mcp_servers.iter_mut() {
        let allowed = allowlist
            .value
            .get(name)
            .is_some_and(|requirement| mcp_server_matches_requirement(requirement, server));
        if allowed {
            server.disabled_reason = None;
        } else {
            server.enabled = false;
            server.disabled_reason = Some(McpServerDisabledReason::Requirements {
                source: source.clone(),
            });
        }
    }
}

fn mcp_server_matches_requirement(
    requirement: &McpServerRequirement,
    server: &McpServerConfig,
) -> bool {
    match &requirement.identity {
        McpServerIdentity::Command {
            command: want_command,
        } => matches!(
            &server.transport,
            McpServerTransportConfig::Stdio { command: got_command, .. }
                if got_command == want_command
        ),
        McpServerIdentity::Url { url: want_url } => matches!(
            &server.transport,
            McpServerTransportConfig::StreamableHttp { url: got_url, .. }
                if got_url == want_url
        ),
    }
}

fn ensure_no_inline_bearer_tokens(value: &TomlValue) -> std::io::Result<()> {
    let Some(servers_table) = value.as_table() else {
        return Ok(());
    };

    for (server_name, server_value) in servers_table {
        if let Some(server_table) = server_value.as_table()
            && server_table.contains_key("bearer_token")
        {
            let message = format!(
                "mcp_servers.{server_name} uses unsupported `bearer_token`; set `bearer_token_env_var`."
            );
            return Err(std::io::Error::new(ErrorKind::InvalidData, message));
        }
    }

    Ok(())
}
