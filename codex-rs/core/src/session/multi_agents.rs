use crate::session::turn_context::TurnContext;
use codex_features::Feature;
use codex_protocol::config_types::MultiAgentMode;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;

pub(super) fn usage_hint_text<'a>(
    turn_context: &'a TurnContext,
    session_source: &SessionSource,
) -> Option<&'a str> {
    if !turn_context.features.enabled(Feature::MultiAgentV2) {
        return None;
    }

    let multi_agent_v2 = &turn_context.config.multi_agent_v2;
    match session_source {
        SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. }) => {
            multi_agent_v2.subagent_usage_hint_text.as_deref()
        }
        SessionSource::Cli
        | SessionSource::VSCode
        | SessionSource::Exec
        | SessionSource::Mcp
        | SessionSource::Custom(_)
        | SessionSource::Unknown => multi_agent_v2.root_agent_usage_hint_text.as_deref(),
        SessionSource::Internal(_) | SessionSource::SubAgent(_) => None,
    }
}

/// Resolves the effective multi-agent delegation policy for a turn.
///
/// Mirrors upstream's effort-derived policy: an `ultra` reasoning effort flips
/// the session into [`MultiAgentMode::Proactive`]; every other effort keeps the
/// default `explicit_request_only` policy. Returns `None` when multi-agent v2 is
/// disabled or the session source never spawns sub-agents.
pub(crate) fn effective_multi_agent_mode(
    turn_context: &TurnContext,
    session_source: &SessionSource,
) -> Option<MultiAgentMode> {
    if !turn_context.features.enabled(Feature::MultiAgentV2) {
        return None;
    }

    let multi_agent_mode = match turn_context.effective_reasoning_effort() {
        Some(ReasoningEffort::Ultra) => MultiAgentMode::Proactive,
        _ => MultiAgentMode::ExplicitRequestOnly,
    };

    match session_source {
        SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. })
        | SessionSource::Cli
        | SessionSource::VSCode
        | SessionSource::Exec
        | SessionSource::Mcp
        | SessionSource::Custom(_)
        | SessionSource::Unknown => Some(multi_agent_mode),
        SessionSource::Internal(_) | SessionSource::SubAgent(_) => None,
    }
}
