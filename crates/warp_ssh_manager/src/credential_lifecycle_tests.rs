use std::collections::HashMap;
use std::sync::Mutex;

use zeroize::Zeroizing;

use super::*;
use crate::repository::setup_in_memory;
use crate::{AuthType, OneKeyCredentialKind, SessionResilience};

#[derive(Default)]
struct FailingSecretStore {
    values: Mutex<HashMap<(String, SecretKind), String>>,
    fail_set_kind: Mutex<Option<SecretKind>>,
    fail_delete_after_remove_kind: Mutex<Option<SecretKind>>,
    mutation_count: Mutex<usize>,
}

impl FailingSecretStore {
    fn fail_sets_for(&self, kind: SecretKind) {
        *self.fail_set_kind.lock().unwrap() = Some(kind);
    }

    fn fail_delete_after_remove_for(&self, kind: SecretKind) {
        *self.fail_delete_after_remove_kind.lock().unwrap() = Some(kind);
    }

    fn mutation_count(&self) -> usize {
        *self.mutation_count.lock().unwrap()
    }
}

impl SshSecretStore for FailingSecretStore {
    fn set(
        &self,
        node_id: &str,
        kind: SecretKind,
        secret: &str,
    ) -> Result<(), SshSecretStoreError> {
        *self.mutation_count.lock().unwrap() += 1;
        if *self.fail_set_kind.lock().unwrap() == Some(kind) {
            return Err(SshSecretStoreError::Keyring("injected set failure".into()));
        }
        self.values
            .lock()
            .unwrap()
            .insert((node_id.to_string(), kind), secret.to_string());
        Ok(())
    }

    fn get(
        &self,
        node_id: &str,
        kind: SecretKind,
    ) -> Result<Option<Zeroizing<String>>, SshSecretStoreError> {
        Ok(self
            .values
            .lock()
            .unwrap()
            .get(&(node_id.to_string(), kind))
            .cloned()
            .map(Zeroizing::new))
    }

    fn delete(&self, node_id: &str, kind: SecretKind) -> Result<(), SshSecretStoreError> {
        *self.mutation_count.lock().unwrap() += 1;
        self.values
            .lock()
            .unwrap()
            .remove(&(node_id.to_string(), kind));
        if *self.fail_delete_after_remove_kind.lock().unwrap() == Some(kind) {
            return Err(SshSecretStoreError::Keyring(
                "injected delete-after-remove failure".into(),
            ));
        }
        Ok(())
    }
}

fn server() -> SshServerInfo {
    SshServerInfo {
        node_id: String::new(),
        host: "example.com".into(),
        port: 22,
        username: "root".into(),
        auth_type: AuthType::Password,
        key_path: None,
        credential_id: None,
        startup_command: None,
        notes: None,
        last_connected_at: None,
        session_resilience: SessionResilience::Off,
        ring_ceiling_mb: 0,
    }
}

#[test]
fn delete_folder_removes_secrets_for_every_descendant_host() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let folder = SshRepository::create_folder(&mut conn, None, "prod").unwrap();
    let child =
        SshRepository::create_server(&mut conn, Some(&folder.id), "web", &server()).unwrap();
    let nested = SshRepository::create_folder(&mut conn, Some(&folder.id), "nested").unwrap();
    let grandchild =
        SshRepository::create_server(&mut conn, Some(&nested.id), "db", &server()).unwrap();
    for owner_id in [&child.id, &grandchild.id] {
        for kind in [
            SecretKind::Password,
            SecretKind::Passphrase,
            SecretKind::RootPassword,
        ] {
            store.set(owner_id, kind, "secret").unwrap();
        }
    }

    delete_node_and_secrets(&mut conn, &store, &folder.id).unwrap();

    for owner_id in [&child.id, &grandchild.id] {
        for kind in [
            SecretKind::Password,
            SecretKind::Passphrase,
            SecretKind::RootPassword,
        ] {
            assert!(store.get(owner_id, kind).unwrap().is_none());
        }
    }
}

#[test]
fn clone_host_rolls_back_when_secret_copy_fails() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let source = SshRepository::create_server(&mut conn, None, "source", &server()).unwrap();
    store
        .set(&source.id, SecretKind::Password, "password")
        .unwrap();
    store
        .set(&source.id, SecretKind::Passphrase, "passphrase")
        .unwrap();
    store.fail_sets_for(SecretKind::Passphrase);
    let cloned_info = SshServerInfo::clone_from_template(&server(), String::new());

    let result = clone_server_with_secrets(
        &mut conn,
        &store,
        &source.id,
        None,
        "source (copy)",
        &cloned_info,
    );

    assert!(result.is_err());
    assert_eq!(SshRepository::list_nodes(&mut conn).unwrap().len(), 1);
    assert_eq!(
        store
            .get(&source.id, SecretKind::Password)
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("password")
    );
}

#[test]
fn credential_operation_retry_is_idempotent() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let node = SshRepository::create_server(&mut conn, None, "server", &server()).unwrap();
    store
        .set(&node.id, SecretKind::Password, "password")
        .unwrap();

    delete_node_and_secrets(&mut conn, &store, &node.id).unwrap();
    delete_node_and_secrets(&mut conn, &store, &node.id).unwrap();

    assert!(SshRepository::list_nodes(&mut conn).unwrap().is_empty());
    assert!(store.get(&node.id, SecretKind::Password).unwrap().is_none());
}

