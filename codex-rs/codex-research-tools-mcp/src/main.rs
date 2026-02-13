use std::borrow::Cow;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use codex_research_tools::ResearchToolkit;
use codex_research_tools::config::ResearchConfig;
use codex_research_tools::tool_specs::ToolDef;
use codex_research_tools::tool_specs::all_tool_defs;
use codex_research_tools::types::PaginationParams;
use codex_research_tools::types::PaperSearchParams;
use codex_research_tools::types::ZoteroAdvancedSearchParams;
use codex_research_tools::types::ZoteroAnnotationsParams;
use codex_research_tools::types::ZoteroCitationParams;
use codex_research_tools::types::ZoteroCollectionItemsParams;
use codex_research_tools::types::ZoteroCollectionsParams;
use codex_research_tools::types::ZoteroGrepParams;
use codex_research_tools::types::ZoteroItemParams;
use codex_research_tools::types::ZoteroListGroupsParams;
use codex_research_tools::types::ZoteroRecentParams;
use codex_research_tools::types::ZoteroSearchNotesParams;
use codex_research_tools::types::ZoteroSearchParams;
use codex_research_tools::types::ZoteroTagsParams;
use rmcp::ErrorData as McpError;
use rmcp::ServiceExt;
use rmcp::handler::server::ServerHandler;
use rmcp::model::CallToolRequestParams;
use rmcp::model::CallToolResult;
use rmcp::model::JsonObject;
use rmcp::model::ListToolsResult;
use rmcp::model::PaginatedRequestParams;
use rmcp::model::ServerCapabilities;
use rmcp::model::ServerInfo;
use rmcp::model::Tool;
use serde::Deserialize;
use serde::de::DeserializeOwned;

#[derive(Debug, Parser)]
#[command(name = "codex-research-tools-mcp")]
struct Cli {
    /// Comma-separated tool groups to expose: all,paper_search,zotero,repo_analysis
    #[arg(long, value_delimiter = ',', default_value = "all")]
    tools: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct ToolGroups {
    paper_search: bool,
    zotero: bool,
    repo_analysis: bool,
}

impl ToolGroups {
    fn from_labels(labels: &[String]) -> anyhow::Result<Self> {
        if labels.is_empty() || labels.iter().any(|label| label.eq_ignore_ascii_case("all")) {
            return Ok(Self {
                paper_search: true,
                zotero: true,
                repo_analysis: true,
            });
        }

        let mut groups = Self {
            paper_search: false,
            zotero: false,
            repo_analysis: false,
        };

        for label in labels {
            let normalized = label.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "paper_search" | "paper" => groups.paper_search = true,
                "zotero" => groups.zotero = true,
                "repo_analysis" | "repo" => groups.repo_analysis = true,
                _ => {
                    anyhow::bail!(
                        "unsupported tool group '{label}'; expected all|paper_search|zotero|repo_analysis"
                    );
                }
            }
        }

        Ok(groups)
    }

    fn allows(self, tool_id: &str) -> bool {
        if tool_id.starts_with("paper_") {
            return self.paper_search;
        }
        if tool_id.starts_with("zotero_") {
            return self.zotero;
        }
        if tool_id.starts_with("repo_") {
            return self.repo_analysis;
        }

        false
    }
}

#[derive(Clone)]
struct ResearchMcpServer {
    toolkit: Arc<ResearchToolkit>,
    tool_defs: Arc<Vec<ToolDef>>,
    tools: Arc<Vec<Tool>>,
}

impl ResearchMcpServer {
    fn new(toolkit: Arc<ResearchToolkit>, groups: ToolGroups) -> anyhow::Result<Self> {
        let tool_defs = all_tool_defs()
            .into_iter()
            .filter(|def| groups.allows(def.id))
            .collect::<Vec<_>>();

        let tools = tool_defs
            .iter()
            .map(to_mcp_tool)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            toolkit,
            tool_defs: Arc::new(tool_defs),
            tools: Arc::new(tools),
        })
    }
}

