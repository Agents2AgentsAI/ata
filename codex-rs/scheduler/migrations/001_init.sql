-- Jobs metadata (synced from TOML files on disk)
CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    definition_hash TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    paused INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_run_at INTEGER,
    next_run_at INTEGER,
    run_count INTEGER NOT NULL DEFAULT 0,
    consecutive_failures INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_jobs_next_run ON jobs(next_run_at) WHERE enabled = 1 AND paused = 0;

-- Run history
CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    status TEXT NOT NULL,  -- pending, running, success, failed, timeout, skipped
    attempt INTEGER NOT NULL DEFAULT 1,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    duration_ms INTEGER,
    output_path TEXT,
    output_preview TEXT,
    error_message TEXT,
    trigger_data TEXT,
    delivery_results TEXT
);
CREATE INDEX IF NOT EXISTS idx_runs_job ON runs(job_id, started_at DESC);

-- Per-job persistent KV state (dedup cursors, counters, etc.)
CREATE TABLE IF NOT EXISTS job_state (
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (job_id, key)
);
