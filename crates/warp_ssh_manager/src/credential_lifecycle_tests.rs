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

fn delete_expectation(
    conn: &mut diesel::sqlite::SqliteConnection,
    store: &dyn SshSecretStore,
    node_id: &str,
) -> DeleteNodeExpectation {
    prepare_delete_node(conn, store, node_id).unwrap().unwrap()
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

    let expected = delete_expectation(&mut conn, &store, &folder.id);
    delete_node_and_secrets(&mut conn, &store, &expected).unwrap();

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
fn changed_delete_impact_is_rejected_before_keychain_mutation() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let folder = SshRepository::create_folder(&mut conn, None, "prod").unwrap();
    SshRepository::create_server(&mut conn, Some(&folder.id), "web", &server()).unwrap();
    let expected = delete_expectation(&mut conn, &store, &folder.id);
    SshRepository::create_server(&mut conn, Some(&folder.id), "db", &server()).unwrap();
    let mutations_before = store.mutation_count();

    assert!(matches!(
        delete_node_and_secrets(&mut conn, &store, &expected),
        Err(CredentialOperationError::ImpactChanged { node_id }) if node_id == folder.id
    ));
    assert_eq!(store.mutation_count(), mutations_before);
    assert_eq!(SshRepository::list_nodes(&mut conn).unwrap().len(), 3);
}

#[test]
fn changed_credential_reference_is_rejected_before_keychain_mutation() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let first = SshRepository::create_onekey_credential(
        &mut conn,
        "first",
        "root",
        OneKeyCredentialKind::Password,
        None,
    )
    .unwrap();
    let second = SshRepository::create_onekey_credential(
        &mut conn,
        "second",
        "root",
        OneKeyCredentialKind::Password,
        None,
    )
    .unwrap();
    let mut info = server();
    info.auth_type = AuthType::OneKey;
    info.credential_id = Some(first.id.clone());
    let node = SshRepository::create_server(&mut conn, None, "server", &info).unwrap();
    let expected = delete_expectation(&mut conn, &store, &node.id);
    let mut updated = SshRepository::get_server(&mut conn, &node.id)
        .unwrap()
        .unwrap();
    updated.credential_id = Some(second.id);
    SshRepository::update_server(&mut conn, &updated).unwrap();

    assert!(matches!(
        delete_node_and_secrets(&mut conn, &store, &expected),
        Err(CredentialOperationError::ImpactChanged { node_id }) if node_id == node.id
    ));
    assert_eq!(store.mutation_count(), 0);
    assert!(
        SshRepository::get_server(&mut conn, &node.id)
            .unwrap()
            .is_some()
    );
}

#[test]
fn delete_expectation_captures_mixed_host_credentials_without_secret_values() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let folder = SshRepository::create_folder(&mut conn, None, "mixed").unwrap();
    let password =
        SshRepository::create_server(&mut conn, Some(&folder.id), "password", &server()).unwrap();
    store
        .set(&password.id, SecretKind::Password, "password-value")
        .unwrap();
    store
        .set(&password.id, SecretKind::RootPassword, "root-value")
        .unwrap();

    let mut key_info = server();
    key_info.auth_type = AuthType::Key;
    key_info.key_path = Some("/keys/id_ed25519".into());
    let key = SshRepository::create_server(&mut conn, Some(&folder.id), "key", &key_info).unwrap();
    store
        .set(&key.id, SecretKind::Passphrase, "passphrase-value")
        .unwrap();

    let credential = SshRepository::create_onekey_credential(
        &mut conn,
        "shared",
        "root",
        OneKeyCredentialKind::Password,
        None,
    )
    .unwrap();
    let mut onekey_info = server();
    onekey_info.auth_type = AuthType::OneKey;
    onekey_info.credential_id = Some(credential.id.clone());
    let onekey =
        SshRepository::create_server(&mut conn, Some(&folder.id), "onekey", &onekey_info).unwrap();

    let expected = delete_expectation(&mut conn, &store, &folder.id);
    let password_impact = expected
        .hosts
        .iter()
        .find(|host| host.node_id == password.id)
        .unwrap();
    assert_eq!(password_impact.auth_type, AuthType::Password);
    assert_eq!(
        password_impact.secret_kinds,
        vec![SecretKind::Password, SecretKind::RootPassword]
    );
    let key_impact = expected
        .hosts
        .iter()
        .find(|host| host.node_id == key.id)
        .unwrap();
    assert_eq!(key_impact.auth_type, AuthType::Key);
    assert_eq!(key_impact.key_path.as_deref(), Some("/keys/id_ed25519"));
    assert_eq!(key_impact.secret_kinds, vec![SecretKind::Passphrase]);
    let onekey_impact = expected
        .hosts
        .iter()
        .find(|host| host.node_id == onekey.id)
        .unwrap();
    assert_eq!(onekey_impact.auth_type, AuthType::OneKey);
    assert_eq!(onekey_impact.credential_id.as_ref(), Some(&credential.id));
    assert!(onekey_impact.secret_kinds.is_empty());
}

