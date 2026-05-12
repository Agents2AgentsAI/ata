//! Streams a turn via the Google Gemini `generateContent` API.
//!
//! Same shape as `client/anthropic.rs`: glue between ATA's
//! `ModelClientSession` / `Prompt` API and the codex-api `GeminiAdapter` +
//! `GeminiStreamState` + `parse_gemini_chunk`.
//!
//! Two transport flavours live behind one entry point:
//!
//! - **API key path** — public Gemini Generative Language API
//!   (`generativelanguage.googleapis.com`), authed with `?key=<API_KEY>`.
//!   Used when an API key is found via `ModelProviderApiKeyExt::api_key_with_auth`
//!   (env var or per-provider stored credential).
//! - **Code Assist OAuth path** — `cloudcode-pa.googleapis.com/v1internal:streamGenerateContent`,
//!   authed with `Authorization: Bearer <access_token>` and a request envelope that
//!   wraps the standard Gemini body with `model` / `project`. Selected when
//!   `resolve_gemini_auth_source` returns an OAuth credential and no API key is
//!   configured. Tokens are refreshed via `gemini_oauth::ensure_gemini_oauth_context_with_refresh`
//!   (which also resolves / onboards the Code Assist project id).
//!
//! Downstream parsing (`parse_gemini_chunk` + `GeminiStreamState`) is shared
//! between both paths.

use std::time::Duration;

use codex_api::GeminiAdapter;
use codex_api::GeminiStreamState;
use codex_api::ProviderAdapter;
use codex_api::ResponseEvent;
use codex_api::parse_gemini_chunk;
use codex_login::default_client::build_reqwest_client;
use codex_login::default_client::get_codex_user_agent;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_tools::ToolSpec;
use codex_tools::create_tools_json_for_responses_api;
use serde_json::Value;
use serde_json::json;

use super::ModelClientSession;
use super::provider_streaming::ParseSseEventResult;
use super::provider_streaming::build_reasoning_value;
use super::provider_streaming::extract_sse_data_line;
use super::provider_streaming::filter_out_created;
use super::provider_streaming::map_api_err_to_codex_err;
use super::provider_streaming::map_status_error;
use super::provider_streaming::serialize_input_items;
use super::provider_streaming::spawn_provider_sse_stream;
use super::provider_streaming::spawn_provider_sse_stream_from_response;
use crate::auth::GOOGLE_API_KEY_ENV_VAR;
use crate::auth::GeminiAuthSource;
use crate::auth::ModelProviderApiKeyExt;
use crate::auth::ProviderOauthCredential;
use crate::auth::gemini_oauth;
use crate::auth::resolve_gemini_auth_source;
use crate::client_common::Prompt;
use crate::client_common::ResponseStream;

