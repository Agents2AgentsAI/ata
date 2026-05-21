//! In-memory registry of active `MonitorTask`s.
//!
//! Mirrors the design of `CronRegistry` but for streaming background
//! commands: tool handlers (`monitor_start` / `monitor_list` / `monitor_stop`)
//! mutate it, and the per-monitor tokio task in core/ updates its
//! `lines_emitted` counter as output arrives.
//!
//! Phase 2b: data only — no tokio types live in this crate so it stays
//! decoupled from any runtime. Per-monitor abort handles are tracked by
//! core/ in a parallel map keyed by `TaskId`.

use chrono::DateTime;
use chrono::Utc;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Mutex;

use crate::monitor::MonitorTask;
use crate::task::TaskId;
use crate::task::TaskStatus;

/// Number of most-recent output lines retained per monitor so callers (e.g.
/// `monitor_wait`) can return a summary tail. Bounded to keep memory
/// predictable regardless of how chatty the underlying command is.
pub const MONITOR_TAIL_CAPACITY: usize = 40;

#[derive(Debug, Default)]
pub struct MonitorRegistry {
    monitors: Mutex<HashMap<TaskId, MonitorTask>>,
    /// Ring buffer of the most recent ~MONITOR_TAIL_CAPACITY output lines
    /// per monitor. Used to power `monitor_wait` and terminate summaries.
    tails: Mutex<HashMap<TaskId, VecDeque<String>>>,
}

impl MonitorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fully-constructed `MonitorTask`. The caller is responsible
    /// for marking it `Running` and setting `started_at` after the
    /// underlying process is actually spawned.
    pub fn insert(&self, task: MonitorTask) -> TaskId {
        let id = task.id.clone();
        self.monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.clone(), task);
        id
    }

    /// Remove a monitor by id. Returns the removed task if it existed.
    /// Callers should also abort the per-monitor tokio task. The tail ring
    /// buffer is also cleared.
    pub fn remove(&self, id: &TaskId) -> Option<MonitorTask> {
        self.tails
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
        self.monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id)
    }

    /// Snapshot of every registered monitor.
    pub fn list(&self) -> Vec<MonitorTask> {
        self.monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    /// Replace the registry contents with the supplied tasks. Used on
    /// session resume (Phase 4) to rehydrate from a saved snapshot. The
    /// underlying subprocess is gone, so the caller is expected to mark
    /// any non-terminal entries as `Interrupted` before storing them.
    pub fn hydrate(&self, tasks: Vec<MonitorTask>) {
        let mut map = self
            .monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.clear();
        for task in tasks {
            map.insert(task.id.clone(), task);
        }
        // Tail buffers don't survive a process restart; start fresh.
        let mut tails = self
            .tails
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tails.clear();
    }

    /// Update bookkeeping when the per-monitor task observes output.
    pub fn record_line(&self, id: &TaskId) {
        let mut monitors = self
            .monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(task) = monitors.get_mut(id) {
            task.lines_emitted = task.lines_emitted.saturating_add(1);
        }
    }

    /// Append a line to the per-monitor tail ring buffer. Older lines beyond
    /// `MONITOR_TAIL_CAPACITY` are discarded.
    pub fn record_tail_line(&self, id: &TaskId, line: String) {
        let mut tails = self
            .tails
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let buf = tails.entry(id.clone()).or_default();
        if buf.len() == MONITOR_TAIL_CAPACITY {
            buf.pop_front();
        }
        buf.push_back(line);
    }

    /// Snapshot the per-monitor tail. Returns an empty vec if no lines have
    /// been recorded.
    pub fn tail_snapshot(&self, id: &TaskId) -> Vec<String> {
        self.tails
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .map(|buf| buf.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Mark a monitor as running (after process spawn succeeded).
    pub fn mark_running(&self, id: &TaskId, started_at: DateTime<Utc>) {
        let mut monitors = self
            .monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(task) = monitors.get_mut(id) {
            task.status = TaskStatus::Running;
            task.started_at = Some(started_at);
        }
    }

    /// Mark a monitor terminal: process exited or kill requested. Status
    /// reflects which case. The entry stays in the registry until an
    /// explicit `remove`, so `list` continues to surface it.
    pub fn mark_terminal(&self, id: &TaskId, status: TaskStatus, stopped_at: DateTime<Utc>) {
        let mut monitors = self
            .monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(task) = monitors.get_mut(id) {
            task.status = status;
            task.stopped_at = Some(stopped_at);
        }
    }

    /// Reset a previously-terminal entry back to `Pending` with cleared
    /// timestamps and a zeroed line count, in preparation for a respawn.
    /// Used by the session-resume path for monitors that opted into
    /// `restart_on_resume`.
    pub fn reset_for_restart(&self, id: &TaskId) {
        let mut monitors = self
            .monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(task) = monitors.get_mut(id) {
            task.status = TaskStatus::Pending;
            task.started_at = None;
            task.stopped_at = None;
            task.lines_emitted = 0;
        }
        // Tail buffer also belongs to the prior, dead subprocess — drop it.
        let mut tails = self
            .tails
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tails.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    #[test]
    fn insert_then_list() {
        let reg = MonitorRegistry::new();
        let id = reg.insert(MonitorTask::new("tail -f log".into()));
        let listed = reg.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].status, TaskStatus::Pending);
    }

    #[test]
    fn record_line_increments_counter() {
        let reg = MonitorRegistry::new();
        let id = reg.insert(MonitorTask::new("noop".into()));
        reg.record_line(&id);
        reg.record_line(&id);
        let task = reg.list().into_iter().find(|t| t.id == id).unwrap();
        assert_eq!(task.lines_emitted, 2);
    }

    #[test]
    fn mark_running_updates_status_and_started_at() {
        let reg = MonitorRegistry::new();
        let id = reg.insert(MonitorTask::new("noop".into()));
        reg.mark_running(&id, at(100));
        let task = reg.list().into_iter().find(|t| t.id == id).unwrap();
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.started_at, Some(at(100)));
    }

    #[test]
    fn mark_terminal_records_killed_status() {
        let reg = MonitorRegistry::new();
        let id = reg.insert(MonitorTask::new("noop".into()));
        reg.mark_terminal(&id, TaskStatus::Killed, at(200));
        let task = reg.list().into_iter().find(|t| t.id == id).unwrap();
        assert_eq!(task.status, TaskStatus::Killed);
        assert_eq!(task.stopped_at, Some(at(200)));
    }

    #[test]
    fn remove_drops_the_entry() {
        let reg = MonitorRegistry::new();
        let id = reg.insert(MonitorTask::new("noop".into()));
        assert_eq!(reg.len(), 1);
        let removed = reg.remove(&id);
        assert!(removed.is_some());
        assert_eq!(reg.len(), 0);
    }
}
