//! Unit tests for panel.rs — covers pure logic like tree building, parent resolution, and display sorting.
//!
//! Author: logic

use super::*;
use chrono::NaiveDateTime;
use diesel::Connection;
use diesel_migrations::MigrationHarness;
use warp_ssh_manager::{NodeKind, OneKeyCredentialKind, SshNode};

// --- Test helpers -------------------------------------------------------

fn ts() -> NaiveDateTime {
    chrono::DateTime::from_timestamp(0, 0).unwrap().naive_utc()
}

fn folder(id: &str, parent_id: Option<&str>, name: &str, sort_order: i32) -> SshNode {
    SshNode {
        id: id.to_string(),
        parent_id: parent_id.map(|s| s.to_string()),
        kind: NodeKind::Folder,
        name: name.to_string(),
        sort_order,
        created_at: ts(),
        updated_at: ts(),
        is_collapsed: false,
    }
}

fn server(id: &str, parent_id: Option<&str>, name: &str, sort_order: i32) -> SshNode {
    SshNode {
        id: id.to_string(),
        parent_id: parent_id.map(|s| s.to_string()),
        kind: NodeKind::Server,
        name: name.to_string(),
        sort_order,
        created_at: ts(),
        updated_at: ts(),
        is_collapsed: false,
    }
}

#[cfg(unix)]
#[test]
fn tailscale_lookup_uses_injected_workspace_command_factory() {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;
    use std::sync::Mutex;
    use warp_ssh_manager::WorkspaceCommandFactory;

    struct RecordingCommandFactory {
        script: std::path::PathBuf,
        programs: Mutex<Vec<String>>,
    }

    impl WorkspaceCommandFactory for RecordingCommandFactory {
        fn async_command(&self, program: &str) -> command::r#async::Command {
            self.programs.lock().unwrap().push(program.to_string());
            let mut command = command::r#async::Command::new(&self.script);
            command.arg(program);
            command
        }

        fn blocking_command(&self, program: &str) -> command::blocking::Command {
            self.programs.lock().unwrap().push(program.to_string());
            let mut command = command::blocking::Command::new(&self.script);
            command.arg(program);
            command
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake-command");
    let mut file = std::fs::File::create(&script).unwrap();
    file.write_all(b"#!/bin/sh\nprintf '%s\\n' \"$@\"\n")
        .unwrap();
    let mut permissions = file.metadata().unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&script, permissions).unwrap();
    drop(file);
    let factory = RecordingCommandFactory {
        script,
        programs: Mutex::new(Vec::new()),
    };

    let output = tailscale_status_output(&factory).unwrap();

    assert!(output.status.success());
    assert_eq!(factory.programs.lock().unwrap().as_slice(), ["tailscale"]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "tailscale\nstatus\n--json\n"
    );
}

#[test]
fn invalid_import_port_is_rejected_without_fallback() {
    let candidate = SshConfigCandidate {
        alias: "example.com".to_string(),
        hostname: None,
        user: None,
        port: None,
        invalid_port: Some("70000".to_string()),
        identity_file: None,
    };

    assert_eq!(
        validate_candidate_endpoint(&candidate),
        Err(SshEndpointValidationError::InvalidPort)
    );
}

#[test]
fn missing_import_port_still_uses_the_explicit_default() {
    let candidate = SshConfigCandidate {
        alias: "example.com".to_string(),
        hostname: None,
        user: None,
        port: None,
        invalid_port: None,
        identity_file: None,
    };

    assert_eq!(validate_candidate_endpoint(&candidate).unwrap().port, 22);
}

// --- resolve_parent_for_new_node tests ----------------------------------------

#[test]
fn parent_no_selection_returns_none() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    assert_eq!(resolve_parent_for_new_node(None, &nodes), None);
}

#[test]
fn parent_folder_selected_returns_folder_id() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    assert_eq!(
        resolve_parent_for_new_node(Some("f1"), &nodes),
        Some("f1".to_string())
    );
}

