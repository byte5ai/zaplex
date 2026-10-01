-- Keep the node IDs until their leaf rows are removed to satisfy foreign keys.
-- Leaving these nodes orphaned would make restoration discard terminal siblings.
CREATE TEMP TABLE zaplex_rollback_sftp_nodes AS
SELECT pane_node_id AS id FROM pane_leaves WHERE kind = 'sftp';
DELETE FROM pane_leaves WHERE kind = 'sftp';
DELETE FROM pane_nodes WHERE id IN (SELECT id FROM zaplex_rollback_sftp_nodes);
DROP TABLE zaplex_rollback_sftp_nodes;
DROP TABLE sftp_panes;
DROP TABLE temporary_file_manager_replacements;
DROP TABLE remote_terminal_pane_identities;
