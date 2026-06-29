-- Direct drop; the app bundles SQLite >= 3.35 (libsqlite3-sys) which supports
-- ALTER TABLE ... DROP COLUMN.
ALTER TABLE ssh_servers DROP COLUMN ring_ceiling_mb;