impl ServerHandler for ResearchMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..ServerInfo::default()
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let tools = Arc::clone(&self.tools);
        async move {
            Ok(ListToolsResult {
                tools: (*tools).clone(),
                next_cursor: None,
                meta: None,
            })
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool_name = request.name.to_string();
        if !self
            .tool_defs
            .iter()
            .any(|def| def.mcp_name == tool_name.as_str())
        {
            return Err(McpError::invalid_params(
                format!("unknown tool: {tool_name}"),
                None,
            ));
        }

        let result = match tool_name.as_str() {
            "search_papers" => {
                let params: PaperSearchParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .paper_search(params)
                    .await
                    .map(serialize_tool_result)
            }
            "get_paper" => {
                let params: PaperIdArgs = parse_arguments(request.arguments)?;
                self.toolkit
                    .paper_get(params.paper_id.as_str())
                    .await
                    .map(serialize_tool_result)
            }
            "get_citations" => {
                let params: PaperPaginationArgs = parse_arguments(request.arguments)?;
                self.toolkit
                    .paper_citations(
                        params.paper_id.as_str(),
                        PaginationParams {
                            offset: params.offset,
                            limit: params.limit,
                            fields: params.fields,
                            max_chars_per_item: params.max_chars_per_item,
                        },
                    )
                    .await
                    .map(serialize_tool_result)
            }
            "get_references" => {
                let params: PaperPaginationArgs = parse_arguments(request.arguments)?;
                self.toolkit
                    .paper_references(
                        params.paper_id.as_str(),
                        PaginationParams {
                            offset: params.offset,
                            limit: params.limit,
                            fields: params.fields,
                            max_chars_per_item: params.max_chars_per_item,
                        },
                    )
                    .await
                    .map(serialize_tool_result)
            }
            "search_library" => {
                let params: ZoteroSearchParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_search(params)
                    .await
                    .map(serialize_tool_result)
            }
            "get_tags" => {
                let params: ZoteroTagsParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_get_tags(params)
                    .await
                    .map(serialize_tool_result)
            }
            "get_recent" => {
                let params: ZoteroRecentParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_get_recent(params)
                    .await
                    .map(serialize_tool_result)
            }
            "advanced_search" => {
                let params: ZoteroAdvancedSearchParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_advanced_search(params)
                    .await
                    .map(serialize_tool_result)
            }
            "grep_library_text" => {
                let params: ZoteroGrepParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_grep_text(params)
                    .await
                    .map(serialize_tool_result)
            }
            "search_notes" => {
                let params: ZoteroSearchNotesParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_search_notes(params)
                    .await
                    .map(serialize_tool_result)
            }
            "get_item_details" => {
                let params: ZoteroItemParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_get_item(params)
                    .await
                    .map(serialize_tool_result)
            }
            "get_item_citation" => {
                let params: ZoteroCitationParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_get_item_citation(params)
                    .await
                    .map(serialize_tool_result)
            }
            "get_item_fulltext" => {
                let params: ZoteroItemParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_get_fulltext(params)
                    .await
                    .map(serialize_tool_result)
            }
            "get_item_notes" => {
                let params: ZoteroItemParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_get_notes(params)
                    .await
                    .map(serialize_tool_result)
            }
            "get_annotations" => {
                let params: ZoteroAnnotationsParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_get_annotations(params)
                    .await
                    .map(serialize_tool_result)
            }
            "get_item_attachments" => {
                let params: ZoteroItemParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_get_attachments(params)
                    .await
                    .map(serialize_tool_result)
            }
            "get_collections" => {
                let params: ZoteroCollectionsParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_get_collections(params)
                    .await
                    .map(serialize_tool_result)
            }
            "list_groups" => {
                let params: ZoteroListGroupsParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_list_groups(params)
                    .await
                    .map(serialize_tool_result)
            }
            "get_collection_items" => {
                let params: ZoteroCollectionItemsParams = parse_arguments(request.arguments)?;
                self.toolkit
                    .zotero_get_collection_items(params)
                    .await
                    .map(serialize_tool_result)
            }
            "clone_and_summarize" => {
                let params: RepoCloneArgs = parse_arguments(request.arguments)?;
                self.toolkit
                    .repo_clone_and_summarize(params.repo_url.as_str(), params.branch.as_deref())
                    .await
                    .map(serialize_tool_result)
            }
            "find_model_definitions" => {
                let params: RepoFindModelsArgs = parse_arguments(request.arguments)?;
                self.toolkit
                    .repo_find_models(params.repo_url.as_str(), params.framework.as_deref())
                    .await
                    .map(serialize_tool_result)
            }
            "extract_requirements" => {
                let params: RepoUrlArgs = parse_arguments(request.arguments)?;
                self.toolkit
                    .repo_extract_requirements(params.repo_url.as_str())
                    .await
                    .map(serialize_tool_result)
            }
            "find_entrypoints" => {
                let params: RepoFindEntrypointsArgs = parse_arguments(request.arguments)?;
                self.toolkit
                    .repo_find_entrypoints(params.repo_url.as_str(), params.task_hint.as_deref())
                    .await
                    .map(serialize_tool_result)
            }
            "extract_io_shapes" => {
                let params: RepoExtractIoShapesArgs = parse_arguments(request.arguments)?;
                self.toolkit
                    .repo_extract_io_shapes(params.repo_url.as_str(), params.model_class.as_deref())
                    .await
                    .map(serialize_tool_result)
            }
            "get_repo_health" => {
                let params: RepoUrlArgs = parse_arguments(request.arguments)?;
                self.toolkit
                    .repo_get_health(params.repo_url.as_str())
                    .await
                    .map(serialize_tool_result)
            }
            "find_export_paths" => {
                let params: RepoUrlArgs = parse_arguments(request.arguments)?;
                self.toolkit
                    .repo_find_export_paths(params.repo_url.as_str())
                    .await
                    .map(serialize_tool_result)
            }
            "extract_config_schema" => {
                let params: RepoUrlArgs = parse_arguments(request.arguments)?;
                self.toolkit
                    .repo_extract_config_schema(params.repo_url.as_str())
                    .await
                    .map(serialize_tool_result)
            }
            "diff_requirements" => {
                let params: RepoDiffRequirementsArgs = parse_arguments(request.arguments)?;
                self.toolkit
                    .repo_diff_requirements(
                        params.repo_url.as_str(),
                        params.local_requirements_path.as_str(),
                    )
                    .await
                    .map(serialize_tool_result)
            }
            _ => {
                return Err(McpError::invalid_params(
                    format!("unknown tool: {tool_name}"),
                    None,
                ));
            }
        };

        match result {
            Ok(value) => Ok(CallToolResult {
                content: Vec::new(),
                structured_content: Some(value),
                is_error: Some(false),
                meta: None,
            }),
            Err(err) => Ok(CallToolResult {
                content: Vec::new(),
                structured_content: Some(serde_json::json!({
                    "error": err.to_string(),
                    "retryable": err.is_retryable(),
                })),
                is_error: Some(true),
                meta: None,
            }),
        }
    }
}

