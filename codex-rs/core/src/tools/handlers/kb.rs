use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use async_trait::async_trait;
use codex_kb::KnowledgeBase;
use codex_kb::card::CardFigure;
use codex_protocol::models::FunctionCallOutputBody;
use futures::FutureExt;
use serde::Deserialize;
use serde::Serialize;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub(crate) struct KbBridgeHandler {
    kb: Arc<KnowledgeBase>,
}

impl KbBridgeHandler {
    pub(crate) fn new(kb: Arc<KnowledgeBase>) -> Self {
        Self { kb }
    }

    async fn execute_native_tool(
        &self,
        tool_name: &str,
        arguments: &str,
    ) -> Result<ToolOutput, FunctionCallError> {
        let fut = self.dispatch_tool_call(tool_name, arguments);
        let guarded = AssertUnwindSafe(fut).catch_unwind().await;
        match guarded {
            Ok(result) => result,
            Err(payload) => {
                let panic_message = panic_payload_to_message(payload.as_ref());
                tracing::error!(
                    tool_name,
                    panic_message = %panic_message,
                    "kb tool panicked"
                );
                Err(FunctionCallError::RespondToModel(format!(
                    "Internal panic: {panic_message}"
                )))
            }
        }
    }

    async fn dispatch_tool_call(
        &self,
        tool_name: &str,
        arguments: &str,
    ) -> Result<ToolOutput, FunctionCallError> {
        match tool_name {
            "kb_write_card" => {
                let params: WriteCardArgs = parse_arguments(arguments)?;
                let output = self
                    .kb
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
                    .map_err(map_kb_error)?;
                serialize_tool_output(&output)
            }
            "kb_write_overview" => {
                let params: WriteOverviewArgs = parse_arguments(arguments)?;
                self.kb
                    .write_overview(&params.topic, &params.content)
                    .await
                    .map_err(map_kb_error)?;
                serialize_tool_output(&serde_json::json!({"ok": true, "result": ()}))
            }
            "kb_suggest_tags" => {
                let params: SuggestTagsArgs = parse_arguments(arguments)?;
                let output = self
                    .kb
                    .suggest_tags(&params.title, params.capsule.as_deref())
                    .await
                    .map_err(map_kb_error)?;
                serialize_tool_output(&output)
            }
            "kb_status" => {
                let output = self.kb.status().await.map_err(map_kb_error)?;
                serialize_tool_output(&output)
            }
            "kb_search" => {
                let params: SearchArgs = parse_arguments(arguments)?;
                let output = self
                    .kb
                    .search(
                        &params.query,
                        params.tag.as_deref(),
                        params.status.as_deref(),
                        params.limit.map(|l| l as usize),
                    )
                    .await
                    .map_err(map_kb_error)?;
                serialize_tool_output(&output)
            }
            "kb_read_card" => {
                let params: ReadCardArgs = parse_arguments(arguments)?;
                let output = self.kb.read_card(&params.id).await.map_err(map_kb_error)?;
                serialize_tool_output(&output)
            }
            "kb_delete_card" => {
                let params: DeleteCardArgs = parse_arguments(arguments)?;
                let output = self
                    .kb
                    .delete_card(&params.id)
                    .await
                    .map_err(map_kb_error)?;
                serialize_tool_output(&output)
            }
            "kb_write_file" => {
                let params: WriteFileArgs = parse_arguments(arguments)?;
                let output = self
                    .kb
                    .write_file(&params.path, &params.content)
                    .await
                    .map_err(map_kb_error)?;
                serialize_tool_output(&output)
            }
            "kb_read_file" => {
                let params: ReadFileArgs = parse_arguments(arguments)?;
                let output = self
                    .kb
                    .read_file(&params.path)
                    .await
                    .map_err(map_kb_error)?;
                serialize_tool_output(&output)
            }
            "kb_reset" => {
                let params: ResetArgs = parse_arguments(arguments)?;
                if !params.confirm {
                    return Err(FunctionCallError::RespondToModel(
                        "confirm must be true to reset the knowledge base".to_string(),
                    ));
                }
                let output = self.kb.reset().await.map_err(map_kb_error)?;
                serialize_tool_output(&output)
            }
            "kb_list_cards" => {
                let params: ListCardsArgs = parse_arguments(arguments)?;
                let output = self
                    .kb
                    .list_cards(
                        params.tag.as_deref(),
                        params.status.as_deref(),
                        params.limit.map(|l| l as usize),
                    )
                    .await
                    .map_err(map_kb_error)?;
                serialize_tool_output(&output)
            }
            _ => Err(FunctionCallError::RespondToModel(format!(
                "unknown kb tool: {tool_name}"
            ))),
        }
    }
}

#[async_trait]
impl ToolHandler for KbBridgeHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            tool_name, payload, ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "kb tool handler received unsupported payload".to_string(),
                ));
            }
        };

        self.execute_native_tool(tool_name.as_str(), arguments.as_str())
            .await
    }
}

fn map_kb_error(err: codex_kb::error::KbError) -> FunctionCallError {
    FunctionCallError::RespondToModel(err.to_string())
}

fn panic_payload_to_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}

fn serialize_tool_output<T>(value: &T) -> Result<ToolOutput, FunctionCallError>
where
    T: Serialize,
{
    let body = serde_json::to_string(value)
        .map(FunctionCallOutputBody::Text)
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to serialize kb output: {err}"))
        })?;

    Ok(ToolOutput::Function {
        body,
        success: Some(true),
    })
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