#[test]
fn changing_onekey_kind_removes_obsolete_secret() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let existing = SshRepository::create_onekey_credential(
        &mut conn,
        "production",
        "root",
        OneKeyCredentialKind::Key,
        Some("/keys/id_ed25519"),
    )
    .unwrap();
    store
        .set(&existing.id, SecretKind::Passphrase, "old-passphrase")
        .unwrap();
    let updated = SshOneKeyCredential {
        kind: OneKeyCredentialKind::Password,
        key_path: None,
        ..existing.clone()
    };

    save_onekey_credential_with_secret(
        &mut conn,
        &store,
        Some(&existing),
        &updated,
        Some("new-password"),
    )
    .unwrap();

    assert!(
        store
            .get(&existing.id, SecretKind::Passphrase)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .get(&existing.id, SecretKind::OneKeyPassword)
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("new-password")
    );
}

#[test]
fn changing_server_auth_removes_obsolete_secret() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let node = SshRepository::create_server(&mut conn, None, "server", &server()).unwrap();
    store
        .set(&node.id, SecretKind::Password, "old-password")
        .unwrap();
    let mut updated = SshRepository::get_server(&mut conn, &node.id)
        .unwrap()
        .unwrap();
    updated.auth_type = AuthType::Key;
    updated.key_path = Some("/keys/id_ed25519".into());

    save_server_with_secrets(
        &mut conn,
        &store,
        SaveServerRequest {
            name: "server",
            server: &updated,
            move_to_parent: false,
            parent_id: None,
            password_or_passphrase: Some(""),
            root_password: Some(""),
        },
    )
    .unwrap();

    assert!(store.get(&node.id, SecretKind::Password).unwrap().is_none());
}

#[test]
fn keychain_failure_keeps_repository_state_recoverable() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let node = SshRepository::create_server(&mut conn, None, "server", &server()).unwrap();
    store
        .set(&node.id, SecretKind::Password, "password")
        .unwrap();
    store.fail_delete_after_remove_for(SecretKind::Password);

    assert!(delete_node_and_secrets(&mut conn, &store, &node.id).is_err());

    assert!(
        SshRepository::list_nodes(&mut conn)
            .unwrap()
            .iter()
            .any(|candidate| candidate.id == node.id)
    );
    assert_eq!(
        store
            .get(&node.id, SecretKind::Password)
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("password")
    );
}

#[test]
fn delete_after_remove_failure_restores_onekey_secret() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let credential = SshRepository::create_onekey_credential(
        &mut conn,
        "production",
        "root",
        OneKeyCredentialKind::Password,
        None,
    )
    .unwrap();
    store
        .set(&credential.id, SecretKind::OneKeyPassword, "password")
        .unwrap();
    store.fail_delete_after_remove_for(SecretKind::OneKeyPassword);

    assert!(delete_onekey_credential_and_secrets(&mut conn, &store, &credential.id).is_err());

    assert!(
        SshRepository::get_onekey_credential(&mut conn, &credential.id)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        store
            .get(&credential.id, SecretKind::OneKeyPassword)
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("password")
    );
}

#[test]
fn lifecycle_errors_use_localized_ui_presenter() {
    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_dir = crate_dir.parent().unwrap().parent().unwrap();
    let server_view =
        std::fs::read_to_string(repo_dir.join("app/src/ssh_manager/server_view.rs")).unwrap();
    let panel = std::fs::read_to_string(repo_dir.join("app/src/ssh_manager/panel.rs")).unwrap();

    assert!(
        server_view
            .matches("credential_operation_message(&e)")
            .count()
            >= 3
    );
    assert!(panel.matches("credential_operation_message(&e)").count() >= 2);
}

#[test]
fn referenced_onekey_delete_does_not_touch_secrets() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let credential = SshRepository::create_onekey_credential(
        &mut conn,
        "shared",
        "deploy",
        OneKeyCredentialKind::Password,
        None,
    )
    .unwrap();
    store
        .set(&credential.id, SecretKind::OneKeyPassword, "password")
        .unwrap();
    let mutations_before = store.mutation_count();
    let mut info = server();
    info.auth_type = AuthType::OneKey;
    info.credential_id = Some(credential.id.clone());
    SshRepository::create_server(&mut conn, None, "edge", &info).unwrap();

    assert!(delete_onekey_credential_and_secrets(&mut conn, &store, &credential.id).is_err());
    assert_eq!(store.mutation_count(), mutations_before);
    assert_eq!(
        store
            .get(&credential.id, SecretKind::OneKeyPassword)
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("password")
    );
}

#[test]
fn onekey_delete_requires_target_specific_confirmation() {
    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_dir = crate_dir.parent().unwrap().parent().unwrap();
    let source =
        std::fs::read_to_string(repo_dir.join("app/src/ssh_manager/server_view.rs")).unwrap();

    assert!(source.contains("pending_onekey_delete_id"));
    assert!(source.contains("ConfirmDeleteManagedOneKeyCredential"));
    assert!(source.contains("CancelDeleteManagedOneKeyCredential"));
}
