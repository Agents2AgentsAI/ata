//! Shared identifiers and status enums used across CronJob and MonitorTask.

use serde::Deserialize;
use serde::Serialize;
use std::fmt;
use uuid::Uuid;

/// Unique identifier for a scheduling task. Opaque newtype so the
/// implementation (currently a v4 UUID string) can change without breaking
/// callers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(String);

impl TaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for TaskId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Discriminator for the three task families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskKind {
    Cron,
    Monitor,
    Loop,
}

/// Lifecycle status for any scheduling task.
///
/// `Interrupted` is reserved for the resume path: tasks that were `Running`
/// when ata exited get marked `Interrupted` on session reload so the user
/// can decide whether to restart them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Killed,
    Interrupted,
}

impl TaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed
                | TaskStatus::Failed
                | TaskStatus::Killed
                | TaskStatus::Interrupted
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_is_unique_across_constructions() {
        let a = TaskId::new();
        let b = TaskId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn task_id_roundtrips_through_json() {
        let id = TaskId::new();
        let serialized = serde_json::to_string(&id).unwrap();
        let parsed: TaskId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn task_kind_serializes_as_expected() {
        let json = serde_json::to_string(&TaskKind::Cron).unwrap();
        assert_eq!(json, "\"Cron\"");
    }

    #[test]
    fn task_status_serializes_as_expected() {
        let json = serde_json::to_string(&TaskStatus::Interrupted).unwrap();
        assert_eq!(json, "\"Interrupted\"");
    }
}
