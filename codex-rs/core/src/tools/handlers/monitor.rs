//! Built-in tool handlers for in-session Monitor watchers.
//!
//! Three tools (`monitor_start`, `monitor_list`, `monitor_stop`) are
//! registered when [`Feature::Scheduling`] is enabled. They operate on the
//! session-scoped `MonitorRuntime` (registry + per-monitor abort handles).

use serde::Deserialize;
use serde::Serialize;

mod list;
mod start;
mod stop;
mod wait;
mod watch_for;

pub use list::MonitorListHandler;
pub use start::MonitorStartHandler;
pub(crate) use start::run_monitor;
pub use stop::MonitorStopHandler;
pub use wait::MonitorWaitHandler;
pub use watch_for::MonitorWatchForHandler;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct MonitorStartArgs {
    command: String,
    /// Optional Slice 5 toggle. When omitted (default) or `true`, per-line
    /// stdout chat cells are suppressed — the user sees the running
    /// monitor's existence and line count in `/scheduling` but not every
    /// streamed line. Set `false` for live tail-style monitors where you
    /// want the per-line stream.
    #[serde(default = "default_background")]
    background: bool,
    /// When `true`, on session resume this monitor is respawned with the
    /// same command. Default `false`. Only safe for idempotent
    /// long-running commands (`tail -F`, `watch`, dev servers).
    #[serde(default)]
    restart_on_resume: bool,
    /// Optional. Short human-readable label for the `/scheduling` panel.
    #[serde(default)]
    name: Option<String>,
}

fn default_background() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct MonitorStopArgs {
    task_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct MonitorWaitArgs {
    task_id: String,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct MonitorWatchForArgs {
    task_id: String,
    /// Literal substring to look for in each line. Case-sensitive.
    pattern: String,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    /// Default `false`: return on the first match (single-match mode).
    /// When `true`, collect every match for the lifetime of the subprocess
    /// (or until `max_matches` / `timeout_seconds` fires) and return them
    /// all at once. Used for "react to each item in a batch" workflows.
    #[serde(default)]
    all_matches: bool,
    /// Only meaningful when `all_matches=true`. Stop collecting after this
    /// many matches and return early. `None` = collect until the subprocess
    /// terminates or `timeout_seconds` fires.
    #[serde(default)]
    max_matches: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct MonitorWatchForMatch {
    matching_line: String,
    /// `"stdout"` or `"stderr"`.
    stream: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct MonitorWatchForResponse {
    /// True if at least one matching line was seen.
    matched: bool,
    /// The first matching line. Always populated when `matched` is true,
    /// regardless of single- or all-matches mode. Single-match mode also
    /// sets `stream` to the originating stream of this line.
    matching_line: Option<String>,
    /// `"stdout"` or `"stderr"`, when `matched` is true and only one match
    /// was captured (single-match mode, or first match in all-matches mode).
    stream: Option<String>,
    /// Populated only when `all_matches=true`. Every matching line in the
    /// order it appeared, with the stream it came from. Empty list when
    /// nothing matched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    matches: Vec<MonitorWatchForMatch>,
    /// True when the subprocess terminated before (or without further)
    /// matches.
    terminated_without_match: bool,
    /// True when an explicit `timeout_seconds` fired.
    timed_out: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct MonitorStartResponse {
    task_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct MonitorWaitResponse {
    status: String,
    timed_out: bool,
    tail: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct MonitorSummary {
    task_id: String,
    command: String,
    status: String,
    started_at: Option<String>,
    stopped_at: Option<String>,
    lines_emitted: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct MonitorListResponse {
    monitors: Vec<MonitorSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct MonitorStopResponse {
    stopped: bool,
}
