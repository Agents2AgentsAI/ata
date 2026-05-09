use std::any::Any;
use std::path::Path;

use async_trait::async_trait;
use codex_protocol::models::ResponseItem;

/// Trait that abstracts the coordination lifecycle.
///
/// All methods have default no-op implementations so that
/// [`NoopPlusProvider`] (and tests) can use the trait without
/// implementing every method.
#[async_trait]
pub trait PlusProvider: Send + Sync {
    /// Developer instructions text to inject into the initial context.
    fn developer_instructions_text(&self) -> Option<String> {
        None
    }

    /// Build coordination context if peer state changed.
    /// The `tracker` is an opaque state bag (`Box<dyn Any>`) owned by the caller.
    async fn build_if_changed(
        &self,
        _tracker: &mut Box<dyn Any + Send>,
        _session_id: &str,
        _cwd: &Path,
    ) -> Option<ResponseItem> {
        None
    }

    /// Update session description based on user input.
    async fn update_description(
        &self,
        _session_id: &str,
        _input: &[codex_protocol::user_input::UserInput],
    ) {
    }

    /// Shutdown coordination (stop heartbeat, deregister session, etc.).
    async fn shutdown(&self, _session_id: &str) {}

    /// Create a new tracker for build_if_changed state tracking.
    fn new_tracker(&self) -> Box<dyn Any + Send> {
        Box::new(())
    }
}

/// No-op coordination provider used when coordination is disabled.
pub struct NoopPlusProvider;

#[async_trait]
impl PlusProvider for NoopPlusProvider {}
