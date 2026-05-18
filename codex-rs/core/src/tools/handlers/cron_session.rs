//! Built-in tool handlers for in-session cron scheduling.
//!
//! These tools (`cron_create_session`, `cron_list_session`,
//! `cron_delete_session`) operate on the session-scoped `CronRegistry` owned
//! by `Session`. They are the in-process counterpart to the OS-level cron
//! tools in `cron.rs` — same clock-aligned semantics, but the schedule lives
//! in ata's memory and dies when the session closes. Persistence is via the
//! scheduling sidecar (so jobs survive `ata resume`).

use serde::Deserialize;
use serde::Serialize;

mod create;
mod delete;
mod list;

pub use create::CronSessionCreateHandler;
pub use delete::CronSessionDeleteHandler;
pub use list::CronSessionListHandler;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CronSessionCreateArgs {
    cron_expr: String,
    prompt: String,
    #[serde(default = "default_background")]
    background: bool,
    #[serde(default)]
    max_firings: Option<u64>,
    #[serde(default)]
    until: Option<String>,
    #[serde(default)]
    timezone: Option<String>,
}

fn default_background() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CronSessionDeleteArgs {
    task_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CronSessionCreateResponse {
    task_id: String,
    next_fire_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CronSessionJobSummary {
    task_id: String,
    cron_expr: String,
    prompt: String,
    status: String,
    next_fire_at: Option<String>,
    last_fired_at: Option<String>,
    fire_count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CronSessionListResponse {
    jobs: Vec<CronSessionJobSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CronSessionDeleteResponse {
    deleted: bool,
}
