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
    if let Some(target_id) = target_id {
        validate_target_id(target_id)?;
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

fn validate_target_id(target_id: &str) -> Result<(), WorkspaceError> {
    if crate::paths::is_safe_single_component(target_id) {
        Ok(())
    } else {
        Err(WorkspaceError::InvalidTargetId {
            id: target_id.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_target_id_rejects_path_traversal() {
        assert!(matches!(
            validate_target_id("../etc"),
            Err(WorkspaceError::InvalidTargetId { .. })
        ));
        assert!(matches!(
            validate_target_id("nested/path"),
            Err(WorkspaceError::InvalidTargetId { .. })
        ));
    }

    #[test]
    fn validate_target_id_accepts_single_component() {
        assert!(validate_target_id("run-123").is_ok());
    }
}
