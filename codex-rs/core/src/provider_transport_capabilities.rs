use crate::model_provider_info::ModelProviderInfo;
use crate::model_provider_info::WireApi;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProviderTransportCapabilities {
    pub(crate) supports_file_url_ingestion: bool,
}

pub(crate) fn provider_transport_capabilities(
    provider: &ModelProviderInfo,
) -> ProviderTransportCapabilities {
    let supports_file_url_ingestion = match provider.wire_api {
        // Restrict to the built-in OpenAI provider for now. Some OpenAI-compatible
        // surfaces do not accept `input_file.file_url`.
        WireApi::Responses => provider.is_openai(),
        WireApi::AnthropicMessages => true,
        WireApi::GeminiGenerate => false,
    };

    ProviderTransportCapabilities {
        supports_file_url_ingestion,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn openai_supports_url_file_ingestion() {
        let provider = ModelProviderInfo::create_openai_provider();
        let caps = provider_transport_capabilities(&provider);
        assert_eq!(caps.supports_file_url_ingestion, true);
    }

    #[test]
    fn anthropic_supports_url_file_ingestion() {
        let provider = ModelProviderInfo::create_anthropic_provider();
        let caps = provider_transport_capabilities(&provider);
        assert_eq!(caps.supports_file_url_ingestion, true);
    }

    #[test]
    fn gemini_does_not_support_url_file_ingestion() {
        let provider = ModelProviderInfo::create_gemini_provider();
        let caps = provider_transport_capabilities(&provider);
        assert_eq!(caps.supports_file_url_ingestion, false);
    }
}
