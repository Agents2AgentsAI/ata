// Wired into session/mod.rs on origin/main via `Feature::CodexGitCommit` and
// `Config.commit_attribution`. Both were dropped by the v0.134 upstream merge;
// allow dead code until the field + call site can be restored in a follow-up.
#![allow(dead_code)]

const DEFAULT_ATTRIBUTION_VALUE: &str = "Codex <noreply@openai.com>";

fn build_commit_message_trailer(config_attribution: Option<&str>) -> Option<String> {
    let value = resolve_attribution_value(config_attribution)?;
    Some(format!("Co-authored-by: {value}"))
}

// @agent-facing
pub(crate) fn commit_message_trailer_instruction(
    config_attribution: Option<&str>,
) -> Option<String> {
    let trailer = build_commit_message_trailer(config_attribution)?;
    Some(format!(
        "When you write or edit a git commit message, ensure the message ends with this trailer exactly once:\n{trailer}\n\nRules:\n- Keep existing trailers and append this trailer at the end if missing.\n- Do not duplicate this trailer if it already exists.\n- Keep one blank line between the commit body and trailer block."
    ))
}

fn resolve_attribution_value(config_attribution: Option<&str>) -> Option<String> {
    match config_attribution {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        None => Some(DEFAULT_ATTRIBUTION_VALUE.to_string()),
    }
}

#[cfg(test)]
#[path = "commit_attribution_tests.rs"]
mod tests;
