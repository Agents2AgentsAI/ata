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

// Stub for non-Unix platforms (no-op lock).
#[cfg(not(unix))]
pub struct FileLock {
    path: PathBuf,
}

#[cfg(not(unix))]
impl FileLock {
    pub fn acquire(lock_path: &Path) -> Result<Self, WorkspaceError> {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self {
            path: lock_path.to_path_buf(),
        })
    }

    pub fn release(self) {
        drop(self);
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
