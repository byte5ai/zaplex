CREATE TABLE remote_terminal_pane_identities (
    terminal_pane_id INTEGER PRIMARY KEY NOT NULL,
    identity_json TEXT NOT NULL,
    FOREIGN KEY (terminal_pane_id) REFERENCES terminal_panes(id) ON DELETE CASCADE
);

CREATE TABLE temporary_file_manager_replacements (
    terminal_pane_id INTEGER PRIMARY KEY NOT NULL,
    node_id TEXT NOT NULL,
    mode TEXT NOT NULL,
    current_path BLOB NOT NULL,
    FOREIGN KEY (terminal_pane_id) REFERENCES terminal_panes(id) ON DELETE CASCADE
);

CREATE TABLE sftp_panes (
    id INTEGER PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL DEFAULT 'sftp' CHECK(kind = 'sftp'),
    node_id TEXT NOT NULL,
    mode TEXT NOT NULL,
    current_path BLOB NOT NULL,
    FOREIGN KEY (id, kind) REFERENCES pane_leaves(pane_node_id, kind) ON DELETE CASCADE
);
