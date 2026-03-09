use crate::auth::OPENAI_API_KEY_ENV_VAR;

use super::types::ANTHROPIC_API_KEY_ENV_VAR;
use super::types::GOOGLE_API_KEY_ENV_VAR;
use super::types::PROVIDER_ANTHROPIC;
use super::types::PROVIDER_GEMINI;
use super::types::PROVIDER_OPENAI;

pub fn provider_env_var(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        PROVIDER_OPENAI => Some(OPENAI_API_KEY_ENV_VAR),
        PROVIDER_ANTHROPIC => Some(ANTHROPIC_API_KEY_ENV_VAR),
        PROVIDER_GEMINI => Some(GOOGLE_API_KEY_ENV_VAR),
        _ => None,
    }
}

pub fn read_api_key_from_env(provider_id: &str) -> Option<String> {
    provider_env_var(provider_id).and_then(|env_var| {
        std::env::var(env_var)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}
