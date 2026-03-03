pub mod cron_trigger;
pub mod file_watch;
pub mod http_poll;
pub mod webhook;

use tokio::sync::mpsc;

/// A trigger event that tells the scheduler to fire a specific job.
#[derive(Debug, Clone)]
pub struct TriggerEvent {
    pub job_id: String,
    /// Optional JSON-encoded data from the trigger source.
    pub data: Option<String>,
}

/// Channel sender for trigger events. Each trigger source holds one and sends
/// events when conditions are met.
pub type TriggerSender = mpsc::UnboundedSender<TriggerEvent>;

/// Channel receiver consumed by the scheduler loop.
pub type TriggerReceiver = mpsc::UnboundedReceiver<TriggerEvent>;

/// Create a trigger channel pair.
pub fn trigger_channel() -> (TriggerSender, TriggerReceiver) {
    mpsc::unbounded_channel()
}
