-- SQLite cannot drop a column on older versions, so rebuild ssh_servers
-- without credential_id and then drop the shared credential table.
CREATE TABLE ssh_servers_backup (
  node_id           TEXT PRIMARY KEY NOT NULL REFERENCES ssh_nodes(id) ON DELETE CASCADE,
  host              TEXT NOT NULL,
  port              INTEGER NOT NULL DEFAULT 22,
  username          TEXT NOT NULL DEFAULT '',
  auth_type         TEXT NOT NULL CHECK(auth_type IN ('password','key')) DEFAULT 'password',
  key_path          TEXT,
  startup_command   TEXT DEFAULT NULL,
  notes             TEXT DEFAULT NULL,
  last_connected_at TIMESTAMP
);

INSERT INTO ssh_servers_backup
SELECT
  node_id,
  host,
  port,
  username,
  CASE auth_type WHEN 'onekey' THEN 'password' ELSE auth_type END,
  key_path,
  startup_command,
  notes,
  last_connected_at
FROM ssh_servers;

DROP TABLE ssh_servers;
ALTER TABLE ssh_servers_backup RENAME TO ssh_servers;
DROP TABLE ssh_onekey_credentials;
