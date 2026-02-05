use std::sync::{Arc, RwLock};

use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;

pub(crate) struct SessionHeader {
    model: Arc<RwLock<String>>,
    reasoning_effort: Arc<RwLock<Option<ReasoningEffortConfig>>>,
}

impl SessionHeader {
    pub(crate) fn new(model: String) -> Self {
        Self {
            model: Arc::new(RwLock::new(model)),
            reasoning_effort: Arc::new(RwLock::new(None)),
        }
    }

    /// Updates the header's model text.
    pub(crate) fn set_model(&self, model: &str) {
        let mut guard = self.model.write().unwrap();
        if *guard != model {
            *guard = model.to_string();
        }
    }

    /// Get a clone of the shared model Arc for passing to SessionHeaderHistoryCell.
    pub(crate) fn shared_model(&self) -> Arc<RwLock<String>> {
        Arc::clone(&self.model)
    }

    /// Updates the header's reasoning effort.
    pub(crate) fn set_reasoning_effort(&self, effort: Option<ReasoningEffortConfig>) {
        let mut guard = self.reasoning_effort.write().unwrap();
        if *guard != effort {
            *guard = effort;
        }
    }

    /// Get a clone of the shared reasoning_effort Arc for passing to SessionHeaderHistoryCell.
    pub(crate) fn shared_reasoning_effort(&self) -> Arc<RwLock<Option<ReasoningEffortConfig>>> {
        Arc::clone(&self.reasoning_effort)
    }
}
