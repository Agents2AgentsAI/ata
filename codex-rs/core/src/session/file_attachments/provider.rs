use super::*;

pub(crate) fn file_capabilities_for_provider(
    provider: &ModelProviderInfo,
    model: Option<&str>,
) -> (String, FileCapabilityConfig) {
    let provider_id = match provider.wire_api {
        WireApi::Responses => "openai",
        WireApi::AnthropicMessages => "anthropic",
        WireApi::GeminiGenerate => "gemini",
    };
    (
        provider_id.to_string(),
        file_capabilities_for(provider_id, model),
    )
}

pub(super) fn max_raw_inline_bytes(max_inline_payload_bytes: u64) -> u64 {
    max_inline_payload_bytes.saturating_mul(3).saturating_div(4)
}

pub(super) fn upload_base_url_for_provider(
    provider_id: &str,
    provider: &ModelProviderInfo,
) -> String {
    let fallback = match provider_id {
        "openai" => "https://api.openai.com",
        "anthropic" => "https://api.anthropic.com",
        "gemini" => "https://generativelanguage.googleapis.com",
        _ => "",
    };
    let base = provider
        .base_url
        .as_deref()
        .unwrap_or(fallback)
        .trim_end_matches('/');

    match provider_id {
        "openai" | "anthropic" => base.strip_suffix("/v1").unwrap_or(base).to_string(),
        "gemini" => base.strip_suffix("/v1beta").unwrap_or(base).to_string(),
        _ => base.to_string(),
    }
}

pub(super) fn upload_service_for_provider(provider_id: &str) -> Option<Box<dyn FileUploadService>> {
    match provider_id {
        "openai" => Some(Box::new(OpenAiFileUpload)),
        "anthropic" => Some(Box::new(AnthropicFileUpload)),
        "gemini" => Some(Box::new(GeminiFileUpload)),
        _ => None,
    }
}
