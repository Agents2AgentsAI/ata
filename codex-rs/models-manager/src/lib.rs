pub(crate) mod cache;
pub mod collaboration_mode_presets;
pub(crate) mod config;
pub mod manager;
pub mod model_info;
pub mod model_presets;
pub mod test_support;

pub use codex_app_server_protocol::AuthMode;
pub use config::ModelsManagerConfig;

/// Load the bundled model catalog shipped with `codex-models-manager`.
pub fn bundled_models_response()
-> std::result::Result<codex_protocol::openai_models::ModelsResponse, serde_json::Error> {
    serde_json::from_str(include_str!("../models.json"))
}

/// Load the bundled GitHub Copilot model catalog. The Copilot provider has no
/// equivalent of OpenAI's `/models` endpoint that ATA can call with the
/// GitHub OAuth bearer, so the `/model` picker is fed from this static list.
/// Sourced from
/// `https://docs.github.com/en/copilot/reference/ai-models/supported-models`.
pub fn bundled_copilot_models_response()
-> std::result::Result<codex_protocol::openai_models::ModelsResponse, serde_json::Error> {
    serde_json::from_str(include_str!("../copilot_models.json"))
}

/// Load the bundled Anthropic Messages API model catalog. Anthropic's
/// `/v1/models` endpoint requires the API key, so we ship the canonical
/// slugs (Claude Opus 4.5 / Sonnet 4.5 / Haiku 4.5 / legacy 3.5 Sonnet)
/// here for the `/model` picker. The Messages API slug format
/// (`claude-<model>-<version>`) differs from Copilot's display slugs.
pub fn bundled_anthropic_models_response()
-> std::result::Result<codex_protocol::openai_models::ModelsResponse, serde_json::Error> {
    serde_json::from_str(include_str!("../anthropic_models.json"))
}

/// Load the bundled Google Gemini (Generative AI API) model catalog.
/// Mirrors the slug shape that `generativelanguage.googleapis.com`
/// expects (`gemini-2.5-pro`, `gemini-2.5-flash`, etc.).
pub fn bundled_gemini_models_response()
-> std::result::Result<codex_protocol::openai_models::ModelsResponse, serde_json::Error> {
    serde_json::from_str(include_str!("../gemini_models.json"))
}

/// Convert the client version string to a whole version string (e.g. "1.2.3-alpha.4" -> "1.2.3").
pub fn client_version_to_whole() -> String {
    format!(
        "{}.{}.{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    )
}
