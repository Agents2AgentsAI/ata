use anyhow::Result;
use codex_core::CodexAuth;
use codex_core::ThreadManager;
use codex_core::built_in_model_providers;
use codex_core::models_manager::manager::RefreshStrategy;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;
use codex_protocol::openai_models::default_input_modalities;
use core_test_support::load_default_config_for_test;
use pretty_assertions::assert_eq;
use tempfile::tempdir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_models_returns_api_key_models() -> Result<()> {
    let codex_home = tempdir()?;
    let config = load_default_config_for_test(&codex_home).await;
    let manager = ThreadManager::with_models_provider(
        CodexAuth::from_api_key("sk-test"),
        built_in_model_providers()["openai"].clone(),
    );
    let models = manager
        .list_models(&config, RefreshStrategy::OnlineIfUncached)
        .await;

    let expected_models = expected_models_for_api_key();
    assert_eq!(expected_models, models);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_models_returns_chatgpt_models() -> Result<()> {
    let codex_home = tempdir()?;
    let config = load_default_config_for_test(&codex_home).await;
    let manager = ThreadManager::with_models_provider(
        CodexAuth::create_dummy_chatgpt_auth_for_testing(),
        built_in_model_providers()["openai"].clone(),
    );
    let models = manager
        .list_models(&config, RefreshStrategy::OnlineIfUncached)
        .await;

    let expected_models = expected_models_for_chatgpt();
    assert_eq!(expected_models, models);

    Ok(())
}

fn expected_models_for_api_key() -> Vec<ModelPreset> {
    expected_models(false)
}

fn expected_models_for_chatgpt() -> Vec<ModelPreset> {
    expected_models(true)
}

fn expected_models(chatgpt_mode: bool) -> Vec<ModelPreset> {
    let response: ModelsResponse = serde_json::from_str(include_str!("../../models.json"))
        .expect("bundled models.json should deserialize");
    let mut remote_models = response.models;
    remote_models.sort_by(|a, b| a.priority.cmp(&b.priority));

    let remote_presets: Vec<ModelPreset> = remote_models.into_iter().map(Into::into).collect();
    let existing_presets = extra_local_presets();
    let mut merged_presets = ModelPreset::merge(remote_presets, existing_presets);
    merged_presets = ModelPreset::filter_by_auth(merged_presets, chatgpt_mode);

    for preset in &mut merged_presets {
        preset.is_default = false;
    }
    if let Some(default) = merged_presets
        .iter_mut()
        .find(|preset| preset.show_in_picker)
    {
        default.is_default = true;
    } else if let Some(default) = merged_presets.first_mut() {
        default.is_default = true;
    }

    merged_presets
}

fn extra_local_presets() -> Vec<ModelPreset> {
    vec![
        bengalfox(),
        boomslang(),
        claude_sonnet_4_5(),
        claude_opus_4_5(),
        gemini_3_pro_preview(),
        gemini_3_flash_preview(),
    ]
}

fn effort(effort: ReasoningEffort, description: &str) -> ReasoningEffortPreset {
    ReasoningEffortPreset {
        effort,
        description: description.to_string(),
    }
}

fn bengalfox() -> ModelPreset {
    ModelPreset {
        id: "bengalfox".to_string(),
        model: "bengalfox".to_string(),
        display_name: "bengalfox".to_string(),
        description: "bengalfox".to_string(),
        default_reasoning_effort: ReasoningEffort::Medium,
        supported_reasoning_efforts: vec![
            effort(
                ReasoningEffort::Low,
                "Fast responses with lighter reasoning",
            ),
            effort(
                ReasoningEffort::Medium,
                "Balances speed and reasoning depth for everyday tasks",
            ),
            effort(
                ReasoningEffort::High,
                "Greater reasoning depth for complex problems",
            ),
            effort(
                ReasoningEffort::XHigh,
                "Extra high reasoning depth for complex problems",
            ),
        ],
        supports_personality: true,
        is_default: false,
        upgrade: None,
        show_in_picker: false,
        supported_in_api: true,
        provider_id: None,
        input_modalities: default_input_modalities(),
    }
}

fn boomslang() -> ModelPreset {
    ModelPreset {
        id: "boomslang".to_string(),
        model: "boomslang".to_string(),
        display_name: "boomslang".to_string(),
        description: "boomslang".to_string(),
        default_reasoning_effort: ReasoningEffort::Medium,
        supported_reasoning_efforts: vec![
            effort(
                ReasoningEffort::Low,
                "Balances speed with some reasoning; useful for straightforward queries and short explanations",
            ),
            effort(
                ReasoningEffort::Medium,
                "Provides a solid balance of reasoning depth and latency for general-purpose tasks",
            ),
            effort(
                ReasoningEffort::High,
                "Maximizes reasoning depth for complex or ambiguous problems",
            ),
            effort(
                ReasoningEffort::XHigh,
                "Extra high reasoning depth for complex problems",
            ),
        ],
        supports_personality: false,
        is_default: false,
        upgrade: None,
        show_in_picker: false,
        supported_in_api: true,
        provider_id: None,
        input_modalities: default_input_modalities(),
    }
}

fn claude_sonnet_4_5() -> ModelPreset {
    ModelPreset {
        id: "claude-sonnet-4-5".to_string(),
        model: "claude-sonnet-4-5".to_string(),
        display_name: "Claude Sonnet 4-5".to_string(),
        description: "Anthropic's balanced model for coding tasks.".to_string(),
        default_reasoning_effort: ReasoningEffort::Medium,
        supported_reasoning_efforts: vec![
            effort(
                ReasoningEffort::Low,
                "Fast responses with lighter reasoning",
            ),
            effort(
                ReasoningEffort::Medium,
                "Balanced reasoning for everyday tasks",
            ),
            effort(
                ReasoningEffort::High,
                "Greater reasoning depth for complex problems",
            ),
        ],
        supports_personality: false,
        is_default: false,
        upgrade: None,
        show_in_picker: true,
        supported_in_api: true,
        provider_id: Some("anthropic".to_string()),
        input_modalities: default_input_modalities(),
    }
}

fn claude_opus_4_5() -> ModelPreset {
    ModelPreset {
        id: "claude-opus-4-6".to_string(),
        model: "claude-opus-4-6".to_string(),
        display_name: "Claude Opus 4-6".to_string(),
        description: "Anthropic's most capable model for complex reasoning.".to_string(),
        default_reasoning_effort: ReasoningEffort::Adaptive,
        supported_reasoning_efforts: vec![
            effort(
                ReasoningEffort::Low,
                "Fast responses with lighter reasoning",
            ),
            effort(
                ReasoningEffort::Medium,
                "Balanced reasoning for everyday tasks",
            ),
            effort(
                ReasoningEffort::High,
                "Greater reasoning depth for complex problems",
            ),
            effort(
                ReasoningEffort::Adaptive,
                "Automatically adjusts reasoning depth based on task complexity",
            ),
        ],
        supports_personality: false,
        is_default: false,
        upgrade: None,
        show_in_picker: true,
        supported_in_api: true,
        provider_id: Some("anthropic".to_string()),
        input_modalities: default_input_modalities(),
    }
}

fn gemini_3_pro_preview() -> ModelPreset {
    ModelPreset {
        id: "gemini-3-pro-preview".to_string(),
        model: "gemini-3-pro-preview".to_string(),
        display_name: "Gemini 3 Pro".to_string(),
        description: "Google's advanced model for complex tasks.".to_string(),
        default_reasoning_effort: ReasoningEffort::Medium,
        supported_reasoning_efforts: vec![
            effort(
                ReasoningEffort::Low,
                "Fast responses with lighter reasoning",
            ),
            effort(ReasoningEffort::High, "Deep reasoning for complex problems"),
        ],
        supports_personality: false,
        is_default: false,
        upgrade: None,
        show_in_picker: true,
        supported_in_api: true,
        provider_id: Some("gemini".to_string()),
        input_modalities: default_input_modalities(),
    }
}

fn gemini_3_flash_preview() -> ModelPreset {
    ModelPreset {
        id: "gemini-3-flash-preview".to_string(),
        model: "gemini-3-flash-preview".to_string(),
        display_name: "Gemini 3 Flash".to_string(),
        description: "Google's fast and efficient model.".to_string(),
        default_reasoning_effort: ReasoningEffort::Medium,
        supported_reasoning_efforts: vec![
            effort(
                ReasoningEffort::Minimal,
                "Fastest responses with minimal reasoning",
            ),
            effort(ReasoningEffort::Low, "Quick responses with light reasoning"),
            effort(
                ReasoningEffort::Medium,
                "Balanced reasoning for everyday tasks",
            ),
            effort(ReasoningEffort::High, "Deep reasoning for complex problems"),
        ],
        supports_personality: false,
        is_default: false,
        upgrade: None,
        show_in_picker: true,
        supported_in_api: true,
        provider_id: Some("gemini".to_string()),
        input_modalities: default_input_modalities(),
    }
}
