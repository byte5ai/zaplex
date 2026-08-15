use super::*;
use crate::secrets::test_support::InMemorySecretStore;

#[test]
fn test_section_key() {
    let provider = SshSyncProvider::new();
    assert_eq!(provider.section_key(), "ssh");
}

#[test]
fn test_sync_node_serialization_roundtrip() {
    let node = SyncNode {
        id: "n1".to_string(),
        parent_id: Some("p1".to_string()),
        kind: "folder".to_string(),
        name: "Prod".to_string(),
        sort_order: 0,
        is_collapsed: true,
    };
    let json = serde_json::to_string(&node).unwrap();
    let parsed: SyncNode = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.id, "n1");
    assert_eq!(parsed.parent_id, Some("p1".to_string()));
    assert_eq!(parsed.kind, "folder");
    assert_eq!(parsed.name, "Prod");
    assert_eq!(parsed.sort_order, 0);
    assert!(parsed.is_collapsed);
}

#[test]
fn test_sync_server_serialization_with_secrets() {
    let server = SyncServer {
        node_id: "s1".to_string(),
        host: "example.com".to_string(),
        port: 22,
        username: "root".to_string(),
        auth_type: "password".to_string(),
        key_path: Some("/key".to_string()),
        startup_command: None,
        notes: Some("test".to_string()),
        credential_id: None,
        session_resilience: "off".to_string(),
        ring_ceiling_mb: 0,
        password_encrypted: Some("enc123".to_string()),
        passphrase_encrypted: None,
        root_password_encrypted: Some("enc456".to_string()),
    };
    let json = serde_json::to_string(&server).unwrap();
    let parsed: SyncServer = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.node_id, "s1");
    assert_eq!(parsed.port, 22);
    assert_eq!(parsed.password_encrypted, Some("enc123".to_string()));
    assert_eq!(parsed.passphrase_encrypted, None);
    assert_eq!(parsed.root_password_encrypted, Some("enc456".to_string()));
}

#[test]
fn test_sync_server_no_secrets() {
    let server = SyncServer {
        node_id: "s2".to_string(),
        host: "host".to_string(),
        port: 2222,
        username: "admin".to_string(),
        auth_type: "key".to_string(),
        key_path: None,
        startup_command: None,
        notes: None,
        credential_id: None,
        session_resilience: "off".to_string(),
        ring_ceiling_mb: 0,
        password_encrypted: None,
        passphrase_encrypted: None,
        root_password_encrypted: None,
    };
    let json = serde_json::to_string(&server).unwrap();
    let parsed: SyncServer = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.password_encrypted, None);
    assert_eq!(parsed.passphrase_encrypted, None);
    assert_eq!(parsed.root_password_encrypted, None);
}

#[test]
fn test_ssh_sync_data_roundtrip() {
    let data = SshSyncData {
        nodes: vec![SyncNode {
            id: "n1".to_string(),
            parent_id: None,
            kind: "folder".to_string(),
            name: "Root".to_string(),
            sort_order: 0,
            is_collapsed: false,
        }],
        servers: vec![SyncServer {
            node_id: "s1".to_string(),
            host: "h".to_string(),
            port: 22,
            username: "u".to_string(),
            auth_type: "password".to_string(),
            key_path: None,
            startup_command: None,
            notes: None,
            credential_id: None,
            session_resilience: "off".to_string(),
            ring_ceiling_mb: 0,
            password_encrypted: Some("enc".to_string()),
            passphrase_encrypted: None,
            root_password_encrypted: None,
        }],
        onekey_credentials: Vec::new(),
    };
    let json = serde_json::to_string(&data).unwrap();
    let parsed: SshSyncData = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.nodes.len(), 1);
    assert_eq!(parsed.servers.len(), 1);
    assert_eq!(parsed.nodes[0].id, "n1");
    assert_eq!(
        parsed.servers[0].password_encrypted,
        Some("enc".to_string())
    );
}

#[test]
fn test_ssh_sync_data_deserializes_legacy_payload_without_onekey_fields() {
    let json = r#"{
        "nodes": [
            {
                "id": "s1",
                "parent_id": null,
                "kind": "server",
                "name": "legacy",
                "sort_order": 0,
                "is_collapsed": false
            }
        ],
        "servers": [
            {
                "node_id": "s1",
                "host": "example.com",
                "port": 22,
                "username": "root",
                "auth_type": "password",
                "key_path": null,
                "startup_command": null,
                "notes": null,
                "password_encrypted": null,
                "passphrase_encrypted": null,
                "root_password_encrypted": null
            }
        ]
    }"#;

    let parsed: SshSyncData = serde_json::from_str(json).unwrap();

    assert!(parsed.onekey_credentials.is_empty());
    assert_eq!(parsed.servers[0].credential_id, None);
    // A payload predating the field deserializes to the "off" default.
    assert_eq!(parsed.servers[0].session_resilience, "off");
}