#[test]
fn parent_server_at_root_selected_returns_none() {
    let nodes = vec![server("s1", None, "srv", 0)];
    assert_eq!(resolve_parent_for_new_node(Some("s1"), &nodes), None);
}

#[test]
fn parent_server_under_folder_selected_returns_folder_id() {
    let nodes = vec![
        folder("f1", None, "Prod", 0),
        server("s1", Some("f1"), "web", 0),
    ];
    assert_eq!(
        resolve_parent_for_new_node(Some("s1"), &nodes),
        Some("f1".to_string())
    );
}

#[test]
fn parent_invalid_selected_id_returns_none() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    assert_eq!(
        resolve_parent_for_new_node(Some("nonexistent"), &nodes),
        None
    );
}

#[test]
fn parent_empty_nodes_with_selection_returns_none() {
    assert_eq!(resolve_parent_for_new_node(Some("any"), &[]), None);
}

#[test]
fn parent_deeply_nested_folder_selected_returns_immediate_parent() {
    // f1(root) → f2(child) → s1(grandchild server)
    let nodes = vec![
        folder("f1", None, "L0", 0),
        folder("f2", Some("f1"), "L1", 0),
        server("s1", Some("f2"), "srv", 0),
    ];
    // Select f2 → new node created under f2
    assert_eq!(
        resolve_parent_for_new_node(Some("f2"), &nodes),
        Some("f2".to_string())
    );
    // Select s1 → new node created under s1's parent (f2) (sibling semantics)
    assert_eq!(
        resolve_parent_for_new_node(Some("s1"), &nodes),
        Some("f2".to_string())
    );
}

#[test]
fn delete_confirmation_lists_all_descendant_hosts() {
    let nodes = vec![
        folder("f1", None, "Production", 0),
        server("s1", Some("f1"), "web", 0),
        folder("f2", Some("f1"), "Databases", 1),
        server("s2", Some("f2"), "db", 0),
        server("s3", None, "unrelated", 1),
    ];
    let credential_labels = HashMap::from([
        ("s1".to_string(), vec!["Deploy key (ops)".to_string()]),
        ("s3".to_string(), vec!["Unrelated key".to_string()]),
    ]);

    let impact = build_delete_impact(&nodes, "f1", &credential_labels).unwrap();

    assert_eq!(impact.node_name, "Production");
    assert_eq!(impact.node_kind, NodeKind::Folder);
    assert_eq!(impact.host_names, vec!["web", "db"]);
    assert_eq!(
        impact.credential_impacts,
        vec![DeleteCredentialImpact {
            host_name: "web".to_string(),
            credential_label: "Deploy key (ops)".to_string(),
        }]
    );
}

#[test]
fn host_delete_impact_never_includes_sibling_hosts() {
    let nodes = vec![
        folder("f1", None, "Production", 0),
        server("s1", Some("f1"), "web", 0),
        server("s2", Some("f1"), "db", 1),
    ];
    let credential_labels = HashMap::from([("s1".to_string(), vec!["Deploy key".to_string()])]);

    let impact = build_delete_impact(&nodes, "s1", &credential_labels).unwrap();

    assert_eq!(impact.node_kind, NodeKind::Server);
    assert_eq!(impact.host_names, vec!["web"]);
    assert_eq!(impact.credential_impacts.len(), 1);
    assert_eq!(impact.credential_impacts[0].host_name, "web");
}

#[test]
fn changed_descendants_require_a_fresh_delete_confirmation() {
    let mut nodes = vec![
        folder("f1", None, "Production", 0),
        server("s1", Some("f1"), "web", 0),
    ];
    let original = build_delete_impact(&nodes, "f1", &HashMap::new()).unwrap();
    nodes.push(server("s2", Some("f1"), "db", 1));
    let current = build_delete_impact(&nodes, "f1", &HashMap::new()).unwrap();

    assert_ne!(current, original);
    assert_eq!(current.host_names, vec!["web", "db"]);
}

