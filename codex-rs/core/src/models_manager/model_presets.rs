use crate::auth::AuthMode;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;
use codex_protocol::openai_models::default_input_modalities;
use once_cell::sync::Lazy;

/// Legacy notice keys kept for config compatibility with older migration prompts.
pub const HIDE_GPT5_1_MIGRATION_PROMPT_CONFIG: &str = "hide_gpt5_1_migration_prompt";
pub const HIDE_GPT_5_1_CODEX_MAX_MIGRATION_PROMPT_CONFIG: &str =
    "hide_gpt-5.1-codex-max_migration_prompt";

pub(crate) static PRESETS: Lazy<Vec<ModelPreset>> = Lazy::new(|| {
    vec![
        bengalfox(),
        boomslang(),
        claude_sonnet_4_6(),
        claude_opus_4_6(),
        gemini_3_pro_preview(),
        gemini_3_1_pro_preview(),
        gemini_3_flash_preview(),
    ]
});

pub(super) fn builtin_model_presets(_auth_mode: Option<AuthMode>) -> Vec<ModelPreset> {
    PRESETS.iter().cloned().collect()
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

fn claude_sonnet_4_6() -> ModelPreset {
    ModelPreset {
        id: "claude-sonnet-4-6".to_string(),
        model: "claude-sonnet-4-6".to_string(),
        display_name: "Claude Sonnet 4-6".to_string(),
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

fn claude_opus_4_6() -> ModelPreset {
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

fn gemini_3_1_pro_preview() -> ModelPreset {
    ModelPreset {
        id: "gemini-3.1-pro-preview".to_string(),
        model: "gemini-3.1-pro-preview".to_string(),
        display_name: "Gemini 3.1 Pro".to_string(),
        description: "Google's advanced model for complex tasks.".to_string(),
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
