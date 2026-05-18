//! In-memory, thread-safe registry of active `CronJob`s.
//!
//! One instance lives per agent session (created at session init when
//! `Feature::Scheduling` is enabled). The engine task in `engine.rs` reads
//! from this registry to find jobs that are due; tool handlers
//! (`CronCreate`, `CronList`, `CronDelete`) mutate it.
//!
//! Phase 2a: in-memory only. Persistence to the session journal lands in
//! Phase 4.

use chrono::DateTime;
use chrono::Utc;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;

use crate::cron_job::CronJob;
use crate::task::TaskId;
use crate::task::TaskStatus;

#[derive(Debug, Default)]
pub struct CronRegistry {
    jobs: Mutex<HashMap<TaskId, CronJob>>,
}

impl CronRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a job. Computes the initial `next_fire_at` from `cron_expr`
    /// using `now` as the anchor. Returns the assigned `TaskId`.
    pub fn insert(&self, mut job: CronJob, now: DateTime<Utc>) -> TaskId {
        if job.next_fire_at.is_none() {
            job.next_fire_at =
                next_fire_after_in_tz(&job.cron_expr, now, job.timezone.as_deref());
        }
        let id = job.id.clone();
        self.jobs
            .lock()
            .expect("CronRegistry mutex poisoned")
            .insert(id.clone(), job);
        id
    }

    /// Remove a job by id. Returns the removed job if it existed.
    pub fn remove(&self, id: &TaskId) -> Option<CronJob> {
        self.jobs
            .lock()
            .expect("CronRegistry mutex poisoned")
            .remove(id)
    }

    /// Snapshot of all registered jobs.
    pub fn list(&self) -> Vec<CronJob> {
        self.jobs
            .lock()
            .expect("CronRegistry mutex poisoned")
            .values()
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.jobs
            .lock()
            .expect("CronRegistry mutex poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs
            .lock()
            .expect("CronRegistry mutex poisoned")
            .is_empty()
    }

    /// Replace the registry contents with the supplied jobs. Used on
    /// session resume to rehydrate state loaded from disk (Phase 4). Any
    /// job whose `next_fire_at` is `None` is left untouched — the engine
    /// will compute the next firing time on its next tick if appropriate.
    pub fn hydrate(&self, jobs: Vec<CronJob>) {
        let mut map = self.jobs.lock().expect("CronRegistry mutex poisoned");
        map.clear();
        for job in jobs {
            map.insert(job.id.clone(), job);
        }
    }

    /// Return prompts to fire now (whose `next_fire_at <= now`) and update
    /// their `next_fire_at`, `last_fired_at`, and `fire_count` in-place.
    ///
    /// Jobs whose expression has no future match (`next_fire_after` returns
    /// `None`) are marked `Completed` so subsequent ticks ignore them.
    ///
    /// Each item is `(task_id, prompt, background)` so the engine can encode
    /// the background flag in the submission id (`cronbg-...` vs `cron-...`)
    /// for the TUI's chat-cell filter.
    pub fn take_due(&self, now: DateTime<Utc>) -> Vec<(TaskId, String, bool)> {
        let mut jobs = self.jobs.lock().expect("CronRegistry mutex poisoned");
        let mut fired = Vec::new();
        for (id, job) in jobs.iter_mut() {
            if !matches!(
                job.status,
                TaskStatus::Pending | TaskStatus::Running
            ) {
                continue;
            }
            let Some(fire_at) = job.next_fire_at else {
                continue;
            };
            if fire_at <= now {
                fired.push((id.clone(), job.prompt.clone(), job.background));
                job.last_fired_at = Some(now);
                job.fire_count = job.fire_count.saturating_add(1);
                let candidate_next =
                    next_fire_after_in_tz(&job.cron_expr, now, job.timezone.as_deref());
                // Apply end conditions: max_firings caps total firings;
                // `until` caps wall-clock duration. If either disqualifies
                // the next candidate, drop it so the job transitions to
                // Completed below.
                let next = match candidate_next {
                    None => None,
                    Some(_) if job.max_firings.is_some_and(|cap| job.fire_count >= cap) => None,
                    Some(t) if job.until.is_some_and(|deadline| t > deadline) => None,
                    Some(t) => Some(t),
                };
                job.next_fire_at = next;
                job.status = if job.next_fire_at.is_some() {
                    TaskStatus::Pending
                } else {
                    TaskStatus::Completed
                };
            }
        }
        fired
    }
}

