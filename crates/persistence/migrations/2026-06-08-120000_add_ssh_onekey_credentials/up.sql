CREATE TABLE ssh_onekey_credentials (
  id          TEXT PRIMARY KEY NOT NULL,
  label       TEXT NOT NULL,
  username    TEXT NOT NULL DEFAULT '',
  created_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE ssh_servers_new (
  node_id           TEXT PRIMARY KEY NOT NULL REFERENCES ssh_nodes(id) ON DELETE CASCADE,
  host              TEXT NOT NULL,
  port              INTEGER NOT NULL DEFAULT 22,
  username          TEXT NOT NULL DEFAULT '',
  auth_type         TEXT NOT NULL CHECK(auth_type IN ('password','key','onekey')) DEFAULT 'password',
  key_path          TEXT,
  startup_command   TEXT DEFAULT NULL,
  notes             TEXT DEFAULT NULL,
  last_connected_at TIMESTAMP,
  credential_id     TEXT REFERENCES ssh_onekey_credentials(id) ON DELETE SET NULL
);

INSERT INTO ssh_servers_new
SELECT node_id, host, port, username, auth_type, key_path, startup_command, notes, last_connected_at, NULL
FROM ssh_servers;

DROP TABLE ssh_servers;
ALTER TABLE ssh_servers_new RENAME TO ssh_servers;
