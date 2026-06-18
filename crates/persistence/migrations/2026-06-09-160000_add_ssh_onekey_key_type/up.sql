ALTER TABLE ssh_onekey_credentials
ADD COLUMN kind TEXT NOT NULL DEFAULT 'password' CHECK(kind IN ('password','key'));

ALTER TABLE ssh_onekey_credentials
ADD COLUMN key_path TEXT DEFAULT NULL;
