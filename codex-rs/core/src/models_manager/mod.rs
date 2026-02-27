pub mod cache;
pub mod collaboration_mode_presets;
pub mod manager;
pub mod model_info;
pub mod model_presets;

pub const OPENAI_MODELS_CLIENT_VERSION: &str = "0.105.0";

/// Canonical OpenAI `/models` client version used for remote model metadata.
pub fn models_api_client_version() -> &'static str {
    OPENAI_MODELS_CLIENT_VERSION
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