#[test]
fn changed_auth_configuration_requires_fresh_confirmation() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let node = SshRepository::create_server(&mut conn, None, "server", &server()).unwrap();
    let expected = delete_expectation(&mut conn, &store, &node.id);
    let mut updated = SshRepository::get_server(&mut conn, &node.id)
        .unwrap()
        .unwrap();
    updated.auth_type = AuthType::Key;
    updated.key_path = Some("/keys/id_ed25519".into());
    SshRepository::update_server(&mut conn, &updated).unwrap();

    assert!(matches!(
        delete_node_and_secrets(&mut conn, &store, &expected),
        Err(CredentialOperationError::ImpactChanged { node_id }) if node_id == node.id
    ));
    assert_eq!(store.mutation_count(), 0);
}

#[test]
fn changed_identity_file_reference_requires_fresh_confirmation() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let mut info = server();
    info.auth_type = AuthType::Key;
    info.key_path = Some("/keys/old_ed25519".into());
    let node = SshRepository::create_server(&mut conn, None, "server", &info).unwrap();
    let expected = delete_expectation(&mut conn, &store, &node.id);
    let mut updated = SshRepository::get_server(&mut conn, &node.id)
        .unwrap()
        .unwrap();
    updated.key_path = Some("/keys/new_ed25519".into());
    SshRepository::update_server(&mut conn, &updated).unwrap();

    assert!(matches!(
        delete_node_and_secrets(&mut conn, &store, &expected),
        Err(CredentialOperationError::ImpactChanged { node_id }) if node_id == node.id
    ));
    assert_eq!(store.mutation_count(), 0);
}

#[test]
fn changed_secret_presence_requires_fresh_confirmation() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let node = SshRepository::create_server(&mut conn, None, "server", &server()).unwrap();
    let expected = delete_expectation(&mut conn, &store, &node.id);
    store
        .set(&node.id, SecretKind::Password, "new-password")
        .unwrap();
    let mutations_before = store.mutation_count();

    assert!(matches!(
        delete_node_and_secrets(&mut conn, &store, &expected),
        Err(CredentialOperationError::ImpactChanged { node_id }) if node_id == node.id
    ));
    assert_eq!(store.mutation_count(), mutations_before);
    assert!(store.get(&node.id, SecretKind::Password).unwrap().is_some());
}

#[test]
fn deleting_key_host_keeps_identity_file_on_disk() {
    let mut conn = setup_in_memory();
    let store = FailingSecretStore::default();
    let directory = tempfile::tempdir().unwrap();
    let identity_file = directory.path().join("id_ed25519");
    std::fs::write(&identity_file, "private key placeholder").unwrap();
    let mut info = server();
    info.auth_type = AuthType::Key;
    info.key_path = Some(identity_file.to_string_lossy().into_owned());
    let node = SshRepository::create_server(&mut conn, None, "server", &info).unwrap();
    let expected = delete_expectation(&mut conn, &store, &node.id);

    delete_node_and_secrets(&mut conn, &store, &expected).unwrap();

    assert!(identity_file.exists());
}

#[test]
fn transaction_finalization_failure_restores_deleted_secrets() {
    let store = FailingSecretStore::default();
    store
        .set("server", SecretKind::Password, "password")
        .unwrap();
    let deleted = snapshot_secrets(&store, &["server".to_string()])
        .unwrap()
        .into_iter()
        .filter(|snapshot| snapshot.value.is_some())
        .collect::<Vec<_>>();
    store.delete("server", SecretKind::Password).unwrap();

    let error = finish_delete_transaction(
        &store,
        &deleted,
        Err(CredentialOperationError::Repository(
            SshRepositoryError::NotFound("injected transaction commit failure".to_string()),
        )),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CredentialOperationError::Compensation {
            failure,
            compensation_failures,
            ..
        } if failure.contains("injected transaction commit failure")
            && compensation_failures.is_empty()
    ));
    assert_eq!(
        store
            .get("server", SecretKind::Password)
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("password")
    );
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

    let expected = delete_expectation(&mut conn, &store, &node.id);
    delete_node_and_secrets(&mut conn, &store, &expected).unwrap();
    delete_node_and_secrets(&mut conn, &store, &expected).unwrap();

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

    let expected = delete_expectation(&mut conn, &store, &node.id);
    assert!(delete_node_and_secrets(&mut conn, &store, &expected).is_err());

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