const GEMINI_DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Streams a turn via the Gemini `generateContent` API (or Gemini Code Assist
/// `streamGenerateContent` when an OAuth credential is configured).
pub(super) async fn stream_gemini_api(
    session: &ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
    summary: ReasoningSummaryConfig,
) -> Result<ResponseStream> {
    let state = session.client().state();
    let provider_info = state.provider.info().clone();

    // Build the standard Gemini request body once — both transports reuse it
    // (the Code Assist branch wraps it in an envelope below).
    let adapter = GeminiAdapter::new();
    let input = prompt.get_formatted_input();
    let input_values = serialize_input_items(&input)?;
    let tools_array = create_tools_json_for_responses_api(prompt_tools(prompt))
        .map_err(|e| CodexErr::Api(format!("failed to serialize tools: {e}")))?;
    let formatted_tools = adapter
        .format_tools(&tools_array)
        .map_err(|e| CodexErr::Api(e.to_string()))?;
    let reasoning_value = build_reasoning_value(model_info, effort, summary);
    let body = adapter
        .build_request_body(
            &model_info.slug,
            prompt_base_instructions(prompt),
            &input_values,
            &formatted_tools,
            &codex_api::RequestOptions {
                parallel_tool_calls: prompt_parallel_tool_calls(prompt),
                reasoning: reasoning_value,
                ..Default::default()
            },
        )
        .map_err(|e| CodexErr::Api(e.to_string()))?;

    // If there is no API key configured anywhere, fall back to Code Assist
    // OAuth when an OAuth credential is present. API-key wins ties so users
    // can keep a Gemini API key alongside an unrelated Google login.
    let auth_source =
        resolve_gemini_auth_source(&state.codex_home, state.cli_auth_credentials_store_mode);
    if let GeminiAuthSource::Oauth(oauth_credential) = auth_source
        && provider_info.api_key()?.is_none()
        && crate::auth::get_provider_api_key(
            &state.codex_home,
            crate::auth::PROVIDER_GEMINI,
            state.cli_auth_credentials_store_mode,
        )
        .is_none()
    {
        return stream_gemini_code_assist(
            session,
            model_info,
            body,
            &input_values,
            oauth_credential,
        )
        .await;
    }

    // API-key path: prefer the standard provider api_key resolver (env var +
    // per-provider stored credential), falling back to a clear error.
    let api_key = provider_info
        .api_key_with_auth(&state.codex_home, state.cli_auth_credentials_store_mode)?
        .ok_or_else(|| {
            CodexErr::Api(format!(
                "Missing {GOOGLE_API_KEY_ENV_VAR} env var (and no stored Gemini credential found)"
            ))
        })?;

    let base_url = provider_info
        .base_url
        .as_deref()
        .unwrap_or(GEMINI_DEFAULT_BASE_URL);
    let endpoint = adapter.streaming_endpoint(&model_info.slug);
    let url = format!(
        "{}{}?alt=sse&key={}",
        base_url.trim_end_matches('/'),
        endpoint,
        urlencoding::encode(&api_key)
    );

    let client = build_reqwest_client();
    let mut request = client.post(&url).json(&body);

    for (name, value) in adapter.extra_headers_for_input(&input_values).iter() {
        if let Ok(value_str) = value.to_str() {
            request = request.header(name.as_str(), value_str);
        }
    }

    Ok(spawn_provider_sse_stream(
        request,
        DEFAULT_IDLE_TIMEOUT,
        "Gemini",
        GeminiStreamState::default(),
        Vec::new(),
        |event_str, state| {
            let data = match extract_sse_data_line(event_str) {
                Some(data) => data,
                None => return ParseSseEventResult::Continue,
            };
            if data.trim().is_empty() {
                return ParseSseEventResult::Continue;
            }
            match parse_gemini_chunk(&data, state) {
                Ok(events) => ParseSseEventResult::Emit(events),
                Err(err) => ParseSseEventResult::Fatal(map_api_err_to_codex_err(err)),
            }
        },
        |_buffer, _state| ParseSseEventResult::Continue,
    ))
}

/// Streams a turn via Gemini Code Assist (`v1internal:streamGenerateContent`).
///
/// Wraps the standard Gemini body in the Code Assist envelope (`model` /
/// `project` / `request`), refreshes/onboards via `gemini_oauth`, and falls
/// back to `force_refresh_gemini_oauth_context` once on `401 Unauthorized` so
/// transiently expired access tokens self-heal.
async fn stream_gemini_code_assist(
    session: &ModelClientSession,
    model_info: &ModelInfo,
    request_body: Value,
    input_values: &[Value],
    oauth_credential: ProviderOauthCredential,
) -> Result<ResponseStream> {
    let state = session.client().state();

    let context = gemini_oauth::ensure_gemini_oauth_context(
        &state.codex_home,
        state.cli_auth_credentials_store_mode,
        oauth_credential.clone(),
    )
    .await?;

    let url = build_code_assist_stream_url();
    let envelope =
        wrap_code_assist_request(&model_info.slug, &context.project_id, request_body.clone());

    let response = post_code_assist_stream(&url, &context.access_token, &envelope, input_values)
        .await
        .map_err(|err| CodexErr::Api(format!("Request failed: {err}")))?;

    let response = if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        // One-shot retry after a forced token refresh — handles the case where
        // the cached access token expired between cache check and request.
        let refreshed = gemini_oauth::force_refresh_gemini_oauth_context(
            &state.codex_home,
            state.cli_auth_credentials_store_mode,
            oauth_credential,
        )
        .await?;
        let envelope =
            wrap_code_assist_request(&model_info.slug, &refreshed.project_id, request_body);
        post_code_assist_stream(&url, &refreshed.access_token, &envelope, input_values)
            .await
            .map_err(|err| CodexErr::Api(format!("Retry request failed: {err}")))?
    } else {
        response
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(map_status_error("Gemini Code Assist", status, &body));
    }

    // Reuse parse_gemini_chunk by unwrapping Code Assist's `{ "response": {...} }`
    // envelope before passing each SSE chunk to the standard parser.
    let mut stream_state = GeminiStreamState::new();
    stream_state.created_sent = true;

    Ok(spawn_provider_sse_stream_from_response(
        response,
        DEFAULT_IDLE_TIMEOUT,
        stream_state,
        vec![ResponseEvent::Created],
        |event_str, state| {
            let Some(data) = extract_sse_data_line(event_str) else {
                return ParseSseEventResult::Continue;
            };
            parse_code_assist_sse_data(&data, state, false)
        },
        |buffer, state| {
            let Some(data) = extract_sse_data_line(buffer) else {
                return ParseSseEventResult::Continue;
            };
            parse_code_assist_sse_data(&data, state, true)
        },
    ))
}

