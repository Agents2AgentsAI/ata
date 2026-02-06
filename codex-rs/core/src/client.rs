//! Session- and turn-scoped helpers for talking to model provider APIs.
//!
//! `ModelClient` is intended to live for the lifetime of a Codex session and holds the stable
//! configuration and state needed to talk to a provider (auth, provider selection, conversation id,
//! and feature-gated request behavior).
//!
//! Per-turn settings (model selection, reasoning controls, telemetry context, and turn metadata)
//! are passed explicitly to streaming and unary methods so that the turn lifetime is visible at the
//! call site.
//!
//! A [`ModelClientSession`] is created per turn and is used to stream one or more Responses API
//! requests during that turn. It caches a Responses WebSocket connection (opened lazily) and
//! stores per-turn state such as the `x-codex-turn-state` token used for sticky routing.

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use crate::api_bridge::CoreAuthProvider;
use crate::api_bridge::auth_provider_from_auth;
use crate::api_bridge::map_api_error;
use crate::auth::UnauthorizedRecovery;
use codex_api::CompactClient as ApiCompactClient;
use codex_api::CompactionInput as ApiCompactionInput;
use codex_api::MemoriesClient as ApiMemoriesClient;
use codex_api::MemoryTrace as ApiMemoryTrace;
use codex_api::MemoryTraceSummarizeInput as ApiMemoryTraceSummarizeInput;
use codex_api::MemoryTraceSummaryOutput as ApiMemoryTraceSummaryOutput;
use codex_api::Prompt as ApiPrompt;
use codex_api::RequestTelemetry;
use codex_api::ReqwestTransport;
use codex_api::ResponseAppendWsRequest;
use codex_api::ResponseCreateWsRequest;
use codex_api::ResponsesClient as ApiResponsesClient;
use codex_api::ResponsesOptions as ApiResponsesOptions;
use codex_api::ResponsesWebsocketClient as ApiWebSocketResponsesClient;
use codex_api::ResponsesWebsocketConnection as ApiWebSocketConnection;
use codex_api::SseTelemetry;
use codex_api::TransportError;
use codex_api::WebsocketTelemetry;
use codex_api::build_conversation_headers;
use codex_api::common::Reasoning;
use codex_api::common::ResponsesWsRequest;
use codex_api::create_text_param_for_request;
use codex_api::error::ApiError;
use codex_api::requests::responses::Compression;
use codex_otel::OtelManager;

use codex_protocol::ThreadId;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::config_types::Verbosity as VerbosityConfig;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::protocol::SessionSource;
use eventsource_stream::Event;
use eventsource_stream::EventStreamError;
use futures::StreamExt;
use http::HeaderMap as ApiHeaderMap;
use http::HeaderValue;
use http::StatusCode as HttpStatusCode;
use reqwest::StatusCode;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Error;
use tokio_tungstenite::tungstenite::Message;
use tracing::warn;

use crate::AuthManager;
use crate::auth::CodexAuth;
use crate::auth::RefreshTokenError;
use crate::auth::AuthCredentialsStoreMode;
use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use crate::default_client::build_reqwest_client;
use crate::error::CodexErr;
use crate::error::Result;
use crate::flags::CODEX_RS_SSE_FIXTURE;
use crate::model_provider_info::ModelProviderInfo;
use crate::model_provider_info::WireApi;
use crate::tools::spec::create_tools_json_for_responses_api;
use std::path::PathBuf;

pub const OPENAI_BETA_HEADER: &str = "OpenAI-Beta";
pub const OPENAI_BETA_RESPONSES_WEBSOCKETS: &str = "responses_websockets=2026-02-04";
pub const X_CODEX_TURN_STATE_HEADER: &str = "x-codex-turn-state";
pub const X_CODEX_TURN_METADATA_HEADER: &str = "x-codex-turn-metadata";
pub const X_RESPONSESAPI_INCLUDE_TIMING_METRICS_HEADER: &str =
    "x-responsesapi-include-timing-metrics";

/// Session-scoped state shared by all [`ModelClient`] clones.
///
/// This is intentionally kept minimal so `ModelClient` does not need to hold a full `Config`. Most
/// configuration is per turn and is passed explicitly to streaming/unary methods.
#[derive(Debug)]
struct ModelClientState {
    auth_manager: Option<Arc<AuthManager>>,
    conversation_id: ThreadId,
    provider: ModelProviderInfo,
    session_source: SessionSource,
    model_verbosity: Option<VerbosityConfig>,
    enable_responses_websockets: bool,
    enable_request_compression: bool,
    include_timing_metrics: bool,
    beta_features_header: Option<String>,
    disable_websockets: AtomicBool,
    /// Path to the codex home directory, used by multi-provider auth to look up stored credentials.
    codex_home: PathBuf,
    /// How auth credentials are stored (keychain vs plaintext).
    cli_auth_credentials_store_mode: AuthCredentialsStoreMode,
}

/// A session-scoped client for model-provider API calls.
///
/// This holds configuration and state that should be shared across turns within a Codex session
/// (auth, provider selection, conversation id, feature-gated request behavior, and transport
/// fallback state).
///
/// WebSocket fallback is session-scoped: once a turn activates the HTTP fallback, subsequent turns
/// will also use HTTP for the remainder of the session.
///
/// Turn-scoped settings (model selection, reasoning controls, telemetry context, and turn metadata)
/// are passed explicitly to the relevant methods to keep turn lifetime visible at the call site.
///
/// This type is cheap to clone.
#[derive(Debug, Clone)]
pub struct ModelClient {
    state: Arc<ModelClientState>,
}

