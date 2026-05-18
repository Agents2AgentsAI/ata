//! Built-in tool handlers for in-session Loop tasks.
//!
//! Three tools (`loop_start`, `loop_list`, `loop_stop`) registered when
//! [`Feature::Scheduling`] is enabled. Module is `loop_tool` rather than
//! `loop` because `loop` is a Rust keyword.

use serde::Deserialize;
use serde::Serialize;

mod list;
mod start;
mod stop;
mod wakeup;

pub use list::LoopListHandler;
pub use start::LoopStartHandler;
pub use stop::LoopStopHandler;
pub use wakeup::LoopWakeupHandler;
// Phase 4: respawning loops on session resume calls back into the same
// runner used by `loop_start`.
pub(crate) use start::run_loop;
pub(crate) use start::run_loop_dynamic;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct LoopStartArgs {
    prompt: String,
    /// Fixed interval mode: fires every N seconds for the life of the loop.
    /// Mutually exclusive with `initial_delay_seconds`.
    #[serde(default)]
    interval_seconds: Option<u64>,
    /// Dynamic-pacing mode: fires the FIRST iteration after this many
    /// seconds. Each subsequent firing's delay is set by the agent via
    /// `loop_wakeup`. Mutually exclusive with `interval_seconds`.
    #[serde(default)]
    initial_delay_seconds: Option<u64>,
    /// Optional Slice 5 toggle. When omitted (default) or `true`, the loop
    /// fires silently — the agent's natural-language reply is hidden so
    /// polling loops don't flood the chat. Tool outputs (bash echoes, etc.)
    /// still render so the prompt's intentional alerts come through. Set
    /// `false` for chatty diagnostic loops where you want every reply.
    #[serde(default = "default_background")]
    background: bool,
}

fn default_background() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct LoopStopArgs {
    task_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct LoopWakeupArgs {
    task_id: String,
    /// Seconds from now until the next firing.
    delay_seconds: u64,
    /// Optional. If provided, replaces the loop's prompt for subsequent
    /// firings. Use this when each iteration should ask a different
    /// question (e.g. "now check status X instead of Y").
    #[serde(default)]
    prompt: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct LoopWakeupResponse {
    /// Wall-clock time (RFC3339) when the next firing will happen.
    next_fire_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct LoopStartResponse {
    task_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct LoopSummary {
    task_id: String,
    prompt: String,
    interval_seconds: Option<u64>,
    status: String,
    last_iter_at: Option<String>,
    iteration_count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct LoopListResponse {
    loops: Vec<LoopSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct LoopStopResponse {
    stopped: bool,
}