#[test]
fn delete_confirmation_lists_every_credential_impact_for_mixed_auth_hosts() {
    let nodes = vec![
        folder("f1", None, "Mixed", 0),
        server("s1", Some("f1"), "password", 0),
        server("s2", Some("f1"), "key", 1),
        server("s3", Some("f1"), "onekey", 2),
    ];
    let password_impacts = delete_credential_impacts(
        &DeleteHostExpectation {
            node_id: "s1".to_string(),
            auth_type: AuthType::Password,
            key_path: None,
            credential_id: None,
            secret_kinds: vec![SecretKind::Password, SecretKind::RootPassword],
        },
        &HashMap::new(),
    );
    let key_impacts = delete_credential_impacts(
        &DeleteHostExpectation {
            node_id: "s2".to_string(),
            auth_type: AuthType::Key,
            key_path: Some("/keys/id_ed25519".to_string()),
            credential_id: None,
            secret_kinds: vec![SecretKind::Passphrase],
        },
        &HashMap::new(),
    );
    let onekey_impacts = delete_credential_impacts(
        &DeleteHostExpectation {
            node_id: "s3".to_string(),
            auth_type: AuthType::OneKey,
            key_path: None,
            credential_id: Some("credential-1".to_string()),
            secret_kinds: Vec::new(),
        },
        &HashMap::from([("credential-1".to_string(), "Shared credential".to_string())]),
    );
    let credential_impacts = HashMap::from([
        ("s1".to_string(), password_impacts.clone()),
        ("s2".to_string(), key_impacts.clone()),
        ("s3".to_string(), onekey_impacts.clone()),
    ]);

    assert_eq!(
        password_impacts,
        vec![
            crate::t!("workspace-left-panel-ssh-manager-delete-credential-password"),
            crate::t!("workspace-left-panel-ssh-manager-delete-credential-root-password"),
        ]
    );
    assert_eq!(
        key_impacts,
        vec![
            crate::t!(
                "workspace-left-panel-ssh-manager-delete-credential-identity-file",
                path = "/keys/id_ed25519"
            ),
            crate::t!("workspace-left-panel-ssh-manager-delete-credential-passphrase"),
        ]
    );
    assert_eq!(
        onekey_impacts,
        vec![crate::t!(
            "workspace-left-panel-ssh-manager-delete-credential-onekey-assignment",
            label = "Shared credential"
        )]
    );

    let impact = build_delete_impact(&nodes, "f1", &credential_impacts).unwrap();

    assert_eq!(impact.host_names, vec!["password", "key", "onekey"]);
    assert_eq!(
        impact.credential_impacts,
        vec![
            DeleteCredentialImpact {
                host_name: "password".to_string(),
                credential_label: password_impacts[0].clone(),
            },
            DeleteCredentialImpact {
                host_name: "password".to_string(),
                credential_label: password_impacts[1].clone(),
            },
            DeleteCredentialImpact {
                host_name: "key".to_string(),
                credential_label: key_impacts[0].clone(),
            },
            DeleteCredentialImpact {
                host_name: "key".to_string(),
                credential_label: key_impacts[1].clone(),
            },
            DeleteCredentialImpact {
                host_name: "onekey".to_string(),
                credential_label: onekey_impacts[0].clone(),
            },
        ]
    );
}

#[test]
fn large_folder_delete_impact_keeps_every_host_and_credential_impact() {
    let mut nodes = vec![folder("f1", None, "Large fleet", 0)];
    let mut credential_impacts = HashMap::new();
    for index in 0..100 {
        let id = format!("s{index}");
        nodes.push(server(&id, Some("f1"), &format!("host-{index:03}"), index));
        if index % 2 == 0 {
            credential_impacts.insert(id, vec![format!("credential-{index:03}")]);
        }
    }

    let impact = build_delete_impact(&nodes, "f1", &credential_impacts).unwrap();

    assert_eq!(impact.host_names.len(), 100);
    assert_eq!(
        impact.host_names.first().map(String::as_str),
        Some("host-000")
    );
    assert_eq!(
        impact.host_names.last().map(String::as_str),
        Some("host-099")
    );
    assert_eq!(impact.credential_impacts.len(), 50);
}

// --- compute_depths tests -------------------------------------------------