/// A turn-scoped streaming session created from a [`ModelClient`].
///
/// The session lazily establishes a Responses WebSocket connection (and reuses it across multiple
/// requests) and caches per-turn state:
///
/// - The last request's input items, so subsequent calls can use `response.append` when the input
///   is an incremental extension of the previous request.
/// - The `x-codex-turn-state` sticky-routing token, which must be replayed for all requests within
///   the same turn.
///
/// Create a fresh `ModelClientSession` for each Codex turn. Reusing it across turns would replay
/// the previous turn's sticky-routing token into the next turn, which violates the client/server
/// contract and can cause routing bugs.
pub struct ModelClientSession {
    client: ModelClient,
    connection: Option<ApiWebSocketConnection>,
    websocket_last_items: Vec<ResponseItem>,
    /// Turn state for sticky routing.
    ///
    /// This is an `OnceLock` that stores the turn state value received from the server
    /// on turn start via the `x-codex-turn-state` response header. Once set, this value
    /// should be sent back to the server in the `x-codex-turn-state` request header for
    /// all subsequent requests within the same turn to maintain sticky routing.
    ///
    /// This is a contract between the client and server: we receive it at turn start,
    /// keep sending it unchanged between turn requests (e.g., for retries, incremental
    /// appends, or continuation requests), and must not send it between different turns.
    turn_state: Arc<OnceLock<String>>,
}

impl ModelClient {
    #[allow(clippy::too_many_arguments)]
    /// Creates a new session-scoped `ModelClient`.
    ///
    /// All arguments are expected to be stable for the lifetime of a Codex session. Per-turn values
    /// are passed to [`ModelClientSession::stream`] (and other turn-scoped methods) explicitly.
    pub fn new(
        auth_manager: Option<Arc<AuthManager>>,
        conversation_id: ThreadId,
        provider: ModelProviderInfo,
        session_source: SessionSource,
        model_verbosity: Option<VerbosityConfig>,
        enable_responses_websockets: bool,
        enable_request_compression: bool,
        include_timing_metrics: bool,
        beta_features_header: Option<String>,
        codex_home: PathBuf,
        cli_auth_credentials_store_mode: AuthCredentialsStoreMode,
    ) -> Self {
        Self {
            state: Arc::new(ModelClientState {
                auth_manager,
                conversation_id,
                provider,
                session_source,
                model_verbosity,
                enable_responses_websockets,
                enable_request_compression,
                include_timing_metrics,
                beta_features_header,
                disable_websockets: AtomicBool::new(false),
                codex_home,
                cli_auth_credentials_store_mode,
            }),
        }
    }

    /// Creates a fresh turn-scoped streaming session.
    ///
    /// This does not open any network connections; the WebSocket connection is established lazily
    /// when the first WebSocket stream request is issued.
    pub fn new_session(&self) -> ModelClientSession {
        ModelClientSession {
            client: self.clone(),
            connection: None,
            websocket_last_items: Vec::new(),
            turn_state: Arc::new(OnceLock::new()),
        }
    }

    /// Compacts the current conversation history using the Compact endpoint.
    ///
    /// This is a unary call (no streaming) that returns a new list of
    /// `ResponseItem`s representing the compacted transcript.
    ///
    /// The model selection and telemetry context are passed explicitly to keep `ModelClient`
    /// session-scoped.
    pub async fn compact_conversation_history(
        &self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        otel_manager: &OtelManager,
    ) -> Result<Vec<ResponseItem>> {
        if prompt.input.is_empty() {
            return Ok(Vec::new());
        }
        let auth_manager = self.state.auth_manager.clone();
        let auth = match auth_manager.as_ref() {
            Some(manager) => manager.auth().await,
            None => None,
        };
        let api_provider = self
            .state
            .provider
            .to_api_provider(auth.as_ref().map(CodexAuth::auth_mode))?;
        let api_auth = auth_provider_from_auth(auth.clone(), &self.state.provider)?;
        let transport = ReqwestTransport::new(build_reqwest_client());
        let request_telemetry = Self::build_request_telemetry(otel_manager);
        let client = ApiCompactClient::new(transport, api_provider, api_auth)
            .with_telemetry(Some(request_telemetry));

        let instructions = prompt.base_instructions.text.clone();
        let payload = ApiCompactionInput {
            model: &model_info.slug,
            input: &prompt.input,
            instructions: &instructions,
        };

        let extra_headers = self.build_subagent_headers();
        client
            .compact_input(&payload, extra_headers)
            .await
            .map_err(map_api_error)
    }

    /// Builds memory summaries for each provided normalized trace.
    ///
    /// This is a unary call (no streaming) to `/v1/memories/trace_summarize`.
    ///
    /// The model selection, reasoning effort, and telemetry context are passed explicitly to keep
    /// `ModelClient` session-scoped.
    pub async fn summarize_memory_traces(
        &self,
        traces: Vec<ApiMemoryTrace>,
        model_info: &ModelInfo,
        effort: Option<ReasoningEffortConfig>,
        otel_manager: &OtelManager,
    ) -> Result<Vec<ApiMemoryTraceSummaryOutput>> {
        if traces.is_empty() {
            return Ok(Vec::new());
        }

        let auth_manager = self.state.auth_manager.clone();
        let auth = match auth_manager.as_ref() {
            Some(manager) => manager.auth().await,
            None => None,
        };
        let api_provider = self
            .state
            .provider
            .to_api_provider(auth.as_ref().map(CodexAuth::auth_mode))?;
        let api_auth = auth_provider_from_auth(auth, &self.state.provider)?;
        let transport = ReqwestTransport::new(build_reqwest_client());
        let request_telemetry = Self::build_request_telemetry(otel_manager);
        let client = ApiMemoriesClient::new(transport, api_provider, api_auth)
            .with_telemetry(Some(request_telemetry));

        let payload = ApiMemoryTraceSummarizeInput {
            model: model_info.slug.clone(),
            traces,
            reasoning: effort.map(|effort| Reasoning {
                effort: Some(effort),
                summary: None,
            }),
        };

        client
            .trace_summarize_input(&payload, self.build_subagent_headers())
            .await
            .map_err(map_api_error)
    }

