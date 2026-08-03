CREATE TABLE terminal_pane_cli_agent_bindings (
    terminal_pane_id INTEGER PRIMARY KEY NOT NULL,
    binding_json TEXT NOT NULL,
    FOREIGN KEY (terminal_pane_id) REFERENCES terminal_panes(id) ON DELETE CASCADE
);