#[test]
fn depths_empty_nodes() {
    let depths = compute_depths(&[]);
    assert!(depths.is_empty());
}

#[test]
fn depths_single_root() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    let depths = compute_depths(&nodes);
    assert_eq!(depths["f1"], 0);
}

#[test]
fn depths_nested_tree() {
    let nodes = vec![
        folder("f1", None, "Root", 0),
        folder("f2", Some("f1"), "Child", 0),
        server("s1", Some("f2"), "Grandchild", 0),
    ];
    let depths = compute_depths(&nodes);
    assert_eq!(depths["f1"], 0);
    assert_eq!(depths["f2"], 1);
    assert_eq!(depths["s1"], 2);
}

#[test]
fn depths_multiple_roots() {
    let nodes = vec![
        folder("f1", None, "Root1", 0),
        folder("f2", None, "Root2", 1),
        server("s1", Some("f1"), "srv", 0),
        server("s2", Some("f2"), "srv", 0),
    ];
    let depths = compute_depths(&nodes);
    assert_eq!(depths["f1"], 0);
    assert_eq!(depths["f2"], 0);
    assert_eq!(depths["s1"], 1);
    assert_eq!(depths["s2"], 1);
}

// --- sort_for_display tests -----------------------------------------------

#[test]
fn sort_empty() {
    let depths = HashMap::new();
    let sorted = sort_for_display(vec![], &depths);
    assert!(sorted.is_empty());
}

#[test]
fn sort_single_root() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    assert_eq!(sorted.len(), 1);
    assert_eq!(sorted[0].id, "f1");
}

