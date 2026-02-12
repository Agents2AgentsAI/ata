use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use codex_data_tools::config::DataConfig;
use codex_data_tools::error::DataError;
use codex_data_tools::types::DatasetDownloadParams;
use codex_data_tools::types::DatasetFilesParams;
use codex_data_tools::types::DatasetGetParams;
use codex_data_tools::types::DatasetSearchParams;
use codex_data_tools::types::KaggleCompetitionDownloadParams;
use codex_data_tools::types::KaggleCompetitionFilesParams;
use codex_data_tools::types::KaggleCompetitionsParams;
use codex_protocol::models::FunctionCallOutputBody;
use codex_secrets::SecretName;
use codex_secrets::SecretScope;
use codex_secrets::SecretsBackendKind;
use codex_secrets::SecretsManager;
use codex_secrets::environment_id_from_cwd;
use futures::FutureExt;
use serde::Serialize;

use crate::config::DataToolsToml;
use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub(crate) struct DataBridgeHandler {
    toolkit: Arc<codex_data_tools::DataToolkit>,
}

impl DataBridgeHandler {
    pub(crate) fn new(toolkit: Arc<codex_data_tools::DataToolkit>) -> Self {
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
                    "data tool panicked"
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
            "dataset_search" => {
                let params: DatasetSearchParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .dataset_search(params)
                    .await
                    .map_err(map_data_error)?;
                serialize_tool_output(&output)
            }
            "dataset_get" => {
                let params: DatasetGetParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .dataset_get(params)
                    .await
                    .map_err(map_data_error)?;
                serialize_tool_output(&output)
            }
            "dataset_list_files" => {
                let params: DatasetFilesParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .dataset_list_files(params)
                    .await
                    .map_err(map_data_error)?;
                serialize_tool_output(&output)
            }
            "dataset_download" => {
                let params: DatasetDownloadParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .dataset_download(params)
                    .await
                    .map_err(map_data_error)?;
                serialize_tool_output(&output)
            }
            // HuggingFace-specific tools
            "hf_dataset_info" => {
                // For now, delegate to dataset_get with HF prefix
                let params: DatasetGetParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .dataset_get(params)
                    .await
                    .map_err(map_data_error)?;
                serialize_tool_output(&output)
            }
            // Kaggle-specific tools
            "kaggle_dataset_info" => {
                let params: DatasetGetParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .dataset_get(params)
                    .await
                    .map_err(map_data_error)?;
                serialize_tool_output(&output)
            }
            "kaggle_competitions" => {
                let params: KaggleCompetitionsParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .kaggle_competitions(params)
                    .await
                    .map_err(map_data_error)?;
                serialize_tool_output(&output)
            }
            "kaggle_competition_list_files" => {
                let params: KaggleCompetitionFilesParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .kaggle_competition_list_files(params)
                    .await
                    .map_err(map_data_error)?;
                serialize_tool_output(&output)
            }
            "kaggle_competition_download" => {
                let params: KaggleCompetitionDownloadParams = parse_arguments(arguments)?;
                let output = self
                    .toolkit
                    .kaggle_competition_download(params)
                    .await
                    .map_err(map_data_error)?;
                serialize_tool_output(&output)
            }
            _ => Err(FunctionCallError::RespondToModel(format!(
                "unknown data tool: {tool_name}"
            ))),
        }
    }
}

#[async_trait]
impl ToolHandler for DataBridgeHandler {
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
                    "data tool handler received unsupported payload".to_string(),
                ));
            }
        };

        self.execute_native_tool(tool_name.as_str(), arguments.as_str())
            .await
    }
}

pub(crate) fn build_data_config(
    toml: Option<&DataToolsToml>,
    codex_home: &Path,
    cwd: &Path,
) -> DataConfig {
    let mut config = DataConfig::from_env();
    let secret_resolver = DataSecretResolver::new(codex_home, cwd);

    // `from_env()` already loaded environment values, so secret lookup only fills missing keys.
    apply_secret_overrides(&mut config, |name| secret_resolver.resolve(name));

    if let Some(toml) = toml {
        if config.huggingface_token.is_none() {
            config.huggingface_token = toml.huggingface_token.clone();
        }
        if config.kaggle_username.is_none() {
            config.kaggle_username = toml.kaggle_username.clone();
        }
        if config.kaggle_key.is_none() {
            config.kaggle_key = toml.kaggle_key.clone();
        }
    }

    config
}

fn apply_secret_overrides<F>(config: &mut DataConfig, mut resolve_secret: F)
where
    F: FnMut(&str) -> Option<String>,
{
    if config.huggingface_token.is_none() {
        config.huggingface_token =
            resolve_secret("HF_TOKEN").or_else(|| resolve_secret("HUGGING_FACE_HUB_TOKEN"));
    }
    if config.kaggle_username.is_none() {
        config.kaggle_username = resolve_secret("KAGGLE_USERNAME");
    }
    if config.kaggle_key.is_none() {
        config.kaggle_key = resolve_secret("KAGGLE_KEY");
    }
    if config.kaggle_api_token.is_none() {
        config.kaggle_api_token = resolve_secret("KAGGLE_API_TOKEN");
    }
    if config.cosmos_api_key.is_none() {
        config.cosmos_api_key = resolve_secret("NVIDIA_API_KEY");
    }
}

struct DataSecretResolver {
    manager: SecretsManager,
    environment_scope: Option<SecretScope>,
}

impl DataSecretResolver {
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
                    "failed to load data secret; falling back"
                );
                None
            }
        }
    }
}

fn map_data_error(err: DataError) -> FunctionCallError {
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
            FunctionCallError::RespondToModel(format!("failed to serialize data output: {err}"))
        })?;

    Ok(ToolOutput::Function {
        body,
        success: Some(true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_client::build_reqwest_client;
    use codex_data_tools::types::DatasetSearchResult;
    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    #[tokio::test(flavor = "multi_thread")]
    async fn dataset_search_handler_uses_toolkit_and_returns_serialized_json() {
        let hf_server = MockServer::start().await;
        let kaggle_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/datasets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": "squad",
                    "author": "rajpurkar",
                    "downloads": 1000000
                }
            ])))
            .mount(&hf_server)
            .await;

        let mut config = DataConfig::default();
        config.huggingface_base_url = hf_server.uri();
        config.kaggle_base_url = kaggle_server.uri();

        let handler = DataBridgeHandler::new(Arc::new(codex_data_tools::DataToolkit::new(
            build_reqwest_client(),
            config,
        )));

        let output = handler
            .execute_native_tool(
                "dataset_search",
                r#"{"query":"test","source":"huggingface"}"#,
            )
            .await
            .expect("dataset_search should succeed");

        let ToolOutput::Function { body, .. } = output else {
            panic!("expected function output");
        };
        let text = body.to_text().expect("expected text body");
        let parsed: DatasetSearchResult =
            serde_json::from_str(&text).expect("parse search result json");
        assert_eq!(parsed.datasets.len(), 1);
        assert_eq!(parsed.datasets[0].id, "squad");
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
