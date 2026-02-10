use codex_utils_file::ALWAYS_INLINE_MAX;
use codex_utils_file::MAX_FILE_SIZE;

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
            max_upload_file_size: MAX_FILE_SIZE,
            max_inline_payload_bytes: 20 * 1024 * 1024,
        }
    }
}

pub fn file_capabilities_for(provider_id: &str) -> FileCapabilityConfig {
    match provider_id {
        "openai" => FileCapabilityConfig {
            supports_pdf: true,
            max_inline_file_size: 50 * 1024 * 1024,
            max_upload_file_size: MAX_FILE_SIZE,
            max_inline_payload_bytes: 50 * 1024 * 1024,
            ..FileCapabilityConfig::default()
        },
        "anthropic" => FileCapabilityConfig {
            supports_pdf: true,
            max_inline_file_size: 24 * 1024 * 1024,
            max_upload_file_size: MAX_FILE_SIZE,
            max_inline_payload_bytes: 32 * 1024 * 1024,
            ..FileCapabilityConfig::default()
        },
        "gemini" => FileCapabilityConfig {
            supports_pdf: true,
            ..FileCapabilityConfig::default()
        },
        _ => FileCapabilityConfig::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_providers_support_pdf() {
        assert!(file_capabilities_for("openai").supports_pdf);
        assert!(file_capabilities_for("anthropic").supports_pdf);
        assert!(file_capabilities_for("gemini").supports_pdf);
    }

    #[test]
    fn unknown_provider_defaults_to_no_pdf() {
        assert!(!file_capabilities_for("unknown").supports_pdf);
    }

    #[test]
    fn gemini_has_lowest_inline_threshold() {
        let openai = file_capabilities_for("openai");
        let gemini = file_capabilities_for("gemini");
        assert!(gemini.max_inline_file_size <= openai.max_inline_file_size);
    }

    #[test]
    fn defaults_use_utils_file_constants() {
        let defaults = FileCapabilityConfig::default();
        assert_eq!(defaults.always_inline_max, ALWAYS_INLINE_MAX);
        assert_eq!(defaults.max_upload_file_size, MAX_FILE_SIZE);
    }

    #[test]
    fn upload_limits_are_capped_by_local_processing_limit() {
        for provider_id in ["openai", "anthropic", "gemini"] {
            let caps = file_capabilities_for(provider_id);
            assert!(caps.max_upload_file_size <= MAX_FILE_SIZE);
        }
    }
}
