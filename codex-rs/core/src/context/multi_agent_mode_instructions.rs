use super::ContextualUserFragment;
use codex_protocol::config_types::MultiAgentMode;
use codex_protocol::protocol::MULTI_AGENT_MODE_CLOSE_TAG;
use codex_protocol::protocol::MULTI_AGENT_MODE_OPEN_TAG;

// @agent-facing
pub(crate) const PROACTIVE_MULTI_AGENT_MODE_TEXT: &str = "Proactive multi-agent delegation is active. Any earlier instruction requiring an explicit user request before spawning sub-agents no longer applies. Use sub-agents when parallel work would materially improve speed or quality. This mode remains active until a later multi-agent mode developer message changes it.";

/// Developer fragment that carries the active multi-agent delegation policy.
///
/// Only the proactive policy is materialized here; the fork keeps its
/// `explicit_request_only` default behavior via the multi-agent v2 usage-hint
/// config path, so this fragment is emitted solely when the effective effort
/// flips the session into [`MultiAgentMode::Proactive`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MultiAgentModeInstructions {
    multi_agent_mode: MultiAgentMode,
}

impl MultiAgentModeInstructions {
    pub(crate) fn new(multi_agent_mode: MultiAgentMode) -> Self {
        Self { multi_agent_mode }
    }
}

impl ContextualUserFragment for MultiAgentModeInstructions {
    fn role() -> &'static str {
        "developer"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        (MULTI_AGENT_MODE_OPEN_TAG, MULTI_AGENT_MODE_CLOSE_TAG)
    }

    fn body(&self) -> String {
        match &self.multi_agent_mode {
            MultiAgentMode::Custom(hint_text) => hint_text.clone(),
            MultiAgentMode::ExplicitRequestOnly => String::new(),
            MultiAgentMode::Proactive => PROACTIVE_MULTI_AGENT_MODE_TEXT.to_string(),
        }
    }
}
