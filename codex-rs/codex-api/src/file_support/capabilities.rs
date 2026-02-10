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
            always_inline_max: 2 * 1024 * 1024,
            max_inline_file_size: 20 * 1024 * 1024,
            max_upload_file_size: 50 * 1024 * 1024,
            max_inline_payload_bytes: 20 * 1024 * 1024,
        }
    }
}

pub fn file_capabilities_for(provider_id: &str) -> FileCapabilityConfig {
    match provider_id {
        "openai" => FileCapabilityConfig {
            supports_pdf: true,
            max_inline_file_size: 50 * 1024 * 1024,
            max_upload_file_size: 512 * 1024 * 1024,
            max_inline_payload_bytes: 50 * 1024 * 1024,
            ..FileCapabilityConfig::default()
        },
        "anthropic" => FileCapabilityConfig {
            supports_pdf: true,
            max_inline_file_size: 24 * 1024 * 1024,
            max_upload_file_size: 500 * 1024 * 1024,
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
}
