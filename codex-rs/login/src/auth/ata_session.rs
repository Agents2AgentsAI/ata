//! ATA Supabase session storage — minimal reader/writer for
//! `$codex_home/ata_session.json`.
//!
//! The canonical writer lives in `codex_core::supabase::session`. This file
//! mirrors the schema enough for `codex-login::load_auth` to reconstruct
//! `CodexAuth::Ata` at session start without taking a dependency on
//! `codex-core` (which would introduce a cycle, since `codex-core` already
//! depends on `codex-login`).
//!
//! Schema drift between this loader and the canonical writer would cause
//! a sign-in to succeed but the next launch to fail to reload the session.
//! The fields below are stable and shared API at the file-format layer.

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use std::fs::File;
use std::io;
use std::io::Read;
use std::path::Path;

/// Filename written under `$codex_home`. Must match
/// `codex_core::supabase::session::SESSION_FILENAME`.
const SESSION_FILENAME: &str = "ata_session.json";

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AtaSession {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
    pub user: AtaSessionUser,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AtaSessionUser {
    /// Supabase auth user id (uuid). Stored as the `account_id` on the
    /// resulting `AtaAuth`.
    pub id: String,
    #[serde(default)]
    pub email: Option<String>,
}

/// Read `$codex_home/ata_session.json` if it exists. Returns `Ok(None)` if
/// the file is absent (the common case for non-ATA-signed-in users); any
/// I/O or parse error is propagated so the caller can fall back without
/// silently swallowing real problems.
pub(crate) fn load_ata_session(codex_home: &Path) -> io::Result<Option<AtaSession>> {
    let path = codex_home.join(SESSION_FILENAME);
    let mut file = match File::open(&path) {
        Ok(f) => f,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let session: AtaSession = serde_json::from_str(&contents)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    Ok(Some(session))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn returns_none_when_file_missing() {
        let dir = tempdir().unwrap();
        let result = load_ata_session(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn round_trips_session_written_by_core_helper() {
        let dir = tempdir().unwrap();
        let json = r#"{
            "access_token": "eyJabc",
            "refresh_token": "rt_abc",
            "expires_at": "2099-01-01T00:00:00Z",
            "user": {
                "id": "11111111-2222-3333-4444-555555555555",
                "email": "alice@example.com",
                "display_name": "Alice"
            }
        }"#;
        fs::write(dir.path().join("ata_session.json"), json).unwrap();
        let session = load_ata_session(dir.path()).unwrap().expect("session loads");
        assert_eq!(session.access_token, "eyJabc");
        assert_eq!(session.refresh_token, "rt_abc");
        assert_eq!(session.user.email.as_deref(), Some("alice@example.com"));
    }

    #[test]
    fn rejects_malformed_json() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("ata_session.json"), "not valid json").unwrap();
        let err = load_ata_session(dir.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
