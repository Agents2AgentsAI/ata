//! Phase 4: durable snapshot of `CronRegistry` + `MonitorRegistry` +
//! `LoopRegistry` for a single thread.
//!
//! The snapshot is written next to the thread's rollout journal so the
//! `~/.codex/sessions/…` directory remains the single source of truth for
//! per-thread state. On `ata resume <thread_id>`, the session loader hydrates
//! the registries from this file; on every mutation (create/delete/stop) the
//! caller re-writes the file atomically.
//!
//! Why sidecar JSON instead of weaving into the rollout journal: scheduling
//! state is *mutable* (statuses, counters, fire timestamps), while the
//! rollout is an append-only narrative. They have different shapes, and
//! keeping them separate lets a future daemon process (Phase 7) read jobs
//! without parsing chat journals.

use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use crate::cron_job::CronJob;
use crate::loop_task::LoopTask;
use crate::monitor::MonitorTask;

/// Persisted scheduling state for one thread. Each `Vec` mirrors the
/// in-memory registry; loaders rebuild the registries from these vectors.
///
/// Schema version is `1`. If we add new fields we can rely on
/// `#[serde(default)]` for backward compatibility; major shape changes
/// would bump the version and require an explicit migration step.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchedulingSnapshot {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub cron_jobs: Vec<CronJob>,
    #[serde(default)]
    pub monitors: Vec<MonitorTask>,
    #[serde(default)]
    pub loops: Vec<LoopTask>,
}

fn default_version() -> u32 {
    1
}

impl SchedulingSnapshot {
    pub fn new(cron_jobs: Vec<CronJob>, monitors: Vec<MonitorTask>, loops: Vec<LoopTask>) -> Self {
        Self {
            version: 1,
            cron_jobs,
            monitors,
            loops,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cron_jobs.is_empty() && self.monitors.is_empty() && self.loops.is_empty()
    }
}

/// Resolve the scheduling sidecar path for a given thread under the supplied
/// codex_home root. Layout: `<codex_home>/scheduling/<thread_id>.json`.
pub fn scheduling_state_path(codex_home: &Path, thread_id: &str) -> PathBuf {
    let mut p = codex_home.to_path_buf();
    p.push("scheduling");
    p.push(format!("{thread_id}.json"));
    p
}

/// Read a snapshot from disk. Returns `Ok(None)` when the file doesn't
/// exist (fresh sessions are not an error), `Err` only for genuine I/O or
/// parse failures.
pub fn load(path: &Path) -> io::Result<Option<SchedulingSnapshot>> {
    match fs::read(path) {
        Ok(bytes) => {
            let snap: SchedulingSnapshot = serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            tracing::info!(
                target: "codex_scheduling::persist",
                path = %path.display(),
                cron_count = snap.cron_jobs.len(),
                monitor_count = snap.monitors.len(),
                loop_count = snap.loops.len(),
                "scheduling.persist.loaded"
            );
            Ok(Some(snap))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Atomically write a snapshot: write to `<path>.tmp`, fsync, rename. This
/// prevents readers from observing a half-written JSON file if the process
/// is killed mid-write. The parent directory is created if missing.
pub fn save(path: &Path, snapshot: &SchedulingSnapshot) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = with_suffix(path, ".tmp");
    let body = serde_json::to_vec_pretty(snapshot)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&body)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    tracing::debug!(
        target: "codex_scheduling::persist",
        path = %path.display(),
        cron_count = snapshot.cron_jobs.len(),
        monitor_count = snapshot.monitors.len(),
        loop_count = snapshot.loops.len(),
        bytes = body.len(),
        "scheduling.persist.saved"
    );
    Ok(())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cron_job::CronJob;
    use crate::loop_task::LoopTask;
    use crate::monitor::MonitorTask;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn load_returns_none_when_file_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.json");
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn save_then_load_roundtrips_all_three_kinds() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("snap.json");

        let cron = CronJob::new("0 * * * * *".into(), "tick".into()).unwrap();
        let mon = MonitorTask::new("ping -c 3 google.com".into());
        let lp = LoopTask::new_fixed("poll".into(), Duration::from_secs(30));

        let snap = SchedulingSnapshot::new(vec![cron.clone()], vec![mon.clone()], vec![lp.clone()]);
        save(&path, &snap).unwrap();

        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.cron_jobs.len(), 1);
        assert_eq!(loaded.cron_jobs[0].id, cron.id);
        assert_eq!(loaded.monitors.len(), 1);
        assert_eq!(loaded.monitors[0].id, mon.id);
        assert_eq!(loaded.loops.len(), 1);
        assert_eq!(loaded.loops[0].id, lp.id);
    }

    #[test]
    fn save_is_atomic_via_rename() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.json");
        let snap = SchedulingSnapshot::default();
        save(&path, &snap).unwrap();
        // The tmp file should not survive a successful save.
        assert!(!path.with_extension("json.tmp").exists());
        assert!(path.exists());
    }

    #[test]
    fn scheduling_state_path_uses_thread_id() {
        let home = Path::new("/tmp/test-codex");
        let p = scheduling_state_path(home, "abc-123");
        assert!(p.ends_with("scheduling/abc-123.json"));
    }
}
