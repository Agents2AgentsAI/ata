//! ATA scheduling protocol types.
//!
//! `SchedulingTasksSnapshotNotification` wraps
//! `codex_protocol::protocol::SchedulingTasksSnapshotEvent` so the TUI can
//! render the `/scheduling` panel from data fetched via the
//! `scheduling/tasks/list` request below.

use codex_protocol::protocol::SchedulingMonitorOutputDeltaEvent;
use codex_protocol::protocol::SchedulingTasksSnapshotEvent;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct SchedulingTasksSnapshotNotification {
    pub thread_id: String,
    pub event: SchedulingTasksSnapshotEvent,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct SchedulingTasksListParams {
    pub thread_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct SchedulingTasksListResponse {}

/// ATA: delete one scheduling task (cron job, monitor, or loop) from a
/// thread's registry. Used by the `/scheduling` TUI panel's `d` shortcut.
/// Server aborts the task first if it's still running. Response is empty;
/// the panel observes the removal via the next `SchedulingTasksSnapshot`
/// notification, which the server emits immediately after the delete.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct SchedulingTaskDeleteParams {
    pub thread_id: String,
    pub task_id: String,
    pub kind: codex_protocol::protocol::SchedulingTaskKind,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct SchedulingTaskDeleteResponse {}

/// Streamed line of output from a running monitor. The TUI renders this for
/// the user only; the LLM never sees it. See
/// `codex_protocol::protocol::SchedulingMonitorOutputDeltaEvent`.
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct SchedulingMonitorOutputDeltaNotification {
    pub thread_id: String,
    pub event: SchedulingMonitorOutputDeltaEvent,
}
