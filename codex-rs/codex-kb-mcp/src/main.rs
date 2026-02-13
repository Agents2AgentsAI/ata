use std::borrow::Cow;
use std::sync::Arc;

use clap::Parser;
use codex_kb::KnowledgeBase;
use codex_kb::card::CardFigure;
use codex_kb::tool_specs::ToolDef;
use codex_kb::tool_specs::all_tool_defs;
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
#[command(name = "codex-kb-mcp")]
struct Cli {
    /// Path to the knowledge base directory.
    #[arg(long, default_value = "./knowledge-base")]
    kb_path: String,
}

#[derive(Clone)]
struct KbMcpServer {
    kb: Arc<KnowledgeBase>,
    tool_defs: Arc<Vec<ToolDef>>,
    tools: Arc<Vec<Tool>>,
}

impl KbMcpServer {
    fn new(kb: Arc<KnowledgeBase>) -> anyhow::Result<Self> {
        let tool_defs = all_tool_defs();
        let tools = tool_defs
            .iter()
            .map(to_mcp_tool)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            kb,
            tool_defs: Arc::new(tool_defs),
            tools: Arc::new(tools),
        })
    }
}

impl ServerHandler for KbMcpServer {
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
            "kb_write_card" => {
                let params: WriteCardArgs = parse_arguments(request.arguments)?;
                self.kb
                    .write_card(
                        params.id,
                        params.title,
                        params.tags,
                        params.capsule,
                        params.source_type,
                        params.source_refs,
                        params.status,
                        params.tensions,
                        params.supersedes,
                        params.figures,
                        params.contributed_by,
                        params.body,
                    )
                    .await
                    .map(serialize_tool_result)
            }
            "kb_write_overview" => {
                let params: WriteOverviewArgs = parse_arguments(request.arguments)?;
                self.kb
                    .write_overview(&params.topic, &params.content)
                    .await
                    .map(|()| serde_json::json!({"ok": true}))
            }
            "kb_suggest_tags" => {
                let params: SuggestTagsArgs = parse_arguments(request.arguments)?;
                self.kb
                    .suggest_tags(&params.title, params.capsule.as_deref())
                    .await
                    .map(serialize_tool_result)
            }
            "kb_status" => self.kb.status().await.map(serialize_tool_result),
            "kb_search" => {
                let params: SearchArgs = parse_arguments(request.arguments)?;
                self.kb
                    .search(
                        &params.query,
                        params.tag.as_deref(),
                        params.status.as_deref(),
                        params.limit.map(|l| l as usize),
                    )
                    .await
                    .map(serialize_tool_result)
            }
            "kb_read_card" => {
                let params: ReadCardArgs = parse_arguments(request.arguments)?;
                self.kb
                    .read_card(&params.id)
                    .await
                    .map(serialize_tool_result)
            }
            "kb_delete_card" => {
                let params: DeleteCardArgs = parse_arguments(request.arguments)?;
                self.kb
                    .delete_card(&params.id)
                    .await
                    .map(serialize_tool_result)
            }
            "kb_write_file" => {
                let params: WriteFileArgs = parse_arguments(request.arguments)?;
                self.kb
                    .write_file(&params.path, &params.content)
                    .await
                    .map(serialize_tool_result)
            }
            "kb_read_file" => {
                let params: ReadFileArgs = parse_arguments(request.arguments)?;
                self.kb
                    .read_file(&params.path)
                    .await
                    .map(serialize_tool_result)
            }
            "kb_reset" => {
                let params: ResetArgs = parse_arguments(request.arguments)?;
                if !params.confirm {
                    return Err(McpError::invalid_params(
                        "confirm must be true to reset the knowledge base".to_string(),
                        None,
                    ));
                }
                self.kb.reset().await.map(serialize_tool_result)
            }
            "kb_list_cards" => {
                let params: ListCardsArgs = parse_arguments(request.arguments)?;
                self.kb
                    .list_cards(
                        params.tag.as_deref(),
                        params.status.as_deref(),
                        params.limit.map(|l| l as usize),
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
struct WriteCardArgs {
    id: String,
    title: String,
    tags: Vec<String>,
    #[serde(default)]
    capsule: Option<String>,
    #[serde(default)]
    source_type: Option<String>,
    #[serde(default)]
    source_refs: Option<Vec<String>>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    tensions: Option<Vec<String>>,
    #[serde(default)]
    supersedes: Option<Vec<String>>,
    #[serde(default)]
    figures: Option<Vec<CardFigure>>,
    #[serde(default)]
    contributed_by: Option<String>,
    body: String,
}

#[derive(Debug, Deserialize)]
struct WriteOverviewArgs {
    topic: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct SuggestTagsArgs {
    title: String,
    #[serde(default)]
    capsule: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ReadCardArgs {
    id: String,
}

#[derive(Debug, Deserialize)]
struct DeleteCardArgs {
    id: String,
}

#[derive(Debug, Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ReadFileArgs {
    path: String,
}

#[derive(Debug, Deserialize)]
struct ResetArgs {
    confirm: bool,
}

#[derive(Debug, Deserialize)]
struct ListCardsArgs {
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let kb_path = std::path::PathBuf::from(&cli.kb_path);
    let kb = Arc::new(KnowledgeBase::new(kb_path));
    kb.init()?;

    let server = KbMcpServer::new(kb)?;
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
