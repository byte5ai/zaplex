-- Per-host scrollback/replay buffer ceiling for a daemon session, in MiB.
-- 0 (the default) means "use the daemon's built-in default ceiling". Only
-- meaningful when session_resilience is enabled; it sizes the daemon-side
-- OutputRing that backs replay/reattach.
ALTER TABLE ssh_servers ADD COLUMN ring_ceiling_mb INTEGER NOT NULL DEFAULT 0;
