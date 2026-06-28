-- Per-host opt-in for the native persistent remote-session layer.
-- 'off' (the default) preserves today's behavior: SSH runs as a local PTY
-- executing `ssh host`. 'persist_only' / 'persist_plus_mosh' make the session
-- daemon-hosted (server-side persistence + replay/reattach; the latter also
-- selects the mosh-grade UDP transport, Phase B3).
ALTER TABLE ssh_servers ADD COLUMN session_resilience TEXT NOT NULL DEFAULT 'off';
