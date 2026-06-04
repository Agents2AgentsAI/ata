use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;

use codex_research_tools::config::ResearchConfig;
use codex_research_tools::error::ResearchError;
use codex_research_tools::types::HnGetThreadParams;
use codex_research_tools::types::HnSearchParams;
use codex_research_tools::types::PaginationParams;
use codex_research_tools::types::PaperRecommendationParams;
use codex_research_tools::types::PaperSearchParams;
use codex_research_tools::types::PatentGetParams;
use codex_research_tools::types::PatentSearchParams;
use codex_research_tools::types::ZoteroAddItemsToCollectionParams;
use codex_research_tools::types::ZoteroAdvancedSearchParams;
use codex_research_tools::types::ZoteroAnnotationsParams;
use codex_research_tools::types::ZoteroCitationParams;
use codex_research_tools::types::ZoteroCollectionItemsParams;
use codex_research_tools::types::ZoteroCollectionsParams;
use codex_research_tools::types::ZoteroCreateAttachmentLinkParams;
use codex_research_tools::types::ZoteroCreateCollectionParams;
use codex_research_tools::types::ZoteroCreateItemsParams;
use codex_research_tools::types::ZoteroFindOrCreateCollectionParams;
use codex_research_tools::types::ZoteroGrepParams;
use codex_research_tools::types::ZoteroItemParams;
use codex_research_tools::types::ZoteroListGroupsParams;
use codex_research_tools::types::ZoteroRecentParams;
use codex_research_tools::types::ZoteroSearchNotesParams;
use codex_research_tools::types::ZoteroSearchParams;
use codex_research_tools::types::ZoteroTagsParams;
use codex_research_tools::types::ZoteroUpdateItemsParams;
use codex_secrets::SecretName;
use codex_secrets::SecretScope;
use codex_secrets::SecretsBackendKind;
use codex_secrets::SecretsManager;
use codex_secrets::environment_id_from_cwd;
use futures::FutureExt;
use serde::Deserialize;
use serde::Serialize;

use crate::config::ResearchToolsToml;
use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;
use codex_tools::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;

pub(crate) struct ResearchBridgeHandler {
    tool_name: ToolName,
    spec: ToolSpec,
    toolkit: Arc<codex_research_tools::ResearchToolkit>,
}

impl ResearchBridgeHandler {
    pub(crate) fn new(
        tool_name: impl Into<String>,
        spec: ToolSpec,
        toolkit: Arc<codex_research_tools::ResearchToolkit>,
    ) -> Self {
        Self {
            tool_name: ToolName::plain(tool_name.into()),
            spec,
            toolkit,
        }
    }

