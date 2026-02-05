use std::sync::{Arc, RwLock};

pub(crate) struct SessionHeader {
    model: Arc<RwLock<String>>,
}

impl SessionHeader {
    pub(crate) fn new(model: String) -> Self {
        Self {
            model: Arc::new(RwLock::new(model)),
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
}
