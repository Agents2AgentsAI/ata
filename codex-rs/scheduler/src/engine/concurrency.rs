use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::sync::Semaphore;

/// Guards concurrent job execution. Limits overall concurrency and tracks
/// which jobs are currently running for overlap detection.
pub struct ConcurrencyGuard {
    semaphore: Arc<Semaphore>,
    running: Arc<Mutex<HashSet<String>>>,
}

impl ConcurrencyGuard {
    /// Create a new guard with a maximum number of concurrent jobs.
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            running: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Try to acquire a slot for the given job. Returns `None` if the job is
    /// already running (and `skip_if_running` semantics should apply).
    pub async fn try_acquire(&self, job_id: &str, skip_if_running: bool) -> Option<JobGuard> {
        if skip_if_running {
            let running = self.running.lock().await;
            if running.contains(job_id) {
                return None;
            }
        }

        // Acquire a semaphore permit (blocks if at capacity).
        let permit = self.semaphore.clone().acquire_owned().await.ok()?;

        let mut running = self.running.lock().await;
        if skip_if_running && running.contains(job_id) {
            // Race: another task started between our check and acquire.
            drop(permit);
            return None;
        }
        running.insert(job_id.to_string());

        Some(JobGuard {
            job_id: job_id.to_string(),
            running: Arc::clone(&self.running),
            _permit: permit,
        })
    }
}

/// RAII guard that releases the concurrency slot when dropped.
pub struct JobGuard {
    job_id: String,
    running: Arc<Mutex<HashSet<String>>>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        let running = Arc::clone(&self.running);
        let job_id = self.job_id.clone();
        // Spawn a blocking-safe cleanup. In practice, the lock is uncontended
        // at drop time so this completes instantly.
        tokio::spawn(async move {
            let mut set = running.lock().await;
            set.remove(&job_id);
        });
    }
}
