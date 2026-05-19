//! `MonitorTask` — a long-running background command whose stdout lines are
//! surfaced back to the agent as event notifications between turns.

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

use crate::task::TaskId;
use crate::task::TaskStatus;

/// One background streaming watcher. The actual process launch lives in a
/// later phase; this struct captures the contract and bookkeeping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorTask {
    pub id: TaskId,
    /// Shell-style command to spawn (e.g., `tail -F /var/log/app.log`).
    pub command: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub stopped_at: Option<DateTime<Utc>>,
    /// Count of stdout lines surfaced to the agent so far.
    pub lines_emitted: u64,
    /// When `true` (the new default), per-line stdout chat cells are
    /// suppressed in the TUI. The `/scheduling` panel's `lines N` counter
    /// still climbs and the terminate-summary still fires, so the user can
    /// see *that* lines came in without seeing each one.
    #[serde(default = "default_background")]
    pub background: bool,
    /// When `true`, on session resume any monitor that was `Running` (and
    /// therefore got marked `Interrupted` by the hydration path) is
    /// respawned with the same command and task_id. Off by default —
    /// only set this for safely-restartable commands like `tail -F`,
    /// `watch`, or long-lived servers. Never set for one-shot or
    /// destructive commands (`cargo build`, `git push`, batch jobs).
    #[serde(default)]
    pub restart_on_resume: bool,
    /// Optional human-readable label assigned by the agent at creation time
    /// (e.g. `"Disk monitor"`). Surfaced in the `/scheduling` panel.
    #[serde(default)]
    pub name: Option<String>,
}

fn default_background() -> bool {
    true
}

impl MonitorTask {
    pub fn new(command: String) -> Self {
        Self::new_with_options(command, true, false)
    }

    pub fn new_with_background(command: String, background: bool) -> Self {
        Self::new_with_options(command, background, false)
    }

    pub fn new_with_options(command: String, background: bool, restart_on_resume: bool) -> Self {
        Self {
            id: TaskId::new(),
            command,
            status: TaskStatus::Pending,
            created_at: Utc::now(),
            started_at: None,
            stopped_at: None,
            lines_emitted: 0,
            background,
            restart_on_resume,
            name: None,
        }
    }

    /// Builder-style helper: set the agent-supplied human-readable name.
    pub fn with_name(mut self, name: Option<String>) -> Self {
        self.name = crate::cron_job::normalize_optional_name(name);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_monitor_starts_pending_with_no_lines() {
        let m = MonitorTask::new("tail -F log.txt".into());
        assert_eq!(m.status, TaskStatus::Pending);
        assert_eq!(m.lines_emitted, 0);
        assert!(m.started_at.is_none());
        assert!(m.stopped_at.is_none());
    }

    #[test]
    fn each_monitor_gets_unique_id() {
        let a = MonitorTask::new("ls".into());
        let b = MonitorTask::new("ls".into());
        assert_ne!(a.id, b.id);
    }
}
