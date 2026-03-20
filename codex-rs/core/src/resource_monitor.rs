//! File descriptor / resource tracking and cleanup.
//!
//! Provides lightweight helpers to monitor the current process's open file
//! descriptor count against the OS soft limit. Used by the unified exec system
//! to avoid hitting `EMFILE` ("Too many open files") during long-running agent
//! sessions.

/// Percentage of the FD soft limit at which we start warning.
const FD_WARNING_THRESHOLD_PCT: f64 = 70.0;

/// Percentage of the FD soft limit at which we refuse to spawn new processes.
const FD_CRITICAL_THRESHOLD_PCT: f64 = 85.0;

/// Minimum recommended FD soft limit. If the user's `ulimit -n` is below this
/// value we log a warning at startup.
pub(crate) const MIN_RECOMMENDED_FD_LIMIT: u64 = 1024;

/// Snapshot of the current file descriptor usage.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct FdUsage {
    /// Number of currently open file descriptors.
    pub current: usize,
    /// Soft limit (`ulimit -n` equivalent).
    pub limit: usize,
    /// `current / limit * 100`.
    pub percentage: f64,
}

/// Health status derived from [`FdUsage`].
#[derive(Debug, Clone)]
pub(crate) enum FdHealth {
    Healthy,
    Warning(f64),
    Critical(f64),
}

impl std::fmt::Display for FdHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FdHealth::Healthy => write!(f, "healthy"),
            FdHealth::Warning(pct) => write!(f, "warning ({pct:.1}% of FD limit)"),
            FdHealth::Critical(pct) => write!(f, "critical ({pct:.1}% of FD limit)"),
        }
    }
}

// ── Unix implementation ────────────────────────────────────────────────

#[cfg(unix)]
mod platform {
    /// Returns the current soft limit for open file descriptors via
    /// `getrlimit(RLIMIT_NOFILE)`.
    pub(super) fn fd_soft_limit() -> Option<usize> {
        let mut rlim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: rlim is a valid, stack-allocated struct pointer.
        let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) };
        if rc == 0 {
            Some(rlim.rlim_cur as usize)
        } else {
            None
        }
    }

    /// Counts currently open file descriptors.
    ///
    /// On macOS `/dev/fd` is the canonical procfs-like mount for per-process FD
    /// enumeration (Linux has `/proc/self/fd`). We try both.
    pub(super) fn count_open_fds() -> Option<usize> {
        // Try Linux path first, then macOS path.
        for path in &["/proc/self/fd", "/dev/fd"] {
            if let Ok(entries) = std::fs::read_dir(path) {
                // Each entry in the directory corresponds to an open FD.
                // `read_dir` itself opens one FD, so the count is approximate.
                return Some(entries.count());
            }
        }
        None
    }
}

#[cfg(not(unix))]
mod platform {
    pub(super) fn fd_soft_limit() -> Option<usize> {
        None
    }

    pub(super) fn count_open_fds() -> Option<usize> {
        None
    }
}

/// Returns a snapshot of the current FD usage, or `None` if the platform does
/// not support the required queries.
pub(crate) fn fd_usage() -> Option<FdUsage> {
    let current = platform::count_open_fds()?;
    let limit = platform::fd_soft_limit()?;
    let percentage = if limit > 0 {
        (current as f64 / limit as f64) * 100.0
    } else {
        0.0
    };
    Some(FdUsage {
        current,
        limit,
        percentage,
    })
}

/// Evaluates the health of the current FD usage.
pub(crate) fn check_fd_health() -> FdHealth {
    match fd_usage() {
        Some(usage) if usage.percentage >= FD_CRITICAL_THRESHOLD_PCT => {
            FdHealth::Critical(usage.percentage)
        }
        Some(usage) if usage.percentage >= FD_WARNING_THRESHOLD_PCT => {
            FdHealth::Warning(usage.percentage)
        }
        _ => FdHealth::Healthy,
    }
}

/// Logs a warning if the FD soft limit is below [`MIN_RECOMMENDED_FD_LIMIT`].
///
/// Call this once during session initialization.
pub(crate) fn check_fd_limit_at_startup() {
    if let Some(limit) = platform::fd_soft_limit() {
        if (limit as u64) < MIN_RECOMMENDED_FD_LIMIT {
            tracing::warn!(
                fd_limit = limit,
                recommended = MIN_RECOMMENDED_FD_LIMIT,
                "Open file descriptor limit is low. Consider increasing it \
                 with `ulimit -n 4096` to avoid 'Too many open files' errors \
                 during long sessions."
            );
        } else {
            tracing::debug!(fd_limit = limit, "FD soft limit check passed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fd_usage_returns_some_on_supported_platforms() {
        // On Unix (CI and local dev) we always expect a result.
        if cfg!(unix) {
            let usage = fd_usage().expect("fd_usage should succeed on Unix");
            assert!(usage.current > 0, "should have at least one open FD");
            assert!(usage.limit > 0, "soft limit should be positive");
            assert!(usage.percentage > 0.0);
            assert!(usage.percentage <= 100.0 || usage.current > usage.limit);
        }
    }

    #[test]
    fn check_fd_health_returns_healthy_normally() {
        // Under normal test conditions we should not be near the limit.
        let health = check_fd_health();
        assert!(
            matches!(health, FdHealth::Healthy),
            "expected Healthy during tests, got {health:?}"
        );
    }

    #[test]
    fn fd_health_display() {
        assert_eq!(FdHealth::Healthy.to_string(), "healthy");
        assert!(FdHealth::Warning(75.0).to_string().contains("warning"));
        assert!(FdHealth::Critical(90.0).to_string().contains("critical"));
    }
}
