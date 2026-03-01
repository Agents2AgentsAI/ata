use crate::error::WorkspaceError;
use crate::lock::FileLock;
use crate::paths;

/// Run a command under a fine-grained workspace lock.
/// Returns the exit code of the child process.
pub fn run(
    workspace_id: &str,
    level: &str,
    target_id: Option<&str>,
    command: &[String],
) -> Result<i32, WorkspaceError> {
    if (level == "run" || level == "index") && target_id.is_none() {
        return Err(WorkspaceError::TargetIdRequired(level.to_string()));
    }
    if command.is_empty() {
        return Err(WorkspaceError::NoCommand);
    }

    let lock_file = paths::lock_file_path(workspace_id, level, target_id);
    let _lock = FileLock::acquire(&lock_file)?;

    let status = std::process::Command::new(&command[0])
        .args(&command[1..])
        .status()?;
    Ok(status.code().unwrap_or(1))
}
