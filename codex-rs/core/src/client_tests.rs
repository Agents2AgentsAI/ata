use super::AuthRequestTelemetryContext;
use super::ModelClient;
use super::PendingUnauthorizedRetry;
use super::Prompt;
use super::UnauthorizedRecoveryExecution;
use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::tools::spec::JsonSchema;
use codex_otel::SessionTelemetry;
use codex_protocol::ThreadId;
use codex_protocol::models::BaseInstructions;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use pretty_assertions::assert_eq;
use serde_json::json;

fn test_model_client(session_source: SessionSource) -> ModelClient {
    test_model_client_with_home(session_source, std::path::PathBuf::from("/tmp"))
}

fn test_model_client_with_home(
    session_source: SessionSource,
    codex_home: std::path::PathBuf,
) -> ModelClient {
    let provider = crate::model_provider_info::create_oss_provider_with_base_url(
        "https://example.com/v1",
        crate::model_provider_info::WireApi::Responses,
    );
    ModelClient::new(
        None,
        ThreadId::new(),
        provider,
        session_source,
        None,
        false,
        false,
        false,
        None,
        codex_home,
        Default::default(),
    )
}

fn test_model_info() -> ModelInfo {
    serde_json::from_value(json!({
        "slug": "gpt-test",
        "display_name": "gpt-test",
        "description": "desc",
        "default_reasoning_level": "medium",
        "supported_reasoning_levels": [
            {"effort": "medium", "description": "medium"}
        ],
        "shell_type": "shell_command",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 1,
        "upgrade": null,
        "base_instructions": "base instructions",
        "model_messages": null,
        "supports_reasoning_summaries": false,
        "support_verbosity": false,
        "default_verbosity": null,
        "apply_patch_tool_type": null,
        "truncation_policy": {"mode": "bytes", "limit": 10000},
        "supports_parallel_tool_calls": false,
        "supports_image_detail_original": false,
        "context_window": 272000,
        "auto_compact_token_limit": null,
        "experimental_supported_tools": []
    }))
    .expect("deserialize test model info")
}

fn test_session_telemetry() -> SessionTelemetry {
    SessionTelemetry::new(
        ThreadId::new(),
        "gpt-test",
        "gpt-test",
        None,
        None,
        None,
        "test-originator".to_string(),
        false,
        "test-terminal".to_string(),
        SessionSource::Cli,
    )
}

#[test]
fn build_subagent_headers_sets_other_subagent_label() {
    let client = test_model_client(SessionSource::SubAgent(SubAgentSource::Other(
        "memory_consolidation".to_string(),
    )));
    let headers = client.build_subagent_headers();
    let value = headers
        .get("x-openai-subagent")
        .and_then(|value| value.to_str().ok());
    assert_eq!(value, Some("memory_consolidation"));
}

#[tokio::test]
async fn summarize_memories_returns_empty_for_empty_input() {
    let client = test_model_client(SessionSource::Cli);
    let model_info = test_model_info();
    let session_telemetry = test_session_telemetry();

    let output = client
        .summarize_memories(Vec::new(), &model_info, None, &session_telemetry)
        .await
        .expect("empty summarize request should succeed");
    assert_eq!(output.len(), 0);
}

#[test]
fn auth_request_telemetry_context_tracks_attached_auth_and_retry_phase() {
    let auth_context = AuthRequestTelemetryContext::new(
        Some(crate::auth::AuthMode::Chatgpt),
        &crate::api_bridge::CoreAuthProvider::for_test(Some("access-token"), Some("workspace-123")),
        PendingUnauthorizedRetry::from_recovery(UnauthorizedRecoveryExecution {
            mode: "managed",
            phase: "refresh_token",
        }),
    );

    assert_eq!(auth_context.auth_mode, Some("Chatgpt"));
    assert!(auth_context.auth_header_attached);
    assert_eq!(auth_context.auth_header_name, Some("authorization"));
    assert!(auth_context.retry_after_unauthorized);
    assert_eq!(auth_context.recovery_mode, Some("managed"));
    assert_eq!(auth_context.recovery_phase, Some("refresh_token"));
}

#[tokio::test]
async fn build_responses_request_logs_full_request_jsonl() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let client = test_model_client_with_home(SessionSource::Cli, tempdir.path().to_path_buf());
    let session = client.new_session();
    let model_info = test_model_info();
    let client_setup = client.current_client_setup().await.expect("client setup");
    let prompt = Prompt {
        input: Vec::new(),
        tools: vec![ToolSpec::Function(ResponsesApiTool {
            name: "present_reading_view".to_string(),
            description: "Reading view tool description".to_string(),
            strict: false,
            parameters: JsonSchema::Object {
                properties: Default::default(),
                required: None,
                additional_properties: Some(false.into()),
            },
            output_schema: None,
            defer_loading: None,
        })],
        parallel_tool_calls: false,
        base_instructions: BaseInstructions {
            text: "base instructions".to_string(),
        },
        personality: None,
        output_schema: None,
    };

    let request = session
        .build_responses_request(
            &client_setup.api_provider,
            &prompt,
            &model_info,
            None,
            codex_protocol::config_types::ReasoningSummary::None,
            None,
            Some("turn-meta"),
            "responses_http",
            false,
        )
        .expect("build request");

    assert_eq!(request.model, "gpt-test");

    let log_path = tempdir.path().join("logs/llm-requests.jsonl");
    let log_contents = std::fs::read_to_string(log_path).expect("read log file");
    assert!(
        log_contents.contains("\"transport\":\"responses_http\""),
        "expected transport in log entry: {log_contents}"
    );
    assert!(
        log_contents.contains("\"turn_metadata_header\":\"turn-meta\""),
        "expected turn metadata in log entry: {log_contents}"
    );
    assert!(
        log_contents.contains("Reading view tool description"),
        "expected serialized tool description in log entry: {log_contents}"
    );
}
