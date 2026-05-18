//! Built-in tool handlers for OS-level cron scheduling.
//!
//! These three tools (`cron_create`, `cron_list`, `cron_delete`) are
//! registered when [`Feature::Scheduling`] is enabled. They manage entries
//! in the user's system crontab via the helpers in `codex_scheduling::os_cron`.
//! Persistence is owned by the OS — schedules survive ata exit.

use serde::Deserialize;
use serde::Serialize;

mod create;
mod delete;
mod list;

pub use create::CronCreateHandler;
pub use delete::CronDeleteHandler;
pub use list::CronListHandler;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CronCreateArgs {
    cron_expr: String,
    prompt: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CronDeleteArgs {
    task_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CronCreateResponse {
    task_id: String,
    next_fire_at: Option<String>,
    log_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CronJobSummary {
    task_id: String,
    cron_expr: String,
    prompt: String,
    next_fire_at: Option<String>,
    log_path: String,
    created_at: Option<String>,
    fire_count: u64,
    last_fired_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CronListResponse {
    jobs: Vec<CronJobSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CronDeleteResponse {
    deleted: bool,
}