#[test]
fn sort_respects_parent_child_order() {
    let nodes = vec![
        server("s1", Some("f1"), "web", 0),
        folder("f1", None, "Prod", 0),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    // f1 comes first, s1 comes second
    assert_eq!(sorted[0].id, "f1");
    assert_eq!(sorted[1].id, "s1");
}

#[test]
fn sort_preserves_existing_folder_children_in_tree_order() {
    let nodes = vec![
        server("s2", Some("f2"), "db", 1),
        folder("f2", None, "Stage", 1),
        server("s1", Some("f1"), "web", 0),
        folder("f1", None, "Prod", 0),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();

    assert_eq!(ids, &["f1", "s1", "f2", "s2"]);
    assert_eq!(depths["s1"], 1);
    assert_eq!(depths["s2"], 1);
}

#[test]
fn sort_multiple_roots_by_sort_order() {
    let nodes = vec![folder("f2", None, "B", 1), folder("f1", None, "A", 0)];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    assert_eq!(sorted[0].id, "f1");
    assert_eq!(sorted[1].id, "f2");
}

#[test]
fn sort_deeply_nested() {
    let nodes = vec![
        folder("f1", None, "Root", 0),
        server("s2", Some("f2"), "deep", 1),
        folder("f2", Some("f1"), "Child", 0),
        server("s1", Some("f1"), "shallow", 1),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, &["f1", "f2", "s2", "s1"]);
}

#[test]
fn sort_multiple_roots_with_children() {
    let nodes = vec![
        folder("f2", None, "Stage", 1),
        folder("f1", None, "Prod", 0),
        server("s1", Some("f1"), "web", 0),
        server("s2", Some("f2"), "app", 0),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();
    // f1 (Prod) and its children come first, f2 (Stage) and its children come later
    assert_eq!(ids, &["f1", "s1", "f2", "s2"]);
}

#[test]
fn sort_keeps_orphaned_existing_nodes_visible_as_roots() {
    let nodes = vec![
        server("s1", Some("missing-folder"), "legacy", 0),
        folder("f1", None, "New folder", 1),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();

    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"s1"));
    assert!(ids.contains(&"f1"));
    assert_eq!(depths["s1"], 0);
    assert_eq!(depths["f1"], 0);
}

#[test]
fn format_ring_bytes_is_human_readable_and_none_at_zero() {
    use super::format_ring_bytes;
    assert_eq!(format_ring_bytes(0), None);
    // < 1 MiB → KB, rounded up so a non-zero footprint never shows "0 KB".
    assert_eq!(format_ring_bytes(1), Some("1 KB".to_string()));
    assert_eq!(format_ring_bytes(500 * 1024), Some("500 KB".to_string()));
    // >= 1 MiB → MB with one decimal.
    assert_eq!(format_ring_bytes(1024 * 1024), Some("1.0 MB".to_string()));
    assert_eq!(
        format_ring_bytes(3 * 1024 * 1024 + 512 * 1024),
        Some("3.5 MB".to_string())
    );
}

#[test]
fn host_ring_usage_uses_only_reported_ring_bytes_and_cap() {
    let inventory = SessionList {
        sessions: vec![
            remote_server::proto::SessionInfo {
                ring_bytes: 40,
                ..Default::default()
            },
            remote_server::proto::SessionInfo {
                ring_bytes: 39,
                ..Default::default()
            },
        ],
        host_ring_cap_bytes: 100,
    };
    assert_eq!(
        host_ring_usage(&inventory),
        Some(HostRingUsage {
            used_bytes: 79,
            cap_bytes: 100,
            tone: HostRingTone::Calm,
        })
    );

    let mut warning = inventory.clone();
    warning.sessions[1].ring_bytes = 40;
    assert_eq!(
        host_ring_usage(&warning).unwrap().tone,
        HostRingTone::Warning
    );

    let mut critical = inventory;
    critical.sessions[1].ring_bytes = 60;
    assert_eq!(
        host_ring_usage(&critical).unwrap().tone,
        HostRingTone::Critical
    );
}

#[test]
fn old_daemon_without_host_cap_never_gets_a_guessed_aggregate() {
    let inventory = SessionList {
        sessions: vec![remote_server::proto::SessionInfo {
            ring_bytes: 64 * 1024 * 1024,
            ..Default::default()
        }],
        host_ring_cap_bytes: 0,
    };
    assert_eq!(host_ring_usage(&inventory), None);
}

#[test]
fn multiplexer_kind_selects_only_non_destructive_attach_modes() {
    assert_eq!(
        multiplexer_attach_mode(MultiplexerKind::Tmux as i32, 0),
        Some(MultiplexerAttachMode::Tmux)
    );
    assert_eq!(
        multiplexer_attach_mode(MultiplexerKind::ByobuTmux as i32, 2),
        Some(MultiplexerAttachMode::Tmux)
    );
    assert_eq!(
        multiplexer_attach_mode(MultiplexerKind::ByobuScreen as i32, 0),
        Some(MultiplexerAttachMode::ScreenDetached)
    );
    assert_eq!(
        multiplexer_attach_mode(MultiplexerKind::ByobuScreen as i32, 1),
        Some(MultiplexerAttachMode::ScreenAttached)
    );
    assert_eq!(
        multiplexer_attach_mode(MultiplexerKind::Unspecified as i32, 0),
        None
    );
}

#[test]
fn multiplexer_connection_resolves_onekey_to_effective_ssh_auth() {
    let mut conn = diesel::sqlite::SqliteConnection::establish(":memory:").unwrap();
    conn.run_pending_migrations(persistence::MIGRATIONS)
        .unwrap();
    let credential = SshRepository::create_onekey_credential(
        &mut conn,
        "shared-key",
        "deploy",
        OneKeyCredentialKind::Key,
        Some("/home/deploy/.ssh/id_ed25519"),
    )
    .unwrap();
    let mut info = SshServerInfo::new_default(String::new());
    info.host = "edge.example.com".into();
    info.auth_type = AuthType::OneKey;
    info.username = "ignored-local-user".into();
    info.credential_id = Some(credential.id);
    let node = SshRepository::create_server(&mut conn, None, "edge", &info).unwrap();

    let resolved = resolve_server_for_node(&mut conn, &node.id)
        .unwrap()
        .unwrap();

    assert_eq!(resolved.username, "deploy");
    assert_eq!(resolved.auth_type, AuthType::Key);
    assert_eq!(
        resolved.key_path.as_deref(),
        Some("/home/deploy/.ssh/id_ed25519")
    );
}