fn serialize_tool_result<T: serde::Serialize>(value: T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or_else(|err| {
        serde_json::json!({
            "error": format!("failed to serialize tool result: {err}")
        })
    })
}

fn parse_arguments<T: DeserializeOwned>(arguments: Option<JsonObject>) -> Result<T, McpError> {
    let object = arguments.unwrap_or_default();
    let value = serde_json::Value::Object(object.into_iter().collect());
    serde_json::from_value(value).map_err(|err| McpError::invalid_params(err.to_string(), None))
}

fn to_mcp_tool(def: &ToolDef) -> Result<Tool, McpError> {
    let input_schema: JsonObject = serde_json::from_value(def.input_schema.clone())
        .map_err(|err| McpError::internal_error(err.to_string(), None))?;

    Ok(Tool::new(
        Cow::Borrowed(def.mcp_name),
        Cow::Borrowed(def.description),
        Arc::new(input_schema),
    ))
}

fn stdio() -> (tokio::io::Stdin, tokio::io::Stdout) {
    (tokio::io::stdin(), tokio::io::stdout())
}

#[derive(Debug, Deserialize)]
struct PaperIdArgs {
    paper_id: String,
}

#[derive(Debug, Deserialize)]
struct PaperPaginationArgs {
    paper_id: String,
    offset: Option<u32>,
    limit: Option<u32>,
    fields: Option<Vec<String>>,
    max_chars_per_item: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RepoUrlArgs {
    repo_url: String,
}

#[derive(Debug, Deserialize)]
struct RepoCloneArgs {
    repo_url: String,
    branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RepoFindModelsArgs {
    repo_url: String,
    framework: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RepoFindEntrypointsArgs {
    repo_url: String,
    task_hint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RepoExtractIoShapesArgs {
    repo_url: String,
    model_class: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RepoDiffRequirementsArgs {
    repo_url: String,
    local_requirements_path: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let groups = ToolGroups::from_labels(cli.tools.as_slice())?;

    let config = ResearchConfig::from_env();
    let client = reqwest::Client::builder()
        .connect_timeout(config.connect_timeout)
        .timeout(config.request_timeout)
        .build()
        .context("failed to build reqwest client for research MCP server")?;

    let toolkit = Arc::new(ResearchToolkit::new(client, config));
    let server = ResearchMcpServer::new(toolkit, groups)?;
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
