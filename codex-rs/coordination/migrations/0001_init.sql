-- Coordination tables for cross-session agent awareness.

CREATE TABLE IF NOT EXISTS sessions (
    session_id   TEXT PRIMARY KEY,
    repo_path    TEXT NOT NULL,
    branch       TEXT,
    description  TEXT,
    started_at   INTEGER NOT NULL DEFAULT (unixepoch()),
    last_heartbeat INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS idx_sessions_repo ON sessions(repo_path);

CREATE TABLE IF NOT EXISTS messages (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id   TEXT NOT NULL,
    repo_path    TEXT NOT NULL,
    message      TEXT NOT NULL,
    message_type TEXT NOT NULL DEFAULT 'info',
    created_at   INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS idx_messages_repo_created ON messages(repo_path, created_at);
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