    fn build_subagent_headers(&self) -> ApiHeaderMap {
        let mut extra_headers = ApiHeaderMap::new();
        if let SessionSource::SubAgent(sub) = &self.state.session_source {
            let subagent = match sub {
                crate::protocol::SubAgentSource::Review => "review".to_string(),
                crate::protocol::SubAgentSource::Compact => "compact".to_string(),
                crate::protocol::SubAgentSource::ThreadSpawn { .. } => "collab_spawn".to_string(),
                crate::protocol::SubAgentSource::Other(label) => label.clone(),
            };
            if let Ok(val) = HeaderValue::from_str(&subagent) {
                extra_headers.insert("x-openai-subagent", val);
            }
        }
        extra_headers
    }

    /// Builds request telemetry for unary API calls (e.g., Compact endpoint).
    fn build_request_telemetry(otel_manager: &OtelManager) -> Arc<dyn RequestTelemetry> {
        let telemetry = Arc::new(ApiTelemetry::new(otel_manager.clone()));
        let request_telemetry: Arc<dyn RequestTelemetry> = telemetry;
        request_telemetry
    }
}

impl ModelClientSession {
    fn disable_websockets(&self) -> bool {
        self.client.state.disable_websockets.load(Ordering::Relaxed)
    }

    fn activate_http_fallback(&self, websocket_enabled: bool) -> bool {
        websocket_enabled
            && !self
                .client
                .state
                .disable_websockets
                .swap(true, Ordering::Relaxed)
    }

    fn responses_websocket_enabled(&self) -> bool {
        self.client.state.provider.supports_websockets
            && self.client.state.enable_responses_websockets
    }

    fn build_responses_request(prompt: &Prompt) -> Result<ApiPrompt> {
        let instructions = prompt.base_instructions.text.clone();
        let tools_json: Vec<Value> = create_tools_json_for_responses_api(&prompt.tools)?;
        Ok(build_api_prompt(prompt, instructions, tools_json))
    }

    #[allow(clippy::too_many_arguments)]
    fn build_responses_options(
        &self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        turn_metadata_header: Option<&str>,
        compression: Compression,
    ) -> ApiResponsesOptions {
        let turn_metadata_header =
            turn_metadata_header.and_then(|value| HeaderValue::from_str(value).ok());

        let default_reasoning_effort = model_info.default_reasoning_level;
        let reasoning = if model_info.supports_reasoning_summaries {
            Some(Reasoning {
                effort: effort.or(default_reasoning_effort),
                summary: if summary == ReasoningSummaryConfig::None {
                    None
                } else {
                    Some(summary)
                },
            })
        } else {
            None
        };

        let include = if reasoning.is_some() {
            vec!["reasoning.encrypted_content".to_string()]
        } else {
            Vec::new()
        };

        let verbosity = if model_info.support_verbosity {
            self.client
                .state
                .model_verbosity
                .or(model_info.default_verbosity)
        } else {
            if self.client.state.model_verbosity.is_some() {
                warn!(
                    "model_verbosity is set but ignored as the model does not support verbosity: {}",
                    model_info.slug
                );
            }
            None
        };

        let text = create_text_param_for_request(verbosity, &prompt.output_schema);
        let conversation_id = self.client.state.conversation_id.to_string();

        ApiResponsesOptions {
            reasoning,
            include,
            prompt_cache_key: Some(conversation_id.clone()),
            text,
            store_override: None,
            conversation_id: Some(conversation_id),
            session_source: Some(self.client.state.session_source.clone()),
            extra_headers: build_responses_headers(
                self.client.state.beta_features_header.as_deref(),
                Some(&self.turn_state),
                turn_metadata_header.as_ref(),
            ),
            compression,
            turn_state: Some(Arc::clone(&self.turn_state)),
        }
    }

    fn get_incremental_items(&self, input_items: &[ResponseItem]) -> Option<Vec<ResponseItem>> {
        // Checks whether the current request input is an incremental append to the previous request.
        // If items in the new request contain all the items from the previous request we build
        // a response.append request otherwise we start with a fresh response.create request.
        let previous_len = self.websocket_last_items.len();
        let can_append = previous_len > 0
            && input_items.starts_with(&self.websocket_last_items)
            && previous_len < input_items.len();
        if can_append {
            Some(input_items[previous_len..].to_vec())
        } else {
            None
        }
    }

    fn prepare_websocket_request(
        &self,
        model_slug: &str,
        api_prompt: &ApiPrompt,
        options: &ApiResponsesOptions,
    ) -> ResponsesWsRequest {
        if let Some(append_items) = self.get_incremental_items(&api_prompt.input) {
            return ResponsesWsRequest::ResponseAppend(ResponseAppendWsRequest {
                input: append_items,
            });
        }

        let ApiResponsesOptions {
            reasoning,
            include,
            prompt_cache_key,
            text,
            store_override,
            ..
        } = options;

        let store = store_override.unwrap_or(false);
        let payload = ResponseCreateWsRequest {
            model: model_slug.to_string(),
            instructions: api_prompt.instructions.clone(),
            input: api_prompt.input.clone(),
            tools: api_prompt.tools.clone(),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: api_prompt.parallel_tool_calls,
            reasoning: reasoning.clone(),
            store,
            stream: true,
            include: include.clone(),
            prompt_cache_key: prompt_cache_key.clone(),
            text: text.clone(),
        };

        ResponsesWsRequest::ResponseCreate(payload)
    }