#[test]
fn test_onekey_credential_serialization_roundtrip() {
    let data = SshSyncData {
        nodes: Vec::new(),
        servers: Vec::new(),
        onekey_credentials: vec![SyncOneKeyCredential {
            id: "cred-1".to_string(),
            label: "prod-root".to_string(),
            username: "root".to_string(),
            kind: "key".to_string(),
            key_path: Some("/home/root/.ssh/id_ed25519".to_string()),
            password_encrypted: Some("enc".to_string()),
        }],
    };

    let json = serde_json::to_string(&data).unwrap();
    let parsed: SshSyncData = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.onekey_credentials.len(), 1);
    assert_eq!(parsed.onekey_credentials[0].id, "cred-1");
    assert_eq!(parsed.onekey_credentials[0].label, "prod-root");
    assert_eq!(parsed.onekey_credentials[0].username, "root");
    assert_eq!(parsed.onekey_credentials[0].kind, "key");
    assert_eq!(
        parsed.onekey_credentials[0].key_path.as_deref(),
        Some("/home/root/.ssh/id_ed25519")
    );
    assert_eq!(
        parsed.onekey_credentials[0].password_encrypted,
        Some("enc".to_string())
    );
}

#[test]
fn test_onekey_credential_deserializes_legacy_payload_as_password() {
    let json = r#"{
        "id": "cred-1",
        "label": "prod-root",
        "username": "root",
        "password_encrypted": null
    }"#;

    let parsed: SyncOneKeyCredential = serde_json::from_str(json).unwrap();

    assert_eq!(parsed.kind, "password");
    assert_eq!(parsed.key_path, None);
}

#[test]
fn test_onekey_key_credentials_use_passphrase_secret_slot() {
    assert_eq!(
        onekey_secret_kind(OneKeyCredentialKind::Password),
        SecretKind::OneKeyPassword
    );
    assert_eq!(
        onekey_secret_kind(OneKeyCredentialKind::Key),
        SecretKind::Passphrase
    );
    assert_eq!(
        obsolete_onekey_secret_kind(SecretKind::OneKeyPassword),
        SecretKind::Passphrase
    );
    assert_eq!(
        obsolete_onekey_secret_kind(SecretKind::Passphrase),
        SecretKind::OneKeyPassword
    );
}

#[test]
fn sync_onekey_kind_change_cleans_obsolete_slot() {
    let data = SshSyncData {
        onekey_credentials: vec![SyncOneKeyCredential {
            id: "credential".into(),
            label: "production".into(),
            username: "root".into(),
            kind: OneKeyCredentialKind::Key.as_db_str().into(),
            key_path: Some("/keys/id_ed25519".into()),
            password_encrypted: None,
        }],
        ..SshSyncData::default()
    };

    let cleanup_targets = keychain_cleanup_targets(&data, Vec::new(), &[]);

    assert!(cleanup_targets.contains(&("credential".to_string(), SecretKind::OneKeyPassword,)));
    assert!(!cleanup_targets.contains(&("credential".to_string(), SecretKind::Passphrase,)));
}

#[test]
fn sync_failure_preserves_recoverable_state() {
    let store = InMemorySecretStore::default();
    store
        .set("server", SecretKind::Password, "old-password")
        .unwrap();

    let mutations = vec![
        WrittenSecret {
            node_id: "server".into(),
            kind: SecretKind::Password,
            prior_value: store.get("server", SecretKind::Password).unwrap(),
        },
        WrittenSecret {
            node_id: "server".into(),
            kind: SecretKind::Passphrase,
            prior_value: store.get("server", SecretKind::Passphrase).unwrap(),
        },
    ];
    store
        .set("server", SecretKind::Password, "incoming-password")
        .unwrap();
    store
        .set("server", SecretKind::Passphrase, "incoming-passphrase")
        .unwrap();

    assert!(rollback_keychain_writes(&store, &mutations).is_empty());
    assert_eq!(
        store
            .get("server", SecretKind::Password)
            .unwrap()
            .as_deref(),
        Some("old-password")
    );
    assert!(
        store
            .get("server", SecretKind::Passphrase)
            .unwrap()
            .is_none()
    );

    // Retrying the compensation must converge on the same pre-sync state.
    assert!(rollback_keychain_writes(&store, &mutations).is_empty());
    assert_eq!(
        store
            .get("server", SecretKind::Password)
            .unwrap()
            .as_deref(),
        Some("old-password")
    );
    assert!(
        store
            .get("server", SecretKind::Passphrase)
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_ssh_sync_data_default_empty() {
    let data = SshSyncData::default();
    assert!(data.nodes.is_empty());
    assert!(data.servers.is_empty());
}

#[test]
fn test_sync_node_null_parent() {
    let node = SyncNode {
        id: "root".to_string(),
        parent_id: None,
        kind: "folder".to_string(),
        name: "R".to_string(),
        sort_order: 0,
        is_collapsed: false,
    };
    let json = serde_json::to_string(&node).unwrap();
    assert!(
        json.contains("\"parent_id\":null"),
        "parent_id=None should serialize as null"
    );
    let parsed: SyncNode = serde_json::from_str(&json).unwrap();
    assert!(parsed.parent_id.is_none());
}
