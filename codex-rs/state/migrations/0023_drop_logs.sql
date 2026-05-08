PRAGMA auto_vacuum = INCREMENTAL;

-- The logs table is no longer written to from the state DB; logs are routed
-- to the dedicated logs DB instead. Empty out the table but keep the schema
-- so existing readers (e.g. test fixtures, dashboards) can still issue
-- queries against it without failing.
DELETE FROM logs;
