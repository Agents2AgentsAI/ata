//! In-memory registry of active `LoopTask`s.
//!
//! Same pattern as `MonitorRegistry`: data only. Per-loop tokio abort
//! handles live in `core/scheduling_runtime.rs`. Each loop has its own
//! task (one tokio timer per loop) so killing one doesn't disturb others.

use chrono::DateTime;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::loop_task::LoopTask;
use crate::task::TaskId;
use crate::task::TaskStatus;

#[derive(Debug, Default)]
pub struct LoopRegistry {
    loops: Mutex<HashMap<TaskId, LoopTask>>,
}

impl LoopRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, task: LoopTask) -> TaskId {
        let id = task.id.clone();
        self.loops
            .lock()
            .expect("LoopRegistry mutex poisoned")
            .insert(id.clone(), task);
        id
    }

    pub fn remove(&self, id: &TaskId) -> Option<LoopTask> {
        self.loops
            .lock()
            .expect("LoopRegistry mutex poisoned")
            .remove(id)
    }

    pub fn list(&self) -> Vec<LoopTask> {
        self.loops
            .lock()
            .expect("LoopRegistry mutex poisoned")
            .values()
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.loops
            .lock()
            .expect("LoopRegistry mutex poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.loops
            .lock()
            .expect("LoopRegistry mutex poisoned")
            .is_empty()
    }

    /// Current status of a loop, or `None` if it has been removed.
    pub fn status(&self, id: &TaskId) -> Option<TaskStatus> {
        self.loops
            .lock()
            .expect("LoopRegistry mutex poisoned")
            .get(id)
            .map(|task| task.status)
    }

    /// Replace the registry contents with the supplied tasks. Used on
    /// session resume (Phase 4) to rehydrate from a saved snapshot. Note
    /// that this hydrates *data only*; the caller still needs to spawn a
    /// new tokio task for each non-terminal loop because the previous
    /// tasks died with the prior session.
    pub fn hydrate(&self, tasks: Vec<LoopTask>) {
        let mut map = self.loops.lock().expect("LoopRegistry mutex poisoned");
        map.clear();
        for task in tasks {
            map.insert(task.id.clone(), task);
        }
    }

    /// Record one iteration of a loop. Called by the per-loop tokio task
    /// each time it fires. Also advances `next_wakeup_at` to `fired_at + interval`
    /// so the `/scheduling` panel can show a live countdown.
    pub fn record_iteration(&self, id: &TaskId, fired_at: DateTime<Utc>) {
        let mut loops = self.loops.lock().expect("LoopRegistry mutex poisoned");
        if let Some(task) = loops.get_mut(id) {
            task.last_iter_at = Some(fired_at);
            task.iteration_count = task.iteration_count.saturating_add(1);
            if let Some(interval) = task.interval
                && let Ok(interval_chrono) = chrono::Duration::from_std(interval)
            {
                task.next_wakeup_at = Some(fired_at + interval_chrono);
            }
            // Return to `Pending` between firings so `/scheduling` reads as
            // "waiting for next interval tick" rather than stuck on `Running`
            // forever. Mirrors the cron registry fix. Terminal transitions
            // (Completed / Killed) come from `mark_terminal` via loop_stop.
            task.status = TaskStatus::Pending;
        }
    }

    /// Set the initial `next_wakeup_at` for a loop. Called by the per-loop
    /// tokio task once when it computes the first scheduled fire time.
    pub fn set_next_wakeup(&self, id: &TaskId, wakeup_at: DateTime<Utc>) {
        let mut loops = self.loops.lock().expect("LoopRegistry mutex poisoned");
        if let Some(task) = loops.get_mut(id) {
            task.next_wakeup_at = Some(wakeup_at);
        }
    }

    /// Clear `next_wakeup_at`. Dynamic loops call this immediately after
    /// firing so the per-loop task doesn't refire on the stale timestamp;
    /// the agent must call `loop_wakeup` to schedule the next iteration.
    pub fn clear_next_wakeup(&self, id: &TaskId) {
        let mut loops = self.loops.lock().expect("LoopRegistry mutex poisoned");
        if let Some(task) = loops.get_mut(id) {
            task.next_wakeup_at = None;
        }
    }

    /// Replace the prompt that gets fired on subsequent iterations.
    /// `loop_wakeup` calls this when the agent wants the next firing to
    /// ask a different question than the original loop_start prompt.
    pub fn update_prompt(&self, id: &TaskId, prompt: String) {
        let mut loops = self.loops.lock().expect("LoopRegistry mutex poisoned");
        if let Some(task) = loops.get_mut(id) {
            task.prompt = prompt;
        }
    }

    /// Mark a loop terminal (stopped by user or completed naturally).
    pub fn mark_terminal(&self, id: &TaskId, status: TaskStatus, stopped_at: DateTime<Utc>) {
        let mut loops = self.loops.lock().expect("LoopRegistry mutex poisoned");
        if let Some(task) = loops.get_mut(id) {
            task.status = status;
            task.last_iter_at = Some(stopped_at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::time::Duration;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    #[test]
    fn insert_then_list() {
        let reg = LoopRegistry::new();
        let id = reg.insert(LoopTask::new_fixed("poll".into(), Duration::from_secs(60)));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.list()[0].id, id);
    }

    #[test]
    fn record_iteration_increments_counter() {
        let reg = LoopRegistry::new();
        let id = reg.insert(LoopTask::new_fixed("p".into(), Duration::from_secs(10)));
        reg.record_iteration(&id, at(100));
        reg.record_iteration(&id, at(200));
        let task = reg.list().into_iter().find(|t| t.id == id).unwrap();
        assert_eq!(task.iteration_count, 2);
        assert_eq!(task.last_iter_at, Some(at(200)));
        // Between iterations the loop returns to Pending ("waiting for next
        // interval tick"); a terminal transition only happens via
        // mark_terminal (loop_stop).
        assert_eq!(task.status, TaskStatus::Pending);
    }

    #[test]
    fn mark_terminal_records_killed() {
        let reg = LoopRegistry::new();
        let id = reg.insert(LoopTask::new_fixed("p".into(), Duration::from_secs(10)));
        reg.mark_terminal(&id, TaskStatus::Killed, at(500));
        let task = reg.list().into_iter().find(|t| t.id == id).unwrap();
        assert_eq!(task.status, TaskStatus::Killed);
    }

    #[test]
    fn status_returns_none_when_missing_and_tracks_terminal() {
        let reg = LoopRegistry::new();
        let unknown = crate::task::TaskId::new();
        assert!(reg.status(&unknown).is_none());
        let id = reg.insert(LoopTask::new_fixed("p".into(), Duration::from_secs(10)));
        assert_eq!(reg.status(&id), Some(TaskStatus::Pending));
        reg.mark_terminal(&id, TaskStatus::Completed, at(1));
        assert_eq!(reg.status(&id), Some(TaskStatus::Completed));
    }

    #[test]
    fn remove_drops_entry() {
        let reg = LoopRegistry::new();
        let id = reg.insert(LoopTask::new_fixed("p".into(), Duration::from_secs(10)));
        assert!(reg.remove(&id).is_some());
        assert_eq!(reg.len(), 0);
    }
}
