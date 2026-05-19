//! Session-side runtime glue for the in-session scheduling tools.
//!
//! `codex-scheduling` owns the pure data (`MonitorRegistry`, `MonitorTask`,
//! etc.) and avoids tokio. The runtime here parks per-monitor tokio
//! `AbortHandle`s next to the registry so `monitor_stop` can actually kill
//! the streaming task that owns the spawned process.

use codex_scheduling::LoopRegistry;
use codex_scheduling::MonitorRegistry;
use codex_scheduling::TaskId;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::broadcast;
use tokio::task::AbortHandle;

/// One line streamed by a monitor: `(stream, line)`. `stream` is
/// `"stdout"` or `"stderr"`.
pub(crate) type MonitorLine = (String, String);

/// Buffer size for the per-monitor broadcast channel. Lagging receivers
/// (e.g. a slow `monitor_watch_for` on a chatty subprocess) get
/// `RecvError::Lagged` and re-sync at the latest line. 256 is plenty for
/// realistic loads and tiny in memory.
const MONITOR_BROADCAST_CAPACITY: usize = 256;

#[derive(Debug)]
pub(crate) struct MonitorRuntime {
    pub registry: Arc<MonitorRegistry>,
    handles: Mutex<HashMap<TaskId, AbortHandle>>,
    /// Per-monitor broadcast of `(stream, line)` events. Created on
    /// `monitor_start`, dropped on terminate. `monitor_watch_for` subscribes
    /// to this so it gets woken up the moment a matching line is emitted —
    /// no polling, no eviction from the 40-line tail buffer.
    watchers: Mutex<HashMap<TaskId, broadcast::Sender<MonitorLine>>>,
}

impl MonitorRuntime {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(MonitorRegistry::new()),
            handles: Mutex::new(HashMap::new()),
            watchers: Mutex::new(HashMap::new()),
        }
    }

    pub fn store_handle(&self, id: TaskId, handle: AbortHandle) {
        self.handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, handle);
    }

    /// Abort the per-monitor task and drop its abort handle. Returns true
    /// if a handle was present. The caller is responsible for separately
    /// removing the entry from the registry data.
    pub fn abort(&self, id: &TaskId) -> bool {
        let mut handles = self
            .handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(handle) = handles.remove(id) {
            handle.abort();
            true
        } else {
            false
        }
    }

    /// Create a broadcast channel for this monitor and return the sender so
    /// the streaming task can publish each line. Dropped when the monitor
    /// terminates (via [`drop_watcher_channel`]) — any active receivers
    /// observe `RecvError::Closed`.
    pub fn register_watcher_channel(&self, id: TaskId) -> broadcast::Sender<MonitorLine> {
        let (tx, _rx) = broadcast::channel(MONITOR_BROADCAST_CAPACITY);
        self.watchers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, tx.clone());
        tx
    }

    /// Subscribe a new receiver to a monitor's broadcast. Returns `None` if
    /// the monitor never registered a channel or has already terminated.
    pub fn subscribe(&self, id: &TaskId) -> Option<broadcast::Receiver<MonitorLine>> {
        self.watchers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .map(broadcast::Sender::subscribe)
    }

    /// Drop the broadcast channel for this monitor. Any active receivers
    /// will see `RecvError::Closed` on their next `recv().await`. Called by
    /// the streaming task once the subprocess has terminated.
    pub fn drop_watcher_channel(&self, id: &TaskId) {
        self.watchers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
    }
}

impl Default for MonitorRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub(crate) struct LoopRuntime {
    pub registry: Arc<LoopRegistry>,
    handles: Mutex<HashMap<TaskId, AbortHandle>>,
}

impl LoopRuntime {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(LoopRegistry::new()),
            handles: Mutex::new(HashMap::new()),
        }
    }

    pub fn store_handle(&self, id: TaskId, handle: AbortHandle) {
        self.handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, handle);
    }

    pub fn abort(&self, id: &TaskId) -> bool {
        let mut handles = self
            .handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(handle) = handles.remove(id) {
            handle.abort();
            true
        } else {
            false
        }
    }
}

impl Default for LoopRuntime {
    fn default() -> Self {
        Self::new()
    }
}
