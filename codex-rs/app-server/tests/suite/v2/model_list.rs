use std::time::Duration;

use anyhow::Result;
use anyhow::anyhow;
use app_test_support::McpProcess;
use app_test_support::to_response;
use app_test_support::write_models_cache;
use codex_app_server_protocol::JSONRPCError;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::Model;
use codex_app_server_protocol::ModelListParams;
use codex_app_server_protocol::ModelListResponse;
use codex_app_server_protocol::ReasoningEffortOption;
use codex_app_server_protocol::RequestId;
use codex_protocol::openai_models::InputModality;
use codex_protocol::openai_models::ReasoningEffort;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tokio::time::timeout;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const INVALID_REQUEST_ERROR_CODE: i64 = -32600;

#[tokio::test]
async fn list_models_returns_all_models_with_large_limit() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(100),
            cursor: None,
            include_hidden: None,
        })
        .await?;

    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let ModelListResponse {
        data: items,
        next_cursor,
    } = to_response::<ModelListResponse>(response)?;

    let expected_models = vec![
        Model {
            id: "gpt-5.2-codex".to_string(),
            model: "gpt-5.2-codex".to_string(),
            upgrade: None,
            display_name: "gpt-5.2-codex".to_string(),
            description: "Latest frontier agentic coding model.".to_string(),
            hidden: false,
            supported_reasoning_efforts: vec![
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Low,
                    description: "Fast responses with lighter reasoning".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Medium,
                    description: "Balances speed and reasoning depth for everyday tasks"
                        .to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::High,
                    description: "Greater reasoning depth for complex problems".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::XHigh,
                    description: "Extra high reasoning depth for complex problems".to_string(),
                },
            ],
            default_reasoning_effort: ReasoningEffort::Medium,
            input_modalities: vec![InputModality::Text, InputModality::Image],
            supports_personality: false,
            is_default: true,
        },
        Model {
            id: "gpt-5.1-codex-max".to_string(),
            model: "gpt-5.1-codex-max".to_string(),
            upgrade: Some("gpt-5.2-codex".to_string()),
            display_name: "gpt-5.1-codex-max".to_string(),
            description: "Ata-optimized flagship for deep and fast reasoning.".to_string(),
            hidden: false,
            supported_reasoning_efforts: vec![
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Low,
                    description: "Fast responses with lighter reasoning".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Medium,
                    description: "Balances speed and reasoning depth for everyday tasks"
                        .to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::High,
                    description: "Greater reasoning depth for complex problems".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::XHigh,
                    description: "Extra high reasoning depth for complex problems".to_string(),
                },
            ],
            default_reasoning_effort: ReasoningEffort::Medium,
            input_modalities: vec![InputModality::Text, InputModality::Image],
            supports_personality: false,
            is_default: false,
        },
        Model {
            id: "gpt-5.1-codex-mini".to_string(),
            model: "gpt-5.1-codex-mini".to_string(),
            upgrade: Some("gpt-5.2-codex".to_string()),
            display_name: "gpt-5.1-codex-mini".to_string(),
            description: "Optimized for codex. Cheaper, faster, but less capable.".to_string(),
            hidden: false,
            supported_reasoning_efforts: vec![
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Medium,
                    description: "Dynamically adjusts reasoning based on the task".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::High,
                    description: "Maximizes reasoning depth for complex or ambiguous problems"
                        .to_string(),
                },
            ],
            default_reasoning_effort: ReasoningEffort::Medium,
            input_modalities: vec![InputModality::Text, InputModality::Image],
            supports_personality: false,
            is_default: false,
        },
        Model {
            id: "gpt-5.2".to_string(),
            model: "gpt-5.2".to_string(),
            upgrade: Some("gpt-5.2-codex".to_string()),
            display_name: "gpt-5.2".to_string(),
            description:
                "Latest frontier model with improvements across knowledge, reasoning and coding"
                    .to_string(),
            hidden: false,
            supported_reasoning_efforts: vec![
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Low,
                    description: "Balances speed with some reasoning; useful for straightforward \
                                   queries and short explanations"
                        .to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Medium,
                    description: "Provides a solid balance of reasoning depth and latency for \
                         general-purpose tasks"
                        .to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::High,
                    description: "Maximizes reasoning depth for complex or ambiguous problems"
                        .to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::XHigh,
                    description: "Extra high reasoning depth for complex problems".to_string(),
                },
            ],
            default_reasoning_effort: ReasoningEffort::Medium,
            input_modalities: vec![InputModality::Text, InputModality::Image],
            supports_personality: false,
            is_default: false,
        },
        // Anthropic Claude models
        Model {
            id: "claude-sonnet-4-5".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            upgrade: None,
            display_name: "Claude Sonnet 4-5".to_string(),
            description: "Anthropic's balanced model for coding tasks.".to_string(),
            hidden: false,
            supported_reasoning_efforts: vec![
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Low,
                    description: "Fast responses with lighter reasoning".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Medium,
                    description: "Balanced reasoning for everyday tasks".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::High,
                    description: "Greater reasoning depth for complex problems".to_string(),
                },
            ],
            default_reasoning_effort: ReasoningEffort::Medium,
            input_modalities: vec![InputModality::Text, InputModality::Image],
            supports_personality: false,
            is_default: false,
        },
        Model {
            id: "claude-opus-4-6".to_string(),
            model: "claude-opus-4-6".to_string(),
            upgrade: None,
            display_name: "Claude Opus 4-6".to_string(),
            description: "Anthropic's most capable model for complex reasoning.".to_string(),
            hidden: false,
            supported_reasoning_efforts: vec![
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Low,
                    description: "Fast responses with lighter reasoning".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Medium,
                    description: "Balanced reasoning for everyday tasks".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::High,
                    description: "Greater reasoning depth for complex problems".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Adaptive,
                    description: "Automatically adjusts reasoning depth based on task complexity"
                        .to_string(),
                },
            ],
            default_reasoning_effort: ReasoningEffort::Adaptive,
            input_modalities: vec![InputModality::Text, InputModality::Image],
            supports_personality: false,
            is_default: false,
        },
        // Google Gemini models
        Model {
            id: "gemini-3-pro-preview".to_string(),
            model: "gemini-3-pro-preview".to_string(),
            upgrade: None,
            display_name: "Gemini 3 Pro".to_string(),
            description: "Google's advanced model for complex tasks.".to_string(),
            hidden: false,
            supported_reasoning_efforts: vec![
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Low,
                    description: "Fast responses with lighter reasoning".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::High,
                    description: "Deep reasoning for complex problems".to_string(),
                },
            ],
            default_reasoning_effort: ReasoningEffort::Medium,
            input_modalities: vec![InputModality::Text, InputModality::Image],
            supports_personality: false,
            is_default: false,
        },
        Model {
            id: "gemini-3-flash-preview".to_string(),
            model: "gemini-3-flash-preview".to_string(),
            upgrade: None,
            display_name: "Gemini 3 Flash".to_string(),
            description: "Google's fast and efficient model.".to_string(),
            hidden: false,
            supported_reasoning_efforts: vec![
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Minimal,
                    description: "Fastest responses with minimal reasoning".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Low,
                    description: "Quick responses with light reasoning".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::Medium,
                    description: "Balanced reasoning for everyday tasks".to_string(),
                },
                ReasoningEffortOption {
                    reasoning_effort: ReasoningEffort::High,
                    description: "Deep reasoning for complex problems".to_string(),
                },
            ],
            default_reasoning_effort: ReasoningEffort::Medium,
            input_modalities: vec![InputModality::Text, InputModality::Image],
            supports_personality: false,
            is_default: false,
        },
    ];

    assert_eq!(items, expected_models);
    assert!(next_cursor.is_none());
    Ok(())
}

