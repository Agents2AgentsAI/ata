//! Coordination message watcher that receives push notifications from a relay
//! server via SSE and broadcasts them to local subscribers.

use std::sync::Arc;

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::CoordinationMessage;
use crate::relay_client::RelayClient;

/// Broadcast capacity — small since consumers read quickly.
const BROADCAST_CAPACITY: usize = 64;

/// Watches a relay server for new coordination messages and broadcasts them
/// to local subscribers.
pub struct CoordinationWatcher {
    tx: broadcast::Sender<CoordinationMessage>,
}

impl CoordinationWatcher {
    /// Start watching for new coordination messages via relay SSE.
    ///
    /// * `relay` — the relay client to subscribe to
    /// * `own_session_id` — messages from this session are filtered out
    /// * `cancel` — token to stop the watcher
    pub fn start(
        relay: Arc<RelayClient>,
        own_session_id: String,
        cancel: CancellationToken,
    ) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);

        // Subscribe to relay SSE events.
        let mut relay_rx = relay.subscribe_events(cancel.clone());

        tracing::info!(
            "coordination watcher: filtering own session_id={}",
            own_session_id
        );

        let tx_clone = tx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    result = relay_rx.recv() => {
                        match result {
                            Ok(msg) => {
                                tracing::debug!(
                                    "coordination watcher: received msg from session={} (own={})",
                                    msg.session_id,
                                    own_session_id,
                                );
                                if msg.session_id == own_session_id {
                                    tracing::debug!("coordination watcher: filtering own message");
                                    continue;
                                }
                                let _ = tx_clone.send(msg);
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!("coordination watcher lagged by {n} messages");
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        });

        Self { tx }
    }

    /// Subscribe to receive new peer coordination messages.
    pub fn subscribe(&self) -> broadcast::Receiver<CoordinationMessage> {
        self.tx.subscribe()
    }
}
