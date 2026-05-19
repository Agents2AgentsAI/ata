//! `CronJob` — a recurring prompt fired on a cron schedule, scoped to the
//! enclosing agent session.

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::str::FromStr;
use thiserror::Error;

use crate::task::TaskId;
use crate::task::TaskStatus;

#[derive(Debug, Error)]
pub enum CronError {
    #[error("invalid cron expression: {0}")]
    InvalidExpression(String),
}

/// One scheduled, recurring prompt. The cron expression is validated at
/// construction; the rest of the fields are bookkeeping that later phases
/// (timer engine, persistence) will read and update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: TaskId,
    /// Validated 5- or 6-field cron expression.
    pub cron_expr: String,
    /// The prompt re-injected as a user message when the job fires.
    pub prompt: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub next_fire_at: Option<DateTime<Utc>>,
    pub fire_count: u64,
    /// When `true` (the new default), firings run silently — the TUI hides
    /// the agent's natural-language reply so periodic crons don't flood the
    /// chat. Tool call cells still render. See `LoopTask::background`.
    #[serde(default = "default_background")]
    pub background: bool,
    /// Optional. Stop firing after this many total firings. `Some(1)` makes
    /// the job effectively one-shot ("at 3pm tomorrow, do X — then done").
    /// `None` (default) means run forever (until deleted).
    #[serde(default)]
    pub max_firings: Option<u64>,
    /// Optional. Stop firing after this wall-clock time. Use for
    /// finite-duration schedules ("every weekday at 9am until next Friday").
    /// `None` (default) means run forever.
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
    /// Optional. Fixed UTC offset string (e.g. `"+07:00"`, `"-05:00"`,
    /// `"Z"` / `"UTC"`). When set, the cron expression is interpreted as
    /// wall-clock time at this offset. `None` (default) = UTC. Does NOT
    /// auto-adjust for DST — callers in DST zones must update the offset
    /// twice a year (or pass UTC and compute the offset themselves).
    #[serde(default)]
    pub timezone: Option<String>,
}

/// Parse a UTC-offset string into a `FixedOffset`. Accepts `+HH:MM`,
/// `+HHMM`, `-HH:MM`, `-HHMM`, `Z`, and `UTC` (case-insensitive).
pub fn parse_utc_offset(s: &str) -> Option<chrono::FixedOffset> {
    if s == "Z" || s.eq_ignore_ascii_case("UTC") {
        return chrono::FixedOffset::east_opt(0);
    }
    let mut chars = s.chars();
    let sign = match chars.next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let rest: String = chars.filter(|c| *c != ':').collect();
    if rest.len() != 4 || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = rest[0..2].parse().ok()?;
    let minutes: i32 = rest[2..4].parse().ok()?;
    if hours > 14 || minutes > 59 {
        return None;
    }
    chrono::FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))
}

fn default_background() -> bool {
    true
}

impl CronJob {
    /// Construct a new pending `CronJob` with background mode enabled (the
    /// new default). Returns `CronError::InvalidExpression` if `cron_expr`
    /// is not a parseable cron schedule.
    pub fn new(cron_expr: String, prompt: String) -> Result<Self, CronError> {
        Self::new_with_background(cron_expr, prompt, true)
    }

    pub fn new_with_background(
        cron_expr: String,
        prompt: String,
        background: bool,
    ) -> Result<Self, CronError> {
        Self::new_with_options(cron_expr, prompt, background, None, None)
    }

    pub fn new_with_options(
        cron_expr: String,
        prompt: String,
        background: bool,
        max_firings: Option<u64>,
        until: Option<DateTime<Utc>>,
    ) -> Result<Self, CronError> {
        Self::new_with_full_options(cron_expr, prompt, background, max_firings, until, None)
    }

    pub fn new_with_full_options(
        cron_expr: String,
        prompt: String,
        background: bool,
        max_firings: Option<u64>,
        until: Option<DateTime<Utc>>,
        timezone: Option<String>,
    ) -> Result<Self, CronError> {
        cron::Schedule::from_str(&cron_expr)
            .map_err(|e| CronError::InvalidExpression(e.to_string()))?;
        if let Some(tz) = timezone.as_deref()
            && parse_utc_offset(tz).is_none()
        {
            return Err(CronError::InvalidExpression(format!(
                "invalid UTC offset `{tz}` (expected `+HH:MM`, `-HH:MM`, `Z`, or `UTC`)"
            )));
        }
        Ok(Self {
            id: TaskId::new(),
            cron_expr,
            prompt,
            status: TaskStatus::Pending,
            created_at: Utc::now(),
            last_fired_at: None,
            next_fire_at: None,
            fire_count: 0,
            background,
            max_firings,
            until,
            timezone,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_cron_expression() {
        let err = CronJob::new("not a cron expression".into(), "do the thing".into()).unwrap_err();
        assert!(matches!(err, CronError::InvalidExpression(_)));
    }

    #[test]
    fn accepts_standard_cron_expression() {
        // "0 9 * * * *" (sec min hour dom mon dow) — daily at 09:00:00 in the cron crate's 6-field form.
        let job = CronJob::new("0 0 9 * * *".into(), "check CI".into()).expect("valid cron");
        assert_eq!(job.status, TaskStatus::Pending);
        assert_eq!(job.fire_count, 0);
        assert!(job.last_fired_at.is_none());
    }

    #[test]
    fn each_job_gets_unique_id() {
        let a = CronJob::new("0 0 * * * *".into(), "a".into()).unwrap();
        let b = CronJob::new("0 0 * * * *".into(), "b".into()).unwrap();
        assert_ne!(a.id, b.id);
    }
}