/// Compute the next firing time, optionally interpreting the cron
/// expression at a fixed UTC offset (e.g. `"+07:00"`). The returned
/// `DateTime<Utc>` is always normalized to UTC for storage.
fn next_fire_after_in_tz(
    cron_expr: &str,
    after: DateTime<Utc>,
    timezone: Option<&str>,
) -> Option<DateTime<Utc>> {
    let schedule = cron::Schedule::from_str(cron_expr).ok()?;
    match timezone {
        None => schedule.after(&after).next(),
        Some(offset_str) => {
            let offset = crate::cron_job::parse_utc_offset(offset_str)?;
            // Convert `after` to the target offset, get the next match in
            // that offset, then convert back to UTC for storage.
            let after_in_zone = after.with_timezone(&offset);
            schedule
                .after(&after_in_zone)
                .next()
                .map(|t| t.with_timezone(&Utc))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn make_job(expr: &str, prompt: &str) -> CronJob {
        CronJob::new(expr.into(), prompt.into()).expect("valid cron")
    }

    #[test]
    fn insert_then_list_returns_the_job() {
        let reg = CronRegistry::new();
        let job = make_job("0 0 0 * * *", "daily");
        let id = reg.insert(job, at(0));
        let listed = reg.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
    }

    #[test]
    fn insert_computes_next_fire_at() {
        let reg = CronRegistry::new();
        let job = make_job("0 0 0 * * *", "daily");
        let id = reg.insert(job, at(0));
        let job = reg.list().into_iter().find(|j| j.id == id).unwrap();
        assert!(job.next_fire_at.is_some());
    }

    #[test]
    fn remove_drops_the_job() {
        let reg = CronRegistry::new();
        let id = reg.insert(make_job("0 0 0 * * *", "x"), at(0));
        assert_eq!(reg.len(), 1);
        let removed = reg.remove(&id);
        assert!(removed.is_some());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn take_due_returns_nothing_when_not_yet_due() {
        let reg = CronRegistry::new();
        // 6-field cron: every minute at 30 seconds past
        reg.insert(make_job("30 * * * * *", "tick"), at(0));
        // Check at the next millisecond — definitely not yet due.
        let due = reg.take_due(at(1));
        assert!(due.is_empty());
    }

    #[test]
    fn take_due_fires_and_advances_next() {
        let reg = CronRegistry::new();
        let id = reg.insert(make_job("0 * * * * *", "tick"), at(0));
        // Far in the future — guaranteed past at least one fire time.
        let due = reg.take_due(at(3_600 * 24 * 365 * 10));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].0, id);
        assert_eq!(due[0].1, "tick");
        // Cron jobs default to background-mode in this slice.
        assert!(due[0].2);
        let job = reg.list().into_iter().find(|j| j.id == id).unwrap();
        assert_eq!(job.fire_count, 1);
        assert!(job.last_fired_at.is_some());
        // Recurring jobs return to Pending after firing so the panel reads
        // as "waiting for next fire" between firings.
        assert_eq!(job.status, TaskStatus::Pending);
    }

    #[test]
    fn take_due_skips_already_completed_jobs() {
        let reg = CronRegistry::new();
        let id = reg.insert(make_job("0 * * * * *", "tick"), at(0));
        {
            let mut jobs = reg.jobs.lock().unwrap();
            jobs.get_mut(&id).unwrap().status = TaskStatus::Completed;
        }
        let due = reg.take_due(at(3_600 * 24 * 365 * 10));
        assert!(due.is_empty());
    }
}