    async fn websocket_connection(
        &mut self,
        otel_manager: &OtelManager,
        api_provider: codex_api::Provider,
        api_auth: CoreAuthProvider,
        options: &ApiResponsesOptions,
    ) -> std::result::Result<&ApiWebSocketConnection, ApiError> {
        let needs_new = match self.connection.as_ref() {
            Some(conn) => conn.is_closed().await,
            None => true,
        };

        if needs_new {
            let headers =
                build_websocket_connect_headers(options, self.client.state.include_timing_metrics);
            let websocket_telemetry = Self::build_websocket_telemetry(otel_manager);
            let new_conn: ApiWebSocketConnection =
                ApiWebSocketResponsesClient::new(api_provider, api_auth)
                    .connect(
                        headers,
                        options.turn_state.clone(),
                        Some(websocket_telemetry),
                    )
                    .await?;
            self.connection = Some(new_conn);
        }

        self.connection.as_ref().ok_or(ApiError::Stream(
            "websocket connection is unavailable".to_string(),
        ))
    }

    fn responses_request_compression(&self, auth: Option<&crate::auth::CodexAuth>) -> Compression {
        if self.client.state.enable_request_compression
            && auth.is_some_and(CodexAuth::is_chatgpt_auth)
            && self.client.state.provider.is_openai()
        {
            Compression::Zstd
        } else {
            Compression::None
        }
    }

    /// Streams a turn via the OpenAI Responses API.
    ///
    /// Handles SSE fixtures, reasoning summaries, verbosity, and the
    /// `text` controls used for output schemas.
    #[allow(clippy::too_many_arguments)]
    async fn stream_responses_api(
        &self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        otel_manager: &OtelManager,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        turn_metadata_header: Option<&str>,
    ) -> Result<ResponseStream> {
        if let Some(path) = &*CODEX_RS_SSE_FIXTURE {
            warn!(path, "Streaming from fixture");
            let stream = codex_api::stream_from_fixture(
                path,
                self.client.state.provider.stream_idle_timeout(),
            )
            .map_err(map_api_error)?;
            return Ok(map_response_stream(stream, otel_manager.clone()));
        }

        let auth_manager = self.client.state.auth_manager.clone();
        let api_prompt = Self::build_responses_request(prompt)?;

        let mut auth_recovery = auth_manager
            .as_ref()
            .map(super::auth::AuthManager::unauthorized_recovery);
        loop {
            let auth = match auth_manager.as_ref() {
                Some(manager) => manager.auth().await,
                None => None,
            };
            let api_provider = self
                .client
                .state
                .provider
                .to_api_provider(auth.as_ref().map(CodexAuth::auth_mode))?;
            let api_auth = auth_provider_from_auth(auth.clone(), &self.client.state.provider)?;
            let transport = ReqwestTransport::new(build_reqwest_client());
            let (request_telemetry, sse_telemetry) = Self::build_streaming_telemetry(otel_manager);
            let compression = self.responses_request_compression(auth.as_ref());

            let client = ApiResponsesClient::new(transport, api_provider, api_auth)
                .with_telemetry(Some(request_telemetry), Some(sse_telemetry));

            let options = self.build_responses_options(
                prompt,
                model_info,
                effort,
                summary,
                turn_metadata_header,
                compression,
            );

            let stream_result = client
                .stream_prompt(&model_info.slug, &api_prompt, options)
                .await;

            match stream_result {
                Ok(stream) => {
                    return Ok(map_response_stream(stream, otel_manager.clone()));
                }
                Err(ApiError::Transport(
                    unauthorized_transport @ TransportError::Http { status, .. },
                )) if status == StatusCode::UNAUTHORIZED => {
                    handle_unauthorized(unauthorized_transport, &mut auth_recovery).await?;
                    continue;
                }
                Err(err) => return Err(map_api_error(err)),
            }
        }
    }

    /// Streams a turn via the Responses API over WebSocket transport.
    #[allow(clippy::too_many_arguments)]
    async fn stream_responses_websocket(
        &mut self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        otel_manager: &OtelManager,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        turn_metadata_header: Option<&str>,
    ) -> Result<ResponseStream> {
        let auth_manager = self.client.state.auth_manager.clone();
        let api_prompt = Self::build_responses_request(prompt)?;

        let mut auth_recovery = auth_manager
            .as_ref()
            .map(super::auth::AuthManager::unauthorized_recovery);
        loop {
            let auth = match auth_manager.as_ref() {
                Some(manager) => manager.auth().await,
                None => None,
            };
            let api_provider = self
                .client
                .state
                .provider
                .to_api_provider(auth.as_ref().map(CodexAuth::auth_mode))?;
            let api_auth = auth_provider_from_auth(auth.clone(), &self.client.state.provider)?;
            let compression = self.responses_request_compression(auth.as_ref());

            let options = self.build_responses_options(
                prompt,
                model_info,
                effort,
                summary,
                turn_metadata_header,
                compression,
            );
            let request = self.prepare_websocket_request(&model_info.slug, &api_prompt, &options);

            let connection = match self
                .websocket_connection(
                    otel_manager,
                    api_provider.clone(),
                    api_auth.clone(),
                    &options,
                )
                .await
            {
                Ok(connection) => connection,
                Err(ApiError::Transport(
                    unauthorized_transport @ TransportError::Http { status, .. },
                )) if status == StatusCode::UNAUTHORIZED => {
                    handle_unauthorized(unauthorized_transport, &mut auth_recovery).await?;
                    continue;
                }
                Err(err) => return Err(map_api_error(err)),
            };

            let stream_result = connection
                .stream_request(request)
                .await
                .map_err(map_api_error)?;
            self.websocket_last_items = api_prompt.input.clone();

            return Ok(map_response_stream(stream_result, otel_manager.clone()));
        }
    }

