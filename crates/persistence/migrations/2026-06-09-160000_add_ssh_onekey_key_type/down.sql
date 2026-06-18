CREATE TABLE ssh_onekey_credentials_backup (
  id          TEXT PRIMARY KEY NOT NULL,
  label       TEXT NOT NULL,
  username    TEXT NOT NULL DEFAULT '',
  created_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO ssh_onekey_credentials_backup
SELECT id, label, username, created_at, updated_at
FROM ssh_onekey_credentials;

DROP TABLE ssh_onekey_credentials;
ALTER TABLE ssh_onekey_credentials_backup RENAME TO ssh_onekey_credentials;
