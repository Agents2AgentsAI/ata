use super::*;

pub(super) fn create_search_tool_bm25_tool(app_tools: &HashMap<String, ToolInfo>) -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "query".to_string(),
            JsonSchema::String {
                description: Some("Search query for apps tools.".to_string()),
            },
        ),
        (
            "limit".to_string(),
            JsonSchema::Number {
                description: Some(format!(
                    "Maximum number of tools to return (defaults to {SEARCH_TOOL_BM25_DEFAULT_LIMIT})."
                )),
            },
        ),
    ]);
    let mut app_names = app_tools
        .values()
        .filter_map(|tool| tool.connector_name.clone())
        .collect::<Vec<_>>();
    app_names.sort();
    app_names.dedup();
    let app_names = app_names.join(", ");

    let description =
        SEARCH_TOOL_BM25_DESCRIPTION_TEMPLATE.replace("{{app_names}}", app_names.as_str());

    ToolSpec::Function(ResponsesApiTool {
        name: SEARCH_TOOL_BM25_TOOL_NAME.to_string(),
        description,
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["query".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

pub(super) fn create_list_mcp_resources_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "server".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional MCP server name. When omitted, lists resources from every configured server."
                        .to_string(),
                ),
            },
        ),
        (
            "cursor".to_string(),
            JsonSchema::String {
                description: Some(
                    "Opaque cursor returned by a previous list_mcp_resources call for the same server."
                        .to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "list_mcp_resources".to_string(),
        description: "Lists resources provided by MCP servers. Resources allow servers to share data that provides context to language models, such as files, database schemas, or application-specific information. Prefer resources over web search when possible.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
    })
}

pub(super) fn create_list_mcp_resource_templates_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "server".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional MCP server name. When omitted, lists resource templates from all configured servers."
                        .to_string(),
                ),
            },
        ),
        (
            "cursor".to_string(),
            JsonSchema::String {
                description: Some(
                    "Opaque cursor returned by a previous list_mcp_resource_templates call for the same server."
                        .to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "list_mcp_resource_templates".to_string(),
        description: "Lists resource templates provided by MCP servers. Parameterized resource templates allow servers to share data that takes parameters and provides context to language models, such as files, database schemas, or application-specific information. Prefer resource templates over web search when possible.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
    })
}

pub(super) fn create_read_mcp_resource_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "server".to_string(),
            JsonSchema::String {
                description: Some(
                    "IMPORTANT: Do NOT guess server names like 'filesystem' - always use the exact server name from a previous list_mcp_resources call. Must match the 'server' field returned by list_mcp_resources."
                        .to_string(),
                ),
            },
        ),
        (
            "uri".to_string(),
            JsonSchema::String {
                description: Some(
                    "IMPORTANT: Do NOT construct URIs from file paths - only use URIs returned by list_mcp_resources. Resource URI to read."
                        .to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "read_mcp_resource".to_string(),
        description:
            "IMPORTANT: Always call list_mcp_resources first to discover available servers and URIs. Read a specific resource from an MCP server given the server name and resource URI."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["server".to_string(), "uri".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}