async fn post_code_assist_stream(
    url: &str,
    access_token: &str,
    body: &Value,
    input_values: &[Value],
) -> std::result::Result<reqwest::Response, reqwest::Error> {
    let client = build_reqwest_client();
    let mut request = client
        .post(url)
        .header("Accept", "text/event-stream")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("User-Agent", get_codex_user_agent())
        .json(body);
    let adapter = GeminiAdapter::new();
    for (name, value) in adapter.extra_headers_for_input(input_values).iter() {
        if let Ok(value_str) = value.to_str() {
            request = request.header(name.as_str(), value_str);
        }
    }
    request.send().await
}

fn build_code_assist_stream_url() -> String {
    let url = gemini_oauth::code_assist_method_url("streamGenerateContent");
    match reqwest::Url::parse(&url) {
        Ok(mut parsed) => {
            parsed.query_pairs_mut().append_pair("alt", "sse");
            parsed.to_string()
        }
        Err(_) => format!("{url}?alt=sse"),
    }
}

fn parse_code_assist_sse_data(
    data: &str,
    state: &mut GeminiStreamState,
    ignore_parse_errors: bool,
) -> ParseSseEventResult {
    if data.trim() == "[DONE]" {
        return ParseSseEventResult::Continue;
    }

    let normalized = unwrap_code_assist_response_envelope(data);
    match parse_gemini_chunk(&normalized, state) {
        Ok(events) => {
            let events = filter_out_created(events);
            if events.is_empty() {
                ParseSseEventResult::Continue
            } else {
                ParseSseEventResult::Emit(events)
            }
        }
        Err(_) if ignore_parse_errors => ParseSseEventResult::Continue,
        Err(err) => ParseSseEventResult::Fatal(map_api_err_to_codex_err(err)),
    }
}

/// Code Assist wraps each SSE chunk in `{ "response": <gemini_chunk> }`.
/// Strip that wrapper so `parse_gemini_chunk` (which expects the bare Gemini
/// chunk shape) keeps working.
fn unwrap_code_assist_response_envelope(data: &str) -> String {
    let Ok(parsed) = serde_json::from_str::<Value>(data) else {
        return data.to_string();
    };
    match parsed.get("response") {
        Some(inner) => inner.to_string(),
        None => data.to_string(),
    }
}

