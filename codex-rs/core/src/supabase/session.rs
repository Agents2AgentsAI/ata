//! ATA session storage — persists `SupabaseSession` to `$codex_home/ata_session.json`.
//!
//! This is completely independent from the provider-auth `auth.json` file.

use chrono::Utc;
use std::fs::File;
use std::fs::OpenOptions;
use std::fs::{self};
use std::io::Read;
use std::io::Write;
use std::io::{self};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;

use super::types::SupabaseSession;

/// Name of the session file within `$codex_home`.
const SESSION_FILENAME: &str = "ata_session.json";

/// Return the full path to the session file.
fn session_file_path(codex_home: &Path) -> PathBuf {
    codex_home.join(SESSION_FILENAME)
}

/// Load a previously-saved ATA session from disk.
///
/// Returns `Ok(None)` if the file does not exist.
pub fn load_ata_session(codex_home: &Path) -> io::Result<Option<SupabaseSession>> {
    let path = session_file_path(codex_home);
    let mut file = match File::open(&path) {
        Ok(f) => f,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let session: SupabaseSession = serde_json::from_str(&contents)?;
    Ok(Some(session))
}

/// Persist an ATA session to disk with restrictive (0600) permissions.
///
/// Creates parent directories if they do not already exist.
pub fn save_ata_session(codex_home: &Path, session: &SupabaseSession) -> io::Result<()> {
    let path = session_file_path(codex_home);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json_data = serde_json::to_string_pretty(session)?;

    let mut options = OpenOptions::new();
    options.truncate(true).write(true).create(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(json_data.as_bytes())?;
    file.flush()?;
    Ok(())
}

/// Delete the ATA session file.
///
/// Returns `Ok(true)` if the file was removed, `Ok(false)` if it did not exist.
pub fn delete_ata_session(codex_home: &Path) -> io::Result<bool> {
    let path = session_file_path(codex_home);
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

/// Check whether a session has expired (i.e. `expires_at` is at or before the current time).
pub fn is_session_expired(session: &SupabaseSession) -> bool {
    session.expires_at <= Utc::now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn sample_session(expires_at: &str) -> SupabaseSession {
        use super::super::types::SupabaseUser;
        SupabaseSession {
            access_token: "eyJabc".to_string(),
            refresh_token: "rt_abc".to_string(),
            expires_at: DateTime::parse_from_rfc3339(expires_at)
                .unwrap()
                .with_timezone(&Utc),
            user: SupabaseUser {
                id: Uuid::nil(),
                email: "test@example.com".to_string(),
                display_name: Some("Test".to_string()),
            },
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        let session = sample_session("2099-01-01T00:00:00Z");

        save_ata_session(dir.path(), &session).unwrap();
        let loaded = load_ata_session(dir.path()).unwrap();

        assert_eq!(loaded, Some(session));
    }

    #[test]
    fn load_returns_none_when_missing() {
        let dir = tempdir().unwrap();
        let loaded = load_ata_session(dir.path()).unwrap();
        assert_eq!(loaded, None);
    }

    #[test]
    fn delete_returns_true_when_file_exists() {
        let dir = tempdir().unwrap();
        let session = sample_session("2099-01-01T00:00:00Z");
        save_ata_session(dir.path(), &session).unwrap();

        let removed = delete_ata_session(dir.path()).unwrap();
        assert!(removed);
        assert!(!dir.path().join(SESSION_FILENAME).exists());
    }

    #[test]
    fn delete_returns_false_when_file_missing() {
        let dir = tempdir().unwrap();
        let removed = delete_ata_session(dir.path()).unwrap();
        assert!(!removed);
    }

    #[test]
    fn is_session_expired_past() {
        let session = sample_session("2000-01-01T00:00:00Z");
        assert!(is_session_expired(&session));
    }

    #[test]
    fn is_session_expired_future() {
        let session = sample_session("2099-01-01T00:00:00Z");
        assert!(!is_session_expired(&session));
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_has_restricted_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let session = sample_session("2099-01-01T00:00:00Z");
        save_ata_session(dir.path(), &session).unwrap();

        let metadata = std::fs::metadata(dir.path().join(SESSION_FILENAME)).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
