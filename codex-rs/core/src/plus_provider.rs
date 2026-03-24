use std::any::Any;
use std::path::Path;

use async_trait::async_trait;
use codex_protocol::models::ResponseItem;

#[cfg(feature = "ata-plus")]
pub use crate::plus_context::PlusTracker;

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

    /// Borrow the underlying coordination DB.
    #[cfg(feature = "ata-plus")]
    fn db(&self) -> Option<&std::sync::Arc<codex_coordination::CoordinationDb>> {
        None
    }

    /// Borrow the relay client.
    #[cfg(feature = "ata-plus")]
    fn relay(&self) -> Option<&std::sync::Arc<codex_coordination::relay_client::RelayClient>> {
        None
    }

    /// Create a new tracker for build_if_changed state tracking.
    fn new_tracker(&self) -> Box<dyn Any + Send> {
        Box::new(())
    }
}

/// No-op coordination provider used when coordination is disabled.
pub struct NoopPlusProvider;

#[async_trait]
impl PlusProvider for NoopPlusProvider {}

// Implement the trait for the existing Handle so it can be used as a
// `Box<dyn PlusProvider>` without changing its internals.
#[cfg(feature = "ata-plus")]
#[async_trait]
impl PlusProvider for crate::plus_context::Handle {
    fn developer_instructions_text(&self) -> Option<String> {
        self.developer_instructions_text()
    }

    async fn build_if_changed(
        &self,
        tracker: &mut Box<dyn Any + Send>,
        session_id: &str,
        cwd: &Path,
    ) -> Option<ResponseItem> {
        let t = tracker.downcast_mut::<PlusTracker>()?;
        self.build_if_changed(t, session_id, cwd).await
    }

    async fn update_description(
        &self,
        session_id: &str,
        input: &[codex_protocol::user_input::UserInput],
    ) {
        self.update_description(session_id, input).await;
    }

    async fn shutdown(&self, session_id: &str) {
        self.shutdown(session_id).await;
    }

    fn db(&self) -> Option<&std::sync::Arc<codex_coordination::CoordinationDb>> {
        self.db()
    }

    fn relay(&self) -> Option<&std::sync::Arc<codex_coordination::relay_client::RelayClient>> {
        self.relay()
    }

    fn new_tracker(&self) -> Box<dyn Any + Send> {
        Box::new(PlusTracker::new())
    }
}
