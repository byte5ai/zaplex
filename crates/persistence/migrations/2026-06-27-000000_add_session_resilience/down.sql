-- Drop the column directly. Unlike the older `ssh_servers` rollbacks (which
-- rebuilt the table for pre-3.35 SQLite compatibility), this is safe and
-- preferred here: the app bundles SQLite >= 3.35 (libsqlite3-sys), which
-- supports ALTER TABLE ... DROP COLUMN, and the live `ssh_servers` schema has
-- since gained a modified auth_type CHECK and a credential_id foreign key — so
-- a hand-replicated rebuild would be more error-prone than a direct drop.
ALTER TABLE ssh_servers DROP COLUMN session_resilience;
