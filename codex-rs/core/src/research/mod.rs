pub mod output_schema;
pub mod prompt;
pub mod researcher_prompt;
pub mod tool_names;
pub mod types;

pub use output_schema::research_output_schema;
pub use prompt::ResearchPromptParams;
pub use prompt::build_research_prompt;
pub use researcher_prompt::RESEARCHER_SYSTEM_PROMPT;
pub use types::ResearchOutput;

pub(crate) type SharedResearchToolkit = codex_research_tools::ResearchToolkit;

use crate::config::types::ResearchToolsToml;
use codex_research_tools::config::ResearchConfig;
use std::path::Path;

/// Build a `ResearchConfig` by layering env vars (via
/// `ResearchConfig::from_env`), then any `[research]` settings from
/// config.toml on top. `codex_home` and `cwd` are accepted for parity with
/// the locus reference signature so a future secret-resolver can plug in
/// without changing call sites.
pub fn build_research_config(
    toml: Option<&ResearchToolsToml>,
    _codex_home: &Path,
    _cwd: &Path,
) -> ResearchConfig {
    let mut config = ResearchConfig::from_env();
    if let Some(toml) = toml {
        apply_toml_research_overrides(&mut config, toml);
    }
    config
}

fn apply_toml_research_overrides(config: &mut ResearchConfig, toml: &ResearchToolsToml) {
    fn normalized(value: Option<&String>) -> Option<Option<String>> {
        value.map(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    }

    fn apply_option(target: &mut Option<String>, override_value: Option<&String>) {
        if let Some(value) = normalized(override_value) {
            *target = value;
        }
    }

    fn apply_string(target: &mut String, override_value: Option<&String>) {
        if let Some(Some(value)) = normalized(override_value) {
            *target = value;
        }
    }

    apply_option(&mut config.zotero_api_key, toml.zotero_api_key.as_ref());
    apply_option(&mut config.zotero_user_id, toml.zotero_user_id.as_ref());
    apply_option(&mut config.openalex_email, toml.openalex_email.as_ref());
    apply_option(
        &mut config.zotero_library_type,
        toml.zotero_library_type.as_ref(),
    );
    apply_option(&mut config.zotero_group_id, toml.zotero_group_id.as_ref());
    apply_option(
        &mut config.zotero_storage_dir,
        toml.zotero_storage_dir.as_ref(),
    );
    apply_string(&mut config.zotero_base_url, toml.zotero_base_url.as_ref());
}