    async fn execute_native_tool(
        &self,
        tool_name: &str,
        arguments: &str,
    ) -> Result<FunctionToolOutput, FunctionCallError> {
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
    ) -> Result<FunctionToolOutput, FunctionCallError> {
        macro_rules! dispatch_with_params {
            ($params_ty:ty, |$params:ident| $tool_call:expr) => {{
                let $params: $params_ty = parse_arguments(arguments)?;
                let output = $tool_call.await.map_err(map_research_error)?;
                serialize_tool_output(&output)
            }};
        }

        match tool_name {
            "paper_search" => dispatch_with_params!(PaperSearchParams, |params| self
                .toolkit
                .paper_search(params)),
            "paper_get" => dispatch_with_params!(PaperIdArgs, |params| self
                .toolkit
                .paper_get(params.paper_id.as_str())),
            "paper_citations" => {
                dispatch_with_params!(PaperPaginationArgs, |params| self.toolkit.paper_citations(
                    params.paper_id.as_str(),
                    PaginationParams {
                        offset: params.offset,
                        limit: params.limit,
                        fields: params.fields,
                        max_chars_per_item: params.max_chars_per_item,
                    },
                ))
            }
            "paper_references" => {
                dispatch_with_params!(PaperPaginationArgs, |params| self.toolkit.paper_references(
                    params.paper_id.as_str(),
                    PaginationParams {
                        offset: params.offset,
                        limit: params.limit,
                        fields: params.fields,
                        max_chars_per_item: params.max_chars_per_item,
                    },
                ))
            }
            "paper_recommendations" => {
                dispatch_with_params!(PaperRecommendationParams, |params| self
                    .toolkit
                    .paper_recommendations(params))
            }
            "zotero_search" => dispatch_with_params!(ZoteroSearchParams, |params| self
                .toolkit
                .zotero_search(params)),
            "zotero_get_tags" => dispatch_with_params!(ZoteroTagsParams, |params| self
                .toolkit
                .zotero_get_tags(params)),
            "zotero_get_recent" => dispatch_with_params!(ZoteroRecentParams, |params| self
                .toolkit
                .zotero_get_recent(params)),
            "zotero_advanced_search" => {
                dispatch_with_params!(ZoteroAdvancedSearchParams, |params| self
                    .toolkit
                    .zotero_advanced_search(params))
            }
            "zotero_grep_text" => dispatch_with_params!(ZoteroGrepParams, |params| self
                .toolkit
                .zotero_grep_text(params)),
            "zotero_search_notes" => dispatch_with_params!(ZoteroSearchNotesParams, |params| self
                .toolkit
                .zotero_search_notes(params)),
            "zotero_get_item" => dispatch_with_params!(ZoteroItemParams, |params| self
                .toolkit
                .zotero_get_item(params)),
            "zotero_get_item_citation" => {
                dispatch_with_params!(ZoteroCitationParams, |params| self
                    .toolkit
                    .zotero_get_item_citation(params))
            }
            "zotero_get_fulltext" => dispatch_with_params!(ZoteroItemParams, |params| self
                .toolkit
                .zotero_get_fulltext(params)),
            "zotero_get_notes" => dispatch_with_params!(ZoteroItemParams, |params| self
                .toolkit
                .zotero_get_notes(params)),
            "zotero_get_annotations" => {
                dispatch_with_params!(ZoteroAnnotationsParams, |params| self
                    .toolkit
                    .zotero_get_annotations(params))
            }
            "zotero_get_attachments" => dispatch_with_params!(ZoteroItemParams, |params| self
                .toolkit
                .zotero_get_attachments(params)),
            "zotero_get_collections" => {
                dispatch_with_params!(ZoteroCollectionsParams, |params| self
                    .toolkit
                    .zotero_get_collections(params))
            }
            "zotero_list_groups" => dispatch_with_params!(ZoteroListGroupsParams, |params| self
                .toolkit
                .zotero_list_groups(params)),
            "zotero_get_collection_items" => {
                dispatch_with_params!(ZoteroCollectionItemsParams, |params| self
                    .toolkit
                    .zotero_get_collection_items(params))
            }
            "zotero_create_collection" => {
                dispatch_with_params!(ZoteroCreateCollectionParams, |params| self
                    .toolkit
                    .zotero_create_collection(params))
            }
            "zotero_find_or_create_collection" => {
                dispatch_with_params!(ZoteroFindOrCreateCollectionParams, |params| self
                    .toolkit
                    .zotero_find_or_create_collection(params))
            }
            "zotero_create_items" => dispatch_with_params!(ZoteroCreateItemsParams, |params| self
                .toolkit
                .zotero_create_items(params)),
            "zotero_update_items" => dispatch_with_params!(ZoteroUpdateItemsParams, |params| self
                .toolkit
                .zotero_update_items(params)),
            "zotero_add_items_to_collection" => {
                dispatch_with_params!(ZoteroAddItemsToCollectionParams, |params| self
                    .toolkit
                    .zotero_add_items_to_collection(params))
            }
            "zotero_create_attachment_link" => {
                dispatch_with_params!(ZoteroCreateAttachmentLinkParams, |params| self
                    .toolkit
                    .zotero_create_attachment_link(params))
            }
            "hn_search" => {
                dispatch_with_params!(HnSearchParams, |params| self.toolkit.hn_search(params))
            }
            "hn_get_thread" => dispatch_with_params!(HnGetThreadParams, |params| self
                .toolkit
                .hn_get_thread(params)),
            "patent_search" => {
                dispatch_with_params!(PatentSearchParams, |params| self
                    .toolkit
                    .patent_search(params))
            }
            "patent_get" => {
                dispatch_with_params!(PatentGetParams, |params| self.toolkit.patent_get(params))
            }
            "repo_clone_and_summarize" => {
                dispatch_with_params!(RepoUrlAndBranchArgs, |params| self
                    .toolkit
                    .repo_clone_and_summarize(params.repo_url.as_str(), params.branch.as_deref()))
            }
            "repo_find_models" => dispatch_with_params!(RepoUrlAndFrameworkArgs, |params| self
                .toolkit
                .repo_find_models(params.repo_url.as_str(), params.framework.as_deref())),
            "repo_extract_requirements" => {
                dispatch_with_params!(RepoUrlArgs, |params| self
                    .toolkit
                    .repo_extract_requirements(params.repo_url.as_str()))
            }
            "repo_find_entrypoints" => dispatch_with_params!(RepoUrlAndTaskHintArgs, |params| self
                .toolkit
                .repo_find_entrypoints(params.repo_url.as_str(), params.task_hint.as_deref())),
            "repo_extract_io_shapes" => dispatch_with_params!(RepoUrlAndModelClassArgs, |params| {
                self.toolkit
                    .repo_extract_io_shapes(params.repo_url.as_str(), params.model_class.as_deref())
            }),
            "repo_get_health" => dispatch_with_params!(RepoUrlArgs, |params| self
                .toolkit
                .repo_get_health(params.repo_url.as_str())),
            "repo_find_export_paths" => dispatch_with_params!(RepoUrlArgs, |params| self
                .toolkit
                .repo_find_export_paths(params.repo_url.as_str())),
            "repo_extract_config_schema" => dispatch_with_params!(RepoUrlArgs, |params| self
                .toolkit
                .repo_extract_config_schema(params.repo_url.as_str())),
            "repo_diff_requirements" => {
                dispatch_with_params!(RepoDiffRequirementsArgs, |params| self
                    .toolkit
                    .repo_diff_requirements(
                        params.repo_url.as_str(),
                        params.local_requirements_path.as_str()
                    ))
            }
            _ => Err(FunctionCallError::RespondToModel(format!(
                "unknown research tool: {tool_name}"
            ))),
        }
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for ResearchBridgeHandler {
    fn tool_name(&self) -> ToolName {
        self.tool_name.clone()
    }

    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn handle(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn crate::tools::context::ToolOutput>, FunctionCallError> {
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

        let output = self
            .execute_native_tool(tool_name.name.as_str(), arguments.as_str())
            .await?;
        Ok(boxed_tool_output(output))
    }
}

impl CoreToolRuntime for ResearchBridgeHandler {}

pub fn build_research_config(
    toml: Option<&ResearchToolsToml>,
    codex_home: &Path,
    cwd: &Path,
) -> ResearchConfig {
    let mut config = ResearchConfig::from_env();
    let secret_resolver = ResearchSecretResolver::new(codex_home, cwd);

    // `from_env()` already loaded environment values, so secret lookup only fills missing keys.
    apply_secret_overrides(&mut config, |name| secret_resolver.resolve(name));

    if let Some(toml) = toml {
        apply_toml_research_overrides(&mut config, toml);
    }

    config
}

fn apply_toml_research_overrides(config: &mut ResearchConfig, toml: &ResearchToolsToml) {
    fn normalized_option_override(value: Option<&String>) -> Option<Option<String>> {
        value.map(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    }

    fn apply_option_override(target: &mut Option<String>, override_value: Option<&String>) {
        if let Some(value) = normalized_option_override(override_value) {
            *target = value;
        }
    }

    fn apply_string_override(target: &mut String, override_value: Option<&String>) {
        if let Some(Some(value)) = normalized_option_override(override_value) {
            *target = value;
        }
    }

    apply_option_override(&mut config.zotero_api_key, toml.zotero_api_key.as_ref());
    apply_option_override(&mut config.zotero_user_id, toml.zotero_user_id.as_ref());
    apply_option_override(&mut config.openalex_email, toml.openalex_email.as_ref());
    apply_option_override(
        &mut config.zotero_library_type,
        toml.zotero_library_type.as_ref(),
    );
    apply_option_override(&mut config.zotero_group_id, toml.zotero_group_id.as_ref());
    apply_option_override(
        &mut config.zotero_storage_dir,
        toml.zotero_storage_dir.as_ref(),
    );
    apply_string_override(&mut config.zotero_base_url, toml.zotero_base_url.as_ref());
}

fn apply_secret_overrides<F>(config: &mut ResearchConfig, mut resolve_secret: F)
where
    F: FnMut(&str) -> Option<String>,
{
    if config.semantic_scholar_api_key.is_none() {
        config.semantic_scholar_api_key = resolve_secret("SEMANTIC_SCHOLAR_API_KEY");
    }
    if config.zotero_api_key.is_none() {
        config.zotero_api_key = resolve_secret("ZOTERO_API_KEY");
    }
    if config.github_token.is_none() {
        config.github_token = resolve_secret("GITHUB_TOKEN");
    }
    if config.epo_consumer_key.is_none() {
        config.epo_consumer_key = resolve_secret("EPO_CONSUMER_KEY");
    }
    if config.epo_consumer_secret.is_none() {
        config.epo_consumer_secret = resolve_secret("EPO_CONSUMER_SECRET");
    }
}

struct ResearchSecretResolver {
    manager: SecretsManager,
    environment_scope: Option<SecretScope>,
}

impl ResearchSecretResolver {
    fn new(codex_home: &Path, cwd: &Path) -> Self {
        let environment_scope = SecretScope::environment(environment_id_from_cwd(cwd)).ok();
        Self {
            manager: SecretsManager::new(codex_home.to_path_buf(), SecretsBackendKind::Local),
            environment_scope,
        }
    }

    fn resolve(&self, secret_name: &str) -> Option<String> {
        let secret_name = SecretName::new(secret_name).ok()?;

        if let Some(scope) = self.environment_scope.as_ref()
            && let Some(value) = self.get(scope, &secret_name)
        {
            return Some(value);
        }

        self.get(&SecretScope::Global, &secret_name)
    }

    fn get(&self, scope: &SecretScope, name: &SecretName) -> Option<String> {
        match self.manager.get(scope, name) {
            Ok(value) => value,
            Err(err) => {
                tracing::debug!(
                    error = %err,
                    scope = ?scope,
                    secret_name = %name,
                    "failed to load research secret; falling back"
                );
                None
            }
        }
    }
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

fn serialize_tool_output<T>(value: &T) -> Result<FunctionToolOutput, FunctionCallError>
where
    T: Serialize,
{
    let text = serde_json::to_string(value).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to serialize research output: {err}"))
    })?;

    Ok(FunctionToolOutput::from_text(text, Some(true)))
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

#[derive(Deserialize)]
struct RepoUrlArgs {
    repo_url: String,
}

#[derive(Deserialize)]
struct RepoUrlAndBranchArgs {
    repo_url: String,
    branch: Option<String>,
}

#[derive(Deserialize)]
struct RepoUrlAndFrameworkArgs {
    repo_url: String,
    framework: Option<String>,
}

#[derive(Deserialize)]
struct RepoUrlAndTaskHintArgs {
    repo_url: String,
    task_hint: Option<String>,
}

#[derive(Deserialize)]
struct RepoUrlAndModelClassArgs {
    repo_url: String,
    model_class: Option<String>,
}

#[derive(Deserialize)]
struct RepoDiffRequirementsArgs {
    repo_url: String,
    local_requirements_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_login::default_client::build_reqwest_client;
    use codex_research_tools::types::PaperDetail;
    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    fn placeholder_spec(name: &str) -> ToolSpec {
        ToolSpec::Function(codex_tools::ResponsesApiTool {
            name: name.to_string(),
            description: String::new(),
            strict: false,
            parameters: codex_tools::JsonSchema::object(
                Default::default(),
                None,
                Some(codex_tools::AdditionalProperties::Boolean(false)),
            ),
            output_schema: None,
            defer_loading: None,
        })
    }

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

        let handler = ResearchBridgeHandler::new(
            "paper_get",
            placeholder_spec("paper_get"),
            Arc::new(codex_research_tools::ResearchToolkit::new(
                build_reqwest_client(),
                config,
            )),
        );

        let output = handler
            .execute_native_tool("paper_get", r#"{"paper_id":"s2:s2id123"}"#)
            .await
            .expect("paper_get should succeed");

        let text = output.into_text();
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

    #[test]
    fn apply_secret_overrides_fills_missing_keys_only() {
        let mut config = ResearchConfig {
            semantic_scholar_api_key: Some("env-s2".to_string()),
            ..ResearchConfig::default()
        };

        apply_secret_overrides(&mut config, |name| match name {
            "SEMANTIC_SCHOLAR_API_KEY" => Some("secret-s2".to_string()),
            "ZOTERO_API_KEY" => Some("secret-zotero".to_string()),
            "GITHUB_TOKEN" => Some("secret-gh".to_string()),
            _ => None,
        });

        assert_eq!(config.semantic_scholar_api_key, Some("env-s2".to_string()));
        assert_eq!(config.zotero_api_key, Some("secret-zotero".to_string()));
        assert_eq!(config.github_token, Some("secret-gh".to_string()));
    }

    #[test]
    fn apply_toml_research_overrides_replace_existing_values_and_can_clear_api_key() {
        let mut config = ResearchConfig {
            zotero_api_key: Some("env-zotero".to_string()),
            zotero_user_id: Some("env-user".to_string()),
            openalex_email: Some("env@example.com".to_string()),
            zotero_library_type: Some("user".to_string()),
            zotero_group_id: Some("123".to_string()),
            zotero_storage_dir: Some("/tmp/env-zotero".to_string()),
            zotero_base_url: "https://api.zotero.org".to_string(),
            ..ResearchConfig::default()
        };
        let toml = ResearchToolsToml {
            zotero_api_key: Some("".to_string()),
            zotero_user_id: Some("cfg-user".to_string()),
            openalex_email: Some("cfg@example.com".to_string()),
            zotero_library_type: Some("group".to_string()),
            zotero_group_id: Some("6416955".to_string()),
            zotero_base_url: Some("http://localhost:23119/api".to_string()),
            zotero_storage_dir: Some("/Users/nima/Zotero".to_string()),
        };

        apply_toml_research_overrides(&mut config, &toml);

        assert_eq!(config.zotero_api_key, None);
        assert_eq!(config.zotero_user_id, Some("cfg-user".to_string()));
        assert_eq!(config.openalex_email, Some("cfg@example.com".to_string()));
        assert_eq!(config.zotero_library_type, Some("group".to_string()));
        assert_eq!(config.zotero_group_id, Some("6416955".to_string()));
        assert_eq!(
            config.zotero_storage_dir,
            Some("/Users/nima/Zotero".to_string())
        );
        assert_eq!(
            config.zotero_base_url,
            "http://localhost:23119/api".to_string()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paper_get_handler_applies_research_request_timeout_with_core_client() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/graph/v1/paper/s2id123"))
            .respond_with(
                ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(100)),
            )
            .mount(&semantic_server)
            .await;

        let config = ResearchConfig {
            semantic_scholar_base_url: format!("{}/graph/v1", semantic_server.uri()),
            arxiv_base_url: arxiv_server.uri(),
            openalex_base_url: openalex_server.uri(),
            request_timeout: std::time::Duration::from_millis(25),
            tool_timeout: std::time::Duration::from_secs(2),
            ..ResearchConfig::default()
        };

        let handler = ResearchBridgeHandler::new(
            "paper_get",
            placeholder_spec("paper_get"),
            Arc::new(codex_research_tools::ResearchToolkit::new(
                build_reqwest_client(),
                config,
            )),
        );

        let err = match handler
            .execute_native_tool("paper_get", r#"{"paper_id":"s2:s2id123"}"#)
            .await
        {
            Ok(_) => panic!("paper_get should time out"),
            Err(err) => err,
        };

        let FunctionCallError::RespondToModel(message) = err else {
            panic!("unexpected error variant");
        };
        assert!(
            message.contains("timed out after 25ms"),
            "unexpected message: {message}"
        );
    }
}
