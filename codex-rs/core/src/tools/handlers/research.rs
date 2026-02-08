use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use async_trait::async_trait;
use codex_protocol::models::FunctionCallOutputBody;
use codex_research_tools::config::ResearchConfig;
use codex_research_tools::error::ResearchError;
use codex_research_tools::types::PaginationParams;
use codex_research_tools::types::ZoteroCollectionItemsParams;
use codex_research_tools::types::ZoteroCollectionsParams;
use codex_research_tools::types::ZoteroItemParams;
use codex_research_tools::types::ZoteroSearchParams;
use codex_research_tools::types::ZoteroTagSearchParams;
use futures::FutureExt;
use serde::Deserialize;
use serde::Serialize;

use crate::config::ResearchToolsToml;
use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub(crate) struct ResearchBridgeHandler {
    toolkit: Arc<codex_research_tools::ResearchToolkit>,
}

impl ResearchBridgeHandler {
    pub(crate) fn new(toolkit: Arc<codex_research_tools::ResearchToolkit>) -> Self {
        Self { toolkit }
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
                    "research tool panicked"
                );
                Err(FunctionCallError::RespondToModel(format!(
                    "{}: {panic_message}",
                    ResearchError::InternalPanic
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
            "paper_search" => {
                let params = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .paper_search(params)
                    .await
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "paper_get" => {
                let params: PaperIdArgs = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .paper_get(params.paper_id.as_str())
                    .await
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "paper_citations" => {
                let params: PaperPaginationArgs = parse_arguments(arguments)?;
                let output = self
                    .toolkit
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
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "paper_references" => {
                let params: PaperPaginationArgs = parse_arguments(arguments)?;
                let output = self
                    .toolkit
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
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "paper_search_sota" => {
                let params = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .paper_search_sota(params)
                    .await
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "paper_find_repos" => {
                let params: PaperIdArgs = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .paper_find_repos(params.paper_id.as_str())
                    .await
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "zotero_search" => {
                let params: ZoteroSearchParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .zotero_search(params)
                    .await
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "zotero_get_item" => {
                let params: ZoteroItemParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .zotero_get_item(params)
                    .await
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "zotero_get_fulltext" => {
                let params: ZoteroItemParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .zotero_get_fulltext(params)
                    .await
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "zotero_get_notes" => {
                let params: ZoteroItemParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .zotero_get_notes(params)
                    .await
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "zotero_get_attachments" => {
                let params: ZoteroItemParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .zotero_get_attachments(params)
                    .await
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "zotero_search_by_tag" => {
                let params: ZoteroTagSearchParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .zotero_search_by_tag(params)
                    .await
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "zotero_get_collections" => {
                let params: ZoteroCollectionsParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .zotero_get_collections(params)
                    .await
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            "zotero_get_collection_items" => {
                let params: ZoteroCollectionItemsParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .zotero_get_collection_items(params)
                    .await
                    .map_err(map_research_error)?;
                serialize_tool_output(&output)
            }
            _ => Err(FunctionCallError::RespondToModel(format!(
                "unknown research tool: {tool_name}"
            ))),
        }
    }
}

#[async_trait]
impl ToolHandler for ResearchBridgeHandler {
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
                    "research tool handler received unsupported payload".to_string(),
                ));
            }
        };

        self.execute_native_tool(tool_name.as_str(), arguments.as_str())
            .await
    }
}

pub(crate) fn build_research_config(toml: Option<&ResearchToolsToml>) -> ResearchConfig {
    let mut config = ResearchConfig::from_env();

    if let Some(toml) = toml {
        if config.zotero_user_id.is_none() {
            config.zotero_user_id = toml.zotero_user_id.clone();
        }
        if config.openalex_email.is_none() {
            config.openalex_email = toml.openalex_email.clone();
        }
        if config.zotero_library_type.is_none() {
            config.zotero_library_type = toml.zotero_library_type.clone();
        }
        if config.zotero_group_id.is_none() {
            config.zotero_group_id = toml.zotero_group_id.clone();
        }
    }

    config
}

fn map_research_error(err: ResearchError) -> FunctionCallError {
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
            FunctionCallError::RespondToModel(format!("failed to serialize research output: {err}"))
        })?;

    Ok(ToolOutput::Function {
        body,
        success: Some(true),
    })
}

#[derive(Deserialize)]
struct PaperIdArgs {
    paper_id: String,
}

#[derive(Deserialize)]
struct PaperPaginationArgs {
    paper_id: String,
    offset: Option<u32>,
    limit: Option<u32>,
    fields: Option<Vec<String>>,
    max_chars_per_item: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_client::build_reqwest_client;
    use codex_research_tools::types::PaperDetail;
    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    #[tokio::test(flavor = "multi_thread")]
    async fn paper_get_handler_uses_toolkit_and_returns_serialized_json() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/graph/v1/paper/s2id123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "paperId": "s2id123",
                "title": "Main Paper",
                "abstract": "Main abstract",
                "year": 2020,
                "citationCount": 15,
                "venue": "ICML",
                "url": "https://example.org/main",
                "externalIds": { "DOI": "10.5555/main" },
                "authors": [{"name": "Carol"}]
            })))
            .mount(&semantic_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/graph/v1/paper/s2id123/references"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total": 1,
                "next": null,
                "data": [{
                    "citedPaper": {
                        "paperId": "s2id-ref",
                        "title": "Reference Paper",
                        "year": 2019,
                        "citationCount": 5,
                        "venue": "NeurIPS",
                        "url": "https://example.org/ref",
                        "externalIds": { "DOI": "10.5555/ref" },
                        "authors": [{"name": "Dave"}]
                    }
                }]
            })))
            .mount(&semantic_server)
            .await;

        let config = ResearchConfig {
            semantic_scholar_base_url: format!("{}/graph/v1", semantic_server.uri()),
            arxiv_base_url: arxiv_server.uri(),
            openalex_base_url: openalex_server.uri(),
            ..ResearchConfig::default()
        };

        let handler = ResearchBridgeHandler::new(Arc::new(
            codex_research_tools::ResearchToolkit::new(build_reqwest_client(), config),
        ));

        let output = handler
            .execute_native_tool("paper_get", r#"{"paper_id":"s2:s2id123"}"#)
            .await
            .expect("paper_get should succeed");

        let ToolOutput::Function { body, .. } = output else {
            panic!("expected function output");
        };
        let text = body.to_text().expect("expected text body");
        let parsed: PaperDetail = serde_json::from_str(&text).expect("parse paper detail json");
        assert_eq!(parsed.paper.title, "Main Paper");
        assert_eq!(parsed.references.len(), 1);
        assert_eq!(parsed.references[0].title, "Reference Paper");
    }

    #[test]
    fn panic_payload_to_message_extracts_string_payloads() {
        let string_payload: Box<dyn Any + Send> = Box::new("boom".to_string());
        assert_eq!(panic_payload_to_message(string_payload.as_ref()), "boom");

        let str_payload: Box<dyn Any + Send> = Box::new("kaboom");
        assert_eq!(panic_payload_to_message(str_payload.as_ref()), "kaboom");

        let other_payload: Box<dyn Any + Send> = Box::new(42_u32);
        assert_eq!(
            panic_payload_to_message(other_payload.as_ref()),
            "non-string panic payload"
        );
    }
}