    /// Streams a turn via the Anthropic Messages API.
    async fn stream_anthropic_api(
        &self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
    ) -> Result<ResponseStream> {
        use crate::client_common::ResponseStream;
        use codex_api::AnthropicAdapter;
        use codex_api::AnthropicStreamState;
        use codex_api::ProviderAdapter;

        let api_prompt = Self::build_responses_request(prompt)?;

        // Get API key
        let api_key = self
            .client
            .state
            .provider
            .api_key_with_auth(
                &self.client.state.codex_home,
                self.client.state.cli_auth_credentials_store_mode,
            )?
            .ok_or_else(|| CodexErr::Api("Missing ANTHROPIC_API_KEY".to_string()))?;

        let adapter = AnthropicAdapter::new();

        // Convert input items to JSON values
        let input_values = serialize_input_items(&api_prompt.input)?;

        // Build reasoning config (mirrors build_responses_options / Gemini path)
        let reasoning_value = if model_info.supports_reasoning_summaries {
            let reasoning = Reasoning {
                effort: effort.or(model_info.default_reasoning_level),
                summary: if summary == ReasoningSummaryConfig::None {
                    None
                } else {
                    Some(summary)
                },
            };
            serde_json::to_value(reasoning).ok()
        } else {
            None
        };

        // Build request body
        let body = adapter
            .build_request_body(
                &model_info.slug,
                &api_prompt.instructions,
                &input_values,
                &api_prompt.tools,
                &codex_api::RequestOptions {
                    parallel_tool_calls: api_prompt.parallel_tool_calls,
                    reasoning: reasoning_value,
                    ..Default::default()
                },
            )
            .map_err(|e| CodexErr::Api(e.to_string()))?;

        // Build URL
        let base_url = self
            .client
            .state
            .provider
            .base_url
            .as_deref()
            .unwrap_or("https://api.anthropic.com/v1");
        let url = format!(
            "{}{}",
            base_url,
            adapter.streaming_endpoint(&model_info.slug)
        );

        // Build request
        let client = build_reqwest_client();
        let mut request = client
            .post(&url)
            .header(
                adapter.auth_header_name(),
                adapter.format_auth_header(&api_key),
            )
            .json(&body);

        // Add extra headers
        for (name, value) in adapter.extra_headers().iter() {
            if let Ok(value_str) = value.to_str() {
                request = request.header(name.as_str(), value_str);
            }
        }

        let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent>>(1600);

        let idle_timeout = self.client.state.provider.stream_idle_timeout();

        tokio::spawn(async move {
            match request.send().await {
                Ok(response) => {
                    if !response.status().is_success() {
                        let status = response.status();
                        let body = response.text().await.unwrap_or_default();
                        let _ = tx_event
                            .send(Err(CodexErr::Api(format!(
                                "Anthropic API error {}: {}",
                                status, body
                            ))))
                            .await;
                        return;
                    }

                    let mut state = AnthropicStreamState::new();
                    let mut stream = response.bytes_stream();
                    let mut buffer = String::new();

                    use futures::StreamExt;

                    loop {
                        let chunk_result = tokio::time::timeout(idle_timeout, stream.next()).await;

                        match chunk_result {
                            Ok(Some(Ok(chunk))) => {
                                // Append chunk to buffer
                                if let Ok(text) = std::str::from_utf8(&chunk) {
                                    buffer.push_str(text);
                                }

                                // Process complete SSE events
                                // Anthropic format: event: <type>\ndata: <json>\n\n
                                while let Some(end_pos) =
                                    buffer.find("\n\n").or_else(|| buffer.find("\r\n\r\n"))
                                {
                                    let event_end = if buffer[end_pos..].starts_with("\r\n\r\n") {
                                        end_pos + 4
                                    } else {
                                        end_pos + 2
                                    };

                                    let event_str = buffer[..end_pos].to_string();
                                    buffer = buffer[event_end..].to_string();

                                    // Skip empty events
                                    if event_str.trim().is_empty() {
                                        continue;
                                    }

                                    // Parse SSE event - extract event type and data
                                    let mut event_type = String::new();
                                    let mut data = String::new();

                                    for line in event_str.lines() {
                                        if let Some(stripped) = line.strip_prefix("event: ") {
                                            event_type = stripped.to_string();
                                        } else if let Some(stripped) = line.strip_prefix("event:") {
                                            event_type = stripped.trim().to_string();
                                        } else if let Some(stripped) = line.strip_prefix("data: ") {
                                            data = stripped.to_string();
                                        } else if let Some(stripped) = line.strip_prefix("data:") {
                                            data = stripped.trim().to_string();
                                        }
                                    }

                                    // Skip if no event type or data
                                    if event_type.is_empty() || data.is_empty() {
                                        continue;
                                    }

                                    // Parse the event
                                    match codex_api::sse::anthropic::parse_anthropic_event(
                                        &event_type,
                                        &data,
                                        &mut state,
                                    ) {
                                        Ok(evts) => {
                                            for evt in evts {
                                                let is_completed =
                                                    matches!(evt, ResponseEvent::Completed { .. });
                                                if tx_event.send(Ok(evt)).await.is_err() {
                                                    return;
                                                }
                                                if is_completed {
                                                    return;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            let _ = tx_event
                                                .send(Err(CodexErr::Api(e.to_string())))
                                                .await;
                                            return;
                                        }
                                    }
                                }
                            }
                            Ok(Some(Err(e))) => {
                                let _ = tx_event
                                    .send(Err(CodexErr::Api(format!("Stream error: {}", e))))
                                    .await;
                                return;
                            }
                            Ok(None) => {
                                // Stream ended
                                let _ = tx_event
                                    .send(Ok(ResponseEvent::Completed {
                                        response_id: String::new(),
                                        token_usage: None,
                                    }))
                                    .await;
                                return;
                            }
                            Err(_) => {
                                let _ = tx_event
                                    .send(Err(CodexErr::Api("Stream timeout".to_string())))
                                    .await;
                                return;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx_event
                        .send(Err(CodexErr::Api(format!("Request failed: {}", e))))
                        .await;
                }
            }
        });

        Ok(ResponseStream { rx_event })
    }

    /// Streams a turn via the Gemini GenerateContent API.
    async fn stream_gemini_api(
        &self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
    ) -> Result<ResponseStream> {
        use crate::client_common::ResponseStream;
        use codex_api::GeminiAdapter;
        use codex_api::GeminiStreamState;
        use codex_api::ProviderAdapter;

        let api_prompt = Self::build_responses_request(prompt)?;

        // Get API key
        let api_key = self
            .client
            .state
            .provider
            .api_key_with_auth(
                &self.client.state.codex_home,
                self.client.state.cli_auth_credentials_store_mode,
            )?
            .ok_or_else(|| CodexErr::Api("Missing GOOGLE_API_KEY".to_string()))?;

        let adapter = GeminiAdapter::new();

        // Convert input items to JSON values
        let input_values = serialize_input_items(&api_prompt.input)?;

        // Build reasoning config for Gemini (mirrors build_responses_options logic)
        let reasoning_value = if model_info.supports_reasoning_summaries {
            let reasoning = Reasoning {
                effort: effort.or(model_info.default_reasoning_level),
                summary: if summary == ReasoningSummaryConfig::None {
                    None
                } else {
                    Some(summary)
                },
            };
            serde_json::to_value(reasoning).ok()
        } else {
            None
        };

        // Build request body
        let body = adapter
            .build_request_body(
                &model_info.slug,
                &api_prompt.instructions,
                &input_values,
                &api_prompt.tools,
                &codex_api::RequestOptions {
                    parallel_tool_calls: api_prompt.parallel_tool_calls,
                    reasoning: reasoning_value,
                    ..Default::default()
                },
            )
            .map_err(|e| CodexErr::Api(e.to_string()))?;

        // Build URL - Gemini uses ?alt=sse for streaming
        let base_url = self
            .client
            .state
            .provider
            .base_url
            .as_deref()
            .unwrap_or("https://generativelanguage.googleapis.com/v1beta");
        let endpoint = adapter.streaming_endpoint(&model_info.slug);
        let url = format!("{}{}?alt=sse", base_url, endpoint);

        // Build request with API key in header (not URL) to prevent leakage in error messages
        let client = build_reqwest_client();
        let request = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header(
                adapter.auth_header_name(),
                adapter.format_auth_header(&api_key),
            )
            .json(&body);

        let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent>>(1600);

        let idle_timeout = self.client.state.provider.stream_idle_timeout();

        tokio::spawn(async move {
            match request.send().await {
                Ok(response) => {
                    if !response.status().is_success() {
                        let status = response.status();
                        let body = response.text().await.unwrap_or_default();
                        let _ = tx_event
                            .send(Err(CodexErr::Api(format!(
                                "Gemini API error {}: {}",
                                status, body
                            ))))
                            .await;
                        return;
                    }

                    // Send Created event first
                    if tx_event.send(Ok(ResponseEvent::Created)).await.is_err() {
                        return;
                    }

                    let mut state = GeminiStreamState::new();
                    // Mark created as sent since we just sent it
                    state.created_sent = true;

                    // Read the response as a stream of bytes and parse SSE manually
                    // Gemini sends SSE in the format: data: {...json...}\n\n
                    let mut stream = response.bytes_stream();
                    let mut buffer = String::new();

                    use futures::StreamExt;

                    loop {
                        let chunk_result = tokio::time::timeout(idle_timeout, stream.next()).await;

                        match chunk_result {
                            Ok(Some(Ok(chunk))) => {
                                // Append chunk to buffer
                                if let Ok(text) = std::str::from_utf8(&chunk) {
                                    buffer.push_str(text);
                                }

                                // Process complete SSE events (separated by \n\n or \r\n\r\n)
                                while let Some(end_pos) =
                                    buffer.find("\n\n").or_else(|| buffer.find("\r\n\r\n"))
                                {
                                    let event_end = if buffer[end_pos..].starts_with("\r\n\r\n") {
                                        end_pos + 4
                                    } else {
                                        end_pos + 2
                                    };

                                    let event_str = buffer[..end_pos].to_string();
                                    buffer = buffer[event_end..].to_string();

                                    // Skip empty events
                                    if event_str.trim().is_empty() {
                                        continue;
                                    }

                                    // Parse SSE event - look for "data: " prefix
                                    let data = if let Some(data_line) =
                                        event_str.lines().find(|line| line.starts_with("data: "))
                                    {
                                        &data_line[6..] // Skip "data: " prefix
                                    } else if event_str.starts_with("data:") {
                                        event_str[5..].trim() // Handle "data:" without space
                                    } else {
                                        // Not a data event, skip
                                        continue;
                                    };

                                    // Skip [DONE] marker if present
                                    if data.trim() == "[DONE]" {
                                        let _ = tx_event
                                            .send(Ok(ResponseEvent::Completed {
                                                response_id: String::new(),
                                                token_usage: None,
                                            }))
                                            .await;
                                        return;
                                    }

                                    // Parse JSON and extract events
                                    match codex_api::sse::gemini::parse_gemini_chunk(
                                        data, &mut state,
                                    ) {
                                        Ok(evts) => {
                                            for evt in evts {
                                                // Skip duplicate Created events
                                                if matches!(evt, ResponseEvent::Created) {
                                                    continue;
                                                }
                                                let is_completed =
                                                    matches!(evt, ResponseEvent::Completed { .. });
                                                if tx_event.send(Ok(evt)).await.is_err() {
                                                    return;
                                                }
                                                if is_completed {
                                                    return;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            // Log parsing error but continue - might be a partial chunk
                                            tracing::debug!(
                                                "Gemini parse error (continuing): {}",
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                            Ok(Some(Err(e))) => {
                                let _ = tx_event
                                    .send(Err(CodexErr::Api(format!("Stream error: {}", e))))
                                    .await;
                                return;
                            }
                            Ok(None) => {
                                // Stream ended - process any remaining buffer
                                if !buffer.trim().is_empty() {
                                    if let Some(data_line) =
                                        buffer.lines().find(|line| line.starts_with("data: "))
                                    {
                                        let data = &data_line[6..];
                                        if data.trim() != "[DONE]" {
                                            if let Ok(evts) =
                                                codex_api::sse::gemini::parse_gemini_chunk(
                                                    data, &mut state,
                                                )
                                            {
                                                for evt in evts {
                                                    if matches!(evt, ResponseEvent::Created) {
                                                        continue;
                                                    }
                                                    let _ = tx_event.send(Ok(evt)).await;
                                                }
                                            }
                                        }
                                    }
                                }
                                // Send completion
                                let _ = tx_event
                                    .send(Ok(ResponseEvent::Completed {
                                        response_id: String::new(),
                                        token_usage: None,
                                    }))
                                    .await;
                                return;
                            }
                            Err(_) => {
                                let _ = tx_event
                                    .send(Err(CodexErr::Api("Stream timeout".to_string())))
                                    .await;
                                return;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx_event
                        .send(Err(CodexErr::Api(format!("Request failed: {}", e))))
                        .await;
                }
            }
        });

        Ok(ResponseStream { rx_event })
    }

    /// Builds request and SSE telemetry for streaming API calls.
    fn build_streaming_telemetry(
        otel_manager: &OtelManager,
    ) -> (Arc<dyn RequestTelemetry>, Arc<dyn SseTelemetry>) {
        let telemetry = Arc::new(ApiTelemetry::new(otel_manager.clone()));
        let request_telemetry: Arc<dyn RequestTelemetry> = telemetry.clone();
        let sse_telemetry: Arc<dyn SseTelemetry> = telemetry;
        (request_telemetry, sse_telemetry)
    }

    /// Builds telemetry for the Responses API WebSocket transport.
    fn build_websocket_telemetry(otel_manager: &OtelManager) -> Arc<dyn WebsocketTelemetry> {
        let telemetry = Arc::new(ApiTelemetry::new(otel_manager.clone()));
        let websocket_telemetry: Arc<dyn WebsocketTelemetry> = telemetry;
        websocket_telemetry
    }

    #[allow(clippy::too_many_arguments)]
    /// Streams a single model request within the current turn.
    ///
    /// The caller is responsible for passing per-turn settings explicitly (model selection,
    /// reasoning settings, telemetry context, and turn metadata). This method will prefer the
    /// Responses WebSocket transport when enabled and healthy, and will fall back to the HTTP
    /// Responses API transport otherwise.
    pub async fn stream(
        &mut self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        otel_manager: &OtelManager,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        turn_metadata_header: Option<&str>,
    ) -> Result<ResponseStream> {
        let wire_api = self.client.state.provider.wire_api;
        match wire_api {
            WireApi::Responses => {
                let websocket_enabled =
                    self.responses_websocket_enabled() && !self.disable_websockets();

                if websocket_enabled {
                    self.stream_responses_websocket(
                        prompt,
                        model_info,
                        otel_manager,
                        effort,
                        summary,
                        turn_metadata_header,
                    )
                    .await
                } else {
                    self.stream_responses_api(
                        prompt,
                        model_info,
                        otel_manager,
                        effort,
                        summary,
                        turn_metadata_header,
                    )
                    .await
                }
            }
            WireApi::AnthropicMessages => {
                self.stream_anthropic_api(prompt, model_info, effort, summary)
                    .await
            }
            WireApi::GeminiGenerate => {
                self.stream_gemini_api(prompt, model_info, effort, summary)
                    .await
            }
        }
    }

    /// Permanently disables WebSockets for this Codex session and resets WebSocket state.
    ///
    /// This is used after exhausting the provider retry budget, to force subsequent requests onto
    /// the HTTP transport. Returns `true` if this call activated fallback, or `false` if fallback
    /// was already active.
    pub(crate) fn try_switch_fallback_transport(&mut self, otel_manager: &OtelManager) -> bool {
        let websocket_enabled = self.responses_websocket_enabled();
        let activated = self.activate_http_fallback(websocket_enabled);
        if activated {
            warn!("falling back to HTTP");
            otel_manager.counter(
                "codex.transport.fallback_to_http",
                1,
                &[("from_wire_api", "responses_websocket")],
            );

            self.connection = None;
            self.websocket_last_items.clear();
        }
        activated
    }
}

/// Serializes input items with proper error handling.
///
/// Unlike `filter_map(...ok())`, this returns an error if any item fails to serialize,
/// preventing incomplete prompts from being sent silently.
fn serialize_input_items(input: &[ResponseItem]) -> Result<Vec<Value>> {
    input
        .iter()
        .map(|item| {
            serde_json::to_value(item)
                .map_err(|e| CodexErr::Api(format!("Failed to serialize input item: {e}")))
        })
        .collect()
}

/// Adapts the core `Prompt` type into the `codex-api` payload shape.
fn build_api_prompt(prompt: &Prompt, instructions: String, tools_json: Vec<Value>) -> ApiPrompt {
    ApiPrompt {
        instructions,
        input: prompt.get_formatted_input(),
        tools: tools_json,
        parallel_tool_calls: prompt.parallel_tool_calls,
        output_schema: prompt.output_schema.clone(),
    }
}

/// Builds the extra headers attached to Responses API requests.
///
/// These headers implement Codex-specific conventions:
///
/// - `x-codex-beta-features`: comma-separated beta feature keys enabled for the session.
/// - `x-codex-turn-state`: sticky routing token captured earlier in the turn.
/// - `x-codex-turn-metadata`: optional per-turn metadata for observability.
fn build_responses_headers(
    beta_features_header: Option<&str>,
    turn_state: Option<&Arc<OnceLock<String>>>,
    turn_metadata_header: Option<&HeaderValue>,
) -> ApiHeaderMap {
    let mut headers = ApiHeaderMap::new();
    if let Some(value) = beta_features_header
        && !value.is_empty()
        && let Ok(header_value) = HeaderValue::from_str(value)
    {
        headers.insert("x-codex-beta-features", header_value);
    }
    if let Some(turn_state) = turn_state
        && let Some(state) = turn_state.get()
        && let Ok(header_value) = HeaderValue::from_str(state)
    {
        headers.insert(X_CODEX_TURN_STATE_HEADER, header_value);
    }
    if let Some(header_value) = turn_metadata_header {
        headers.insert(X_CODEX_TURN_METADATA_HEADER, header_value.clone());
    }
    headers
}

fn build_websocket_connect_headers(
    options: &ApiResponsesOptions,
    include_timing_metrics: bool,
) -> ApiHeaderMap {
    let mut headers = options.extra_headers.clone();
    headers.extend(build_conversation_headers(options.conversation_id.clone()));
    headers.insert(
        OPENAI_BETA_HEADER,
        HeaderValue::from_static(OPENAI_BETA_RESPONSES_WEBSOCKETS),
    );
    if include_timing_metrics {
        headers.insert(
            X_RESPONSESAPI_INCLUDE_TIMING_METRICS_HEADER,
            HeaderValue::from_static("true"),
        );
    }
    headers
}

fn map_response_stream<S>(api_stream: S, otel_manager: OtelManager) -> ResponseStream
where
    S: futures::Stream<Item = std::result::Result<ResponseEvent, ApiError>>
        + Unpin
        + Send
        + 'static,
{
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent>>(1600);

    tokio::spawn(async move {
        let mut logged_error = false;
        let mut api_stream = api_stream;
        while let Some(event) = api_stream.next().await {
            match event {
                Ok(ResponseEvent::Completed {
                    response_id,
                    token_usage,
                }) => {
                    if let Some(usage) = &token_usage {
                        otel_manager.sse_event_completed(
                            usage.input_tokens,
                            usage.output_tokens,
                            Some(usage.cached_input_tokens),
                            Some(usage.reasoning_output_tokens),
                            usage.total_tokens,
                        );
                    }
                    if tx_event
                        .send(Ok(ResponseEvent::Completed {
                            response_id,
                            token_usage,
                        }))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Ok(event) => {
                    if tx_event.send(Ok(event)).await.is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let mapped = map_api_error(err);
                    if !logged_error {
                        otel_manager.see_event_completed_failed(&mapped);
                        logged_error = true;
                    }
                    if tx_event.send(Err(mapped)).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    ResponseStream { rx_event }
}

/// Handles a 401 response by optionally refreshing ChatGPT tokens once.
///
/// When refresh succeeds, the caller should retry the API call; otherwise
/// the mapped `CodexErr` is returned to the caller.
async fn handle_unauthorized(
    transport: TransportError,
    auth_recovery: &mut Option<UnauthorizedRecovery>,
) -> Result<()> {
    if let Some(recovery) = auth_recovery
        && recovery.has_next()
    {
        return match recovery.next().await {
            Ok(_) => Ok(()),
            Err(RefreshTokenError::Permanent(failed)) => Err(CodexErr::RefreshTokenFailed(failed)),
            Err(RefreshTokenError::Transient(other)) => Err(CodexErr::Io(other)),
        };
    }

    Err(map_api_error(ApiError::Transport(transport)))
}

struct ApiTelemetry {
    otel_manager: OtelManager,
}

impl ApiTelemetry {
    fn new(otel_manager: OtelManager) -> Self {
        Self { otel_manager }
    }
}

impl RequestTelemetry for ApiTelemetry {
    fn on_request(
        &self,
        attempt: u64,
        status: Option<HttpStatusCode>,
        error: Option<&TransportError>,
        duration: Duration,
    ) {
        let error_message = error.map(std::string::ToString::to_string);
        self.otel_manager.record_api_request(
            attempt,
            status.map(|s| s.as_u16()),
            error_message.as_deref(),
            duration,
        );
    }
}

impl SseTelemetry for ApiTelemetry {
    fn on_sse_poll(
        &self,
        result: &std::result::Result<
            Option<std::result::Result<Event, EventStreamError<TransportError>>>,
            tokio::time::error::Elapsed,
        >,
        duration: Duration,
    ) {
        self.otel_manager.log_sse_event(result, duration);
    }
}

impl WebsocketTelemetry for ApiTelemetry {
    fn on_ws_request(&self, duration: Duration, error: Option<&ApiError>) {
        let error_message = error.map(std::string::ToString::to_string);
        self.otel_manager
            .record_websocket_request(duration, error_message.as_deref());
    }

    fn on_ws_event(
        &self,
        result: &std::result::Result<Option<std::result::Result<Message, Error>>, ApiError>,
        duration: Duration,
    ) {
        self.otel_manager.record_websocket_event(result, duration);
    }
}
