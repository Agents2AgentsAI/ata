use codex_utils_file::ALWAYS_INLINE_MAX;

// Keep this separate from `codex_utils_file::MAX_FILE_SIZE` so changing the local processing cap
// doesn't implicitly change provider upload limits.
const DEFAULT_MAX_UPLOAD_FILE_SIZE: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCapabilityConfig {
    pub supports_pdf: bool,
    pub always_inline_max: u64,
    pub max_inline_file_size: u64,
    pub max_upload_file_size: u64,
    pub max_inline_payload_bytes: u64,
}

impl Default for FileCapabilityConfig {
    fn default() -> Self {
        Self {
            supports_pdf: false,
            always_inline_max: ALWAYS_INLINE_MAX,
            max_inline_file_size: 20 * 1024 * 1024,
            // We intentionally cap provider uploads at 50MB. While some providers accept larger
            // uploads, current cross-provider PDF processing support is limited to ~50 MB, so
            // allowing bigger uploads would lead to confusing failures.
            max_upload_file_size: DEFAULT_MAX_UPLOAD_FILE_SIZE,
            max_inline_payload_bytes: 20 * 1024 * 1024,
        }
    }
}

pub fn file_capabilities_for(provider_id: &str, model: Option<&str>) -> FileCapabilityConfig {
    let mut config = match provider_id {
        "openai" => FileCapabilityConfig {
            supports_pdf: true,
            max_inline_file_size: 50 * 1024 * 1024,
            max_upload_file_size: DEFAULT_MAX_UPLOAD_FILE_SIZE,
            max_inline_payload_bytes: 50 * 1024 * 1024,
            ..FileCapabilityConfig::default()
        },
        "anthropic" => FileCapabilityConfig {
            supports_pdf: true,
            max_inline_file_size: 24 * 1024 * 1024,
            max_upload_file_size: DEFAULT_MAX_UPLOAD_FILE_SIZE,
            max_inline_payload_bytes: 32 * 1024 * 1024,
            ..FileCapabilityConfig::default()
        },
        "gemini" => FileCapabilityConfig {
            supports_pdf: true,
            max_upload_file_size: DEFAULT_MAX_UPLOAD_FILE_SIZE,
            ..FileCapabilityConfig::default()
        },
        _ => FileCapabilityConfig::default(),
    };

    // Model-level overrides: some older models lack PDF support even though their
    // provider generally supports it.
    if let Some(model) = model {
        apply_model_overrides(provider_id, model, &mut config);
    }

    config
}

/// Disable PDF support for known models that don't handle document attachments.
fn apply_model_overrides(provider_id: &str, model: &str, config: &mut FileCapabilityConfig) {
    if provider_id == "anthropic" {
        // Claude 3 Opus and Claude 3 Haiku do not support PDF input.
        let model_lower = model.to_lowercase();
        if model_lower.contains("claude-3-opus") || model_lower.contains("claude-3-haiku") {
            config.supports_pdf = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_providers_support_pdf() {
        assert!(file_capabilities_for("openai", None).supports_pdf);
        assert!(file_capabilities_for("anthropic", None).supports_pdf);
        assert!(file_capabilities_for("gemini", None).supports_pdf);
    }

    #[test]
    fn unknown_provider_defaults_to_no_pdf() {
        assert!(!file_capabilities_for("unknown", None).supports_pdf);
    }

    #[test]
    fn gemini_has_lowest_inline_threshold() {
        let openai = file_capabilities_for("openai", None);
        let gemini = file_capabilities_for("gemini", None);
        assert!(gemini.max_inline_file_size <= openai.max_inline_file_size);
    }

    #[test]
    fn defaults_use_utils_file_constants() {
        let defaults = FileCapabilityConfig::default();
        assert_eq!(defaults.always_inline_max, ALWAYS_INLINE_MAX);
        assert_eq!(defaults.max_upload_file_size, DEFAULT_MAX_UPLOAD_FILE_SIZE);
    }

    #[test]
    fn upload_limits_are_capped_by_local_processing_limit() {
        use codex_utils_file::MAX_FILE_SIZE;
        for provider_id in ["openai", "anthropic", "gemini"] {
            let caps = file_capabilities_for(provider_id, None);
            assert!(caps.max_upload_file_size <= MAX_FILE_SIZE);
        }
    }

    #[test]
    fn claude_3_opus_disables_pdf() {
        let caps = file_capabilities_for("anthropic", Some("claude-3-opus-20240229"));
        assert!(!caps.supports_pdf);
    }

    #[test]
    fn claude_3_haiku_disables_pdf() {
        let caps = file_capabilities_for("anthropic", Some("claude-3-haiku-20240307"));
        assert!(!caps.supports_pdf);
    }

    #[test]
    fn claude_3_5_sonnet_keeps_pdf() {
        let caps = file_capabilities_for("anthropic", Some("claude-3-5-sonnet-20241022"));
        assert!(caps.supports_pdf);
    }
}
