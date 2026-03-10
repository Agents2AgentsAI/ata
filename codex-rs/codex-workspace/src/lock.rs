use crate::error::WorkspaceError;
use std::path::Path;
use std::path::PathBuf;

const LOCK_TIMEOUT_S: u64 = 30;
const LOCK_POLL_INTERVAL_MS: u64 = 50;

/// A file lock backed by `flock(2)` on Unix.
#[cfg(unix)]
pub struct FileLock {
    fd: std::os::unix::io::RawFd,
    path: PathBuf,
}

#[cfg(unix)]
impl FileLock {
    /// Acquire an exclusive lock on the given path, with a 30s timeout.
    pub fn acquire(lock_path: &Path) -> Result<Self, WorkspaceError> {
        use std::os::unix::io::IntoRawFd;

        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        let fd = file.into_raw_fd();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(LOCK_TIMEOUT_S);

        loop {
            let ret = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
            if ret == 0 {
                return Ok(Self {
                    fd,
                    path: lock_path.to_path_buf(),
                });
            }
            if std::time::Instant::now() >= deadline {
                unsafe { libc::close(fd) };
                return Err(WorkspaceError::LockTimeout {
                    path: lock_path.to_path_buf(),
                    timeout_secs: LOCK_TIMEOUT_S,
                });
            }
            std::thread::sleep(std::time::Duration::from_millis(LOCK_POLL_INTERVAL_MS));
        }
    }

    /// Release the lock.
    pub fn release(self) {
        // Drop impl handles this
        drop(self);
    }

    /// Get the lock file path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(unix)]
impl Drop for FileLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.fd, libc::LOCK_UN);
            libc::close(self.fd);
        }
    }
}

// Best-effort lock for non-Unix platforms backed by exclusive lockfile creation.
#[cfg(not(unix))]
pub struct FileLock {
    file: Option<std::fs::File>,
    path: PathBuf,
}

#[cfg(not(unix))]
impl FileLock {
    pub fn acquire(lock_path: &Path) -> Result<Self, WorkspaceError> {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(LOCK_TIMEOUT_S);
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(lock_path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    let _ = writeln!(file, "pid={}", std::process::id());
                    let _ = writeln!(file, "created_at={}", chrono::Utc::now().timestamp());
                    return Ok(Self {
                        file: Some(file),
                        path: lock_path.to_path_buf(),
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if try_reclaim_stale_lock(lock_path) {
                        continue;
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(WorkspaceError::LockTimeout {
                            path: lock_path.to_path_buf(),
                            timeout_secs: LOCK_TIMEOUT_S,
                        });
                    }
                    std::thread::sleep(std::time::Duration::from_millis(LOCK_POLL_INTERVAL_MS));
                }
                Err(err) => return Err(err.into()),
            }
        }
    }

    pub fn release(self) {
        drop(self);
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(not(unix))]
impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.file.take();
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(not(unix))]
fn try_reclaim_stale_lock(lock_path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(lock_path) else {
        return false;
    };
    let Some(pid) = contents.lines().find_map(parse_lock_pid) else {
        return false;
    };
    if lock_owner_is_alive(pid) {
        return false;
    }
    std::fs::remove_file(lock_path).is_ok()
}

#[cfg(not(unix))]
fn parse_lock_pid(line: &str) -> Option<u32> {
    line.strip_prefix("pid=")?.trim().parse().ok()
}

#[cfg(all(not(unix), windows))]
fn lock_owner_is_alive(pid: u32) -> bool {
    let filter = format!("PID eq {pid}");
    std::process::Command::new("tasklist")
        .args(["/FI", &filter, "/NH"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            let stdout = String::from_utf8_lossy(&output.stdout);
            !stdout.contains("No tasks are running")
        })
        .unwrap_or(true)
}

#[cfg(all(not(unix), not(windows)))]
fn lock_owner_is_alive(_pid: u32) -> bool {
    true
}