#[tokio::test]
async fn list_models_includes_hidden_models() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(100),
            cursor: None,
            include_hidden: Some(true),
        })
        .await?;

    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let ModelListResponse {
        data: items,
        next_cursor,
    } = to_response::<ModelListResponse>(response)?;

    assert!(items.iter().any(|item| item.hidden));
    assert!(next_cursor.is_none());
    Ok(())
}

#[tokio::test]
async fn list_models_pagination_works() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let first_request = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(1),
            cursor: None,
            include_hidden: None,
        })
        .await?;

    let first_response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(first_request)),
    )
    .await??;

    let ModelListResponse {
        data: first_items,
        next_cursor: first_cursor,
    } = to_response::<ModelListResponse>(first_response)?;

    assert_eq!(first_items.len(), 1);
    assert_eq!(first_items[0].id, "gpt-5.2-codex");
    let next_cursor = first_cursor.ok_or_else(|| anyhow!("cursor for second page"))?;

    let second_request = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(1),
            cursor: Some(next_cursor.clone()),
            include_hidden: None,
        })
        .await?;

    let second_response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(second_request)),
    )
    .await??;

    let ModelListResponse {
        data: second_items,
        next_cursor: second_cursor,
    } = to_response::<ModelListResponse>(second_response)?;

    assert_eq!(second_items.len(), 1);
    assert_eq!(second_items[0].id, "gpt-5.1-codex-max");
    let third_cursor = second_cursor.ok_or_else(|| anyhow!("cursor for third page"))?;

    let third_request = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(1),
            cursor: Some(third_cursor.clone()),
            include_hidden: None,
        })
        .await?;

    let third_response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(third_request)),
    )
    .await??;

    let ModelListResponse {
        data: third_items,
        next_cursor: third_cursor,
    } = to_response::<ModelListResponse>(third_response)?;

    assert_eq!(third_items.len(), 1);
    assert_eq!(third_items[0].id, "gpt-5.1-codex-mini");
    let fourth_cursor = third_cursor.ok_or_else(|| anyhow!("cursor for fourth page"))?;

    let fourth_request = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(1),
            cursor: Some(fourth_cursor.clone()),
            include_hidden: None,
        })
        .await?;

    let fourth_response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(fourth_request)),
    )
    .await??;

    let ModelListResponse {
        data: fourth_items,
        next_cursor: fourth_cursor,
    } = to_response::<ModelListResponse>(fourth_response)?;

    assert_eq!(fourth_items.len(), 1);
    assert_eq!(fourth_items[0].id, "gpt-5.2");
    let fifth_cursor = fourth_cursor.ok_or_else(|| anyhow!("cursor for fifth page"))?;

    // Fifth page: claude-sonnet-4-5
    let fifth_request = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(1),
            cursor: Some(fifth_cursor.clone()),
            include_hidden: None,
        })
        .await?;

    let fifth_response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(fifth_request)),
    )
    .await??;

    let ModelListResponse {
        data: fifth_items,
        next_cursor: fifth_cursor,
    } = to_response::<ModelListResponse>(fifth_response)?;

    assert_eq!(fifth_items.len(), 1);
    assert_eq!(fifth_items[0].id, "claude-sonnet-4-5");
    let sixth_cursor = fifth_cursor.ok_or_else(|| anyhow!("cursor for sixth page"))?;

    // Sixth page: claude-opus-4-6
    let sixth_request = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(1),
            cursor: Some(sixth_cursor.clone()),
            include_hidden: None,
        })
        .await?;

    let sixth_response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(sixth_request)),
    )
    .await??;

    let ModelListResponse {
        data: sixth_items,
        next_cursor: sixth_cursor,
    } = to_response::<ModelListResponse>(sixth_response)?;

    assert_eq!(sixth_items.len(), 1);
    assert_eq!(sixth_items[0].id, "claude-opus-4-6");
    let seventh_cursor = sixth_cursor.ok_or_else(|| anyhow!("cursor for seventh page"))?;

    // Seventh page: gemini-3-pro-preview
    let seventh_request = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(1),
            cursor: Some(seventh_cursor.clone()),
            include_hidden: None,
        })
        .await?;

    let seventh_response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(seventh_request)),
    )
    .await??;

    let ModelListResponse {
        data: seventh_items,
        next_cursor: seventh_cursor,
    } = to_response::<ModelListResponse>(seventh_response)?;

    assert_eq!(seventh_items.len(), 1);
    assert_eq!(seventh_items[0].id, "gemini-3-pro-preview");
    let eighth_cursor = seventh_cursor.ok_or_else(|| anyhow!("cursor for eighth page"))?;

    // Eighth page: gemini-3-flash-preview (last)
    let eighth_request = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(1),
            cursor: Some(eighth_cursor.clone()),
            include_hidden: None,
        })
        .await?;

    let eighth_response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(eighth_request)),
    )
    .await??;

    let ModelListResponse {
        data: eighth_items,
        next_cursor: eighth_cursor,
    } = to_response::<ModelListResponse>(eighth_response)?;

    assert_eq!(eighth_items.len(), 1);
    assert_eq!(eighth_items[0].id, "gemini-3-flash-preview");
    assert!(eighth_cursor.is_none());
    Ok(())
}

#[tokio::test]
async fn list_models_rejects_invalid_cursor() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: None,
            cursor: Some("invalid".to_string()),
            include_hidden: None,
        })
        .await?;

    let error: JSONRPCError = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(request_id)),
    )
    .await??;

    assert_eq!(error.id, RequestId::Integer(request_id));
    assert_eq!(error.error.code, INVALID_REQUEST_ERROR_CODE);
    assert_eq!(error.error.message, "invalid cursor: invalid");
    Ok(())
}