fn wrap_code_assist_request(model: &str, project: &str, request_body: Value) -> Value {
    let mapped_model = map_code_assist_model(model);

    if let Value::Object(ref envelope) = request_body
        && envelope.contains_key("request")
        && envelope.contains_key("model")
    {
        let mut envelope = request_body;
        if let Some(map) = envelope.as_object_mut() {
            if !project.is_empty() && !map.contains_key("project") {
                map.insert("project".to_string(), Value::String(project.to_string()));
            }
            if let Some(model_value) = map.get_mut("model")
                && let Some(existing) = model_value.as_str()
            {
                *model_value = Value::String(map_code_assist_model(existing).to_string());
            }
        }
        return envelope;
    }

    if project.is_empty() {
        json!({
            "model": mapped_model,
            "request": request_body,
        })
    } else {
        json!({
            "project": project,
            "model": mapped_model,
            "request": request_body,
        })
    }
}

/// Code Assist refuses some Gemini-API model aliases (image-only previews,
/// pre-release Gemini 3 names). Map them down to the closest accepted slug.
fn map_code_assist_model(model: &str) -> &str {
    match model {
        "gemini-2.5-flash-image-preview" => "gemini-2.5-flash",
        "gemini-2.0-flash-exp-image-generation" => "gemini-2.0-flash",
        "gemini-3-flash-preview" | "gemini-3-flash" => "gemini-2.5-flash",
        "gemini-3-pro-preview" | "gemini-3-pro" => "gemini-2.5-pro",
        "gemini-3.1-pro-preview" | "gemini-3.1-pro" => "gemini-2.5-pro",
        _ => model,
    }
}

fn prompt_tools(prompt: &Prompt) -> &[ToolSpec] {
    &prompt.tools
}

fn prompt_base_instructions(prompt: &Prompt) -> &str {
    &prompt.base_instructions.text
}

fn prompt_parallel_tool_calls(prompt: &Prompt) -> bool {
    prompt.parallel_tool_calls
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn wrap_request_adds_project_and_model() {
        let body = json!({"contents": []});
        let wrapped = wrap_code_assist_request("gemini-2.5-pro", "my-project", body.clone());
        assert_eq!(wrapped["project"], "my-project");
        assert_eq!(wrapped["model"], "gemini-2.5-pro");
        assert_eq!(wrapped["request"], body);
    }

    #[test]
    fn wrap_request_omits_project_when_empty() {
        let body = json!({"contents": []});
        let wrapped = wrap_code_assist_request("gemini-2.5-pro", "", body.clone());
        assert!(wrapped.get("project").is_none());
        assert_eq!(wrapped["model"], "gemini-2.5-pro");
        assert_eq!(wrapped["request"], body);
    }

    #[test]
    fn wrap_request_preserves_existing_envelope_and_remaps_model() {
        let already_wrapped = json!({
            "model": "gemini-3-pro",
            "request": {"contents": []},
        });
        let wrapped = wrap_code_assist_request("ignored", "my-project", already_wrapped);
        assert_eq!(wrapped["project"], "my-project");
        // Existing model is remapped, not the call-site model.
        assert_eq!(wrapped["model"], "gemini-2.5-pro");
    }

    #[test]
    fn map_code_assist_model_remaps_unsupported_aliases() {
        assert_eq!(
            map_code_assist_model("gemini-2.5-flash-image-preview"),
            "gemini-2.5-flash"
        );
        assert_eq!(map_code_assist_model("gemini-3-pro"), "gemini-2.5-pro");
        assert_eq!(map_code_assist_model("gemini-2.5-pro"), "gemini-2.5-pro");
    }

    #[test]
    fn unwrap_envelope_strips_response_wrapper() {
        let data = r#"{"response":{"candidates":[{"content":{"parts":[]}}]}}"#;
        let unwrapped = unwrap_code_assist_response_envelope(data);
        let parsed: Value = serde_json::from_str(&unwrapped).expect("valid json");
        assert!(parsed.get("candidates").is_some());
        assert!(parsed.get("response").is_none());
    }

    #[test]
    fn unwrap_envelope_passthrough_when_no_wrapper() {
        let data = r#"{"candidates":[]}"#;
        let unwrapped = unwrap_code_assist_response_envelope(data);
        assert_eq!(unwrapped, data);
    }
}
