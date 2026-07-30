/// Unit tests for resolve_test_password
/// author: logic
/// date: 2026/06/01
use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// In-process mock bypassing OS keychain. Supports error injection to simulate NoBackend / Keyring errors.
struct MockSecretStore {
    inner: Mutex<HashMap<String, String>>,
    get_err: Mutex<Option<SshSecretStoreError>>,
}

impl MockSecretStore {
    fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            get_err: Mutex::new(None),
        }
    }

    fn with_secret(node: &str, kind: SecretKind, value: &str) -> Self {
        let s = Self::new();
        s.set(node, kind, value).unwrap();
        s
    }

    fn inject_get_error(&self, err: SshSecretStoreError) {
        *self.get_err.lock().unwrap() = Some(err);
    }
}

fn account_key(node_id: &str, kind: SecretKind) -> String {
    let suffix = match kind {
        SecretKind::Password => "password",
        SecretKind::Passphrase => "passphrase",
        SecretKind::RootPassword => "root_password",
        SecretKind::OneKeyPassword => "onekey_password",
    };
    format!("{node_id}:{suffix}")
}

impl SshSecretStore for MockSecretStore {
    fn set(
        &self,
        node_id: &str,
        kind: SecretKind,
        secret: &str,
    ) -> Result<(), SshSecretStoreError> {
        self.inner
            .lock()
            .unwrap()
            .insert(account_key(node_id, kind), secret.to_string());
        Ok(())
    }

    fn get(
        &self,
        node_id: &str,
        kind: SecretKind,
    ) -> Result<Option<Zeroizing<String>>, SshSecretStoreError> {
        if let Some(err) = self.get_err.lock().unwrap().take() {
            return Err(err);
        }
        Ok(self
            .inner
            .lock()
            .unwrap()
            .get(&account_key(node_id, kind))
            .cloned()
            .map(Zeroizing::new))
    }

    fn delete(&self, _node_id: &str, _kind: SecretKind) -> Result<(), SshSecretStoreError> {
        unimplemented!()
    }
}

#[test]
fn auth_toggle_includes_onekey_option() {
    crate::i18n::init(Some("en"));

    let options = auth_toggle_options();
    assert_eq!(
        options,
        [AuthType::Password, AuthType::Key, AuthType::OneKey]
    );
    assert_eq!(auth_toggle_label(AuthType::OneKey), "OneKey");
    assert_eq!(
        auth_toggle_action(AuthType::OneKey),
        SshServerAction::SetAuthOneKey
    );
}

#[test]
fn onekey_auth_only_renders_credential_field_in_server_form() {
    assert_eq!(
        auth_specific_fields(AuthType::OneKey),
        vec![AuthSpecificField::OneKeyCredential]
    );
}

#[test]
fn empty_editor_empty_store_returns_none() {
    let store = MockSecretStore::new();
    assert!(resolve_test_password(Some("n1"), SecretKind::Password, "", &store).is_none());
}

#[test]
fn empty_editor_stored_returns_secret() {
    let store = MockSecretStore::with_secret("n1", SecretKind::Password, "from-keychain");
    let pw = resolve_test_password(Some("n1"), SecretKind::Password, "", &store).unwrap();
    assert_eq!(&*pw, "from-keychain");
}

#[test]
fn filled_editor_ignores_keychain() {
    // Keychain has old password, form typed new password → must use the form's new password;
    // otherwise after user changes host, test would be polluted by old password.
    let store = MockSecretStore::with_secret("n1", SecretKind::Password, "old-pw");
    let pw = resolve_test_password(Some("n1"), SecretKind::Password, "new-pw", &store).unwrap();
    assert_eq!(&*pw, "new-pw");
}

#[test]
fn empty_editor_no_backend_returns_none() {
    let store = MockSecretStore::new();
    store.inject_get_error(SshSecretStoreError::NoBackend);
    assert!(resolve_test_password(Some("n1"), SecretKind::Password, "", &store).is_none());
}

#[test]
fn empty_editor_keyring_error_returns_none() {
    let store = MockSecretStore::new();
    store.inject_get_error(SshSecretStoreError::Keyring("locked".into()));
    assert!(resolve_test_password(Some("n1"), SecretKind::Password, "", &store).is_none());
}

#[test]
fn onekey_lookup_uses_shared_credential_id_and_kind() {
    let store = MockSecretStore::with_secret("cred-1", SecretKind::OneKeyPassword, "shared-pw");
    let pw = resolve_test_password(Some("cred-1"), SecretKind::OneKeyPassword, "", &store).unwrap();
    assert_eq!(&*pw, "shared-pw");
}

fn credential(
    id: &str,
    username: &str,
    kind: OneKeyCredentialKind,
    key_path: Option<&str>,
) -> SshOneKeyCredential {
    let now = chrono::Utc::now().naive_utc();
    SshOneKeyCredential {
        id: id.to_string(),
        label: "shared".to_string(),
        username: username.to_string(),
        kind,
        key_path: key_path.map(ToString::to_string),
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn onekey_test_connection_uses_shared_password_credential() {
    let store = MockSecretStore::with_secret("cred-1", SecretKind::OneKeyPassword, "shared-pw");
    let credentials = vec![credential(
        "cred-1",
        "shared-user",
        OneKeyCredentialKind::Password,
        None,
    )];
    let server = SshServerInfo {
        node_id: "server-1".to_string(),
        host: "example.com".to_string(),
        port: 22,
        username: "draft-user".to_string(),
        auth_type: AuthType::OneKey,
        key_path: None,
        credential_id: Some("cred-1".to_string()),
        startup_command: None,
        notes: None,
        last_connected_at: None,
        session_resilience: warp_ssh_manager::SessionResilience::default(),
        ring_ceiling_mb: 0,
    };

    let (server, pw) = resolve_test_server_and_password(server, &credentials, "", &store).unwrap();

    assert_eq!(server.username, "shared-user");
    assert_eq!(server.auth_type, AuthType::Password);
    assert_eq!(server.key_path, None);
    assert_eq!(&*pw.unwrap(), "shared-pw");
}

#[test]
fn onekey_test_connection_prefers_editor_password() {
    let store = MockSecretStore::with_secret("cred-1", SecretKind::OneKeyPassword, "old-pw");
    let credentials = vec![credential(
        "cred-1",
        "shared-user",
        OneKeyCredentialKind::Password,
        None,
    )];
    let server = SshServerInfo {
        node_id: "server-1".to_string(),
        host: "example.com".to_string(),
        port: 22,
        username: "draft-user".to_string(),
        auth_type: AuthType::OneKey,
        key_path: None,
        credential_id: Some("cred-1".to_string()),
        startup_command: None,
        notes: None,
        last_connected_at: None,
        session_resilience: warp_ssh_manager::SessionResilience::default(),
        ring_ceiling_mb: 0,
    };

    let (_, pw) =
        resolve_test_server_and_password(server, &credentials, "typed-pw", &store).unwrap();

    assert_eq!(&*pw.unwrap(), "typed-pw");
}

#[test]
fn onekey_key_credential_resolves_test_connection_to_key_auth() {
    let store = MockSecretStore::with_secret("cred-1", SecretKind::Passphrase, "key-passphrase");
    let credentials = vec![credential(
        "cred-1",
        "key-user",
        OneKeyCredentialKind::Key,
        Some("/home/me/.ssh/id_ed25519"),
    )];
    let server = SshServerInfo {
        node_id: "server-1".to_string(),
        host: "example.com".to_string(),
        port: 22,
        username: "draft-user".to_string(),
        auth_type: AuthType::OneKey,
        key_path: None,
        credential_id: Some("cred-1".to_string()),
        startup_command: None,
        notes: None,
        last_connected_at: None,
        session_resilience: warp_ssh_manager::SessionResilience::default(),
        ring_ceiling_mb: 0,
    };

    let (server, pw) = resolve_test_server_and_password(server, &credentials, "", &store).unwrap();

    assert_eq!(server.username, "key-user");
    assert_eq!(server.auth_type, AuthType::Key);
    assert_eq!(server.key_path.as_deref(), Some("/home/me/.ssh/id_ed25519"));
    assert_eq!(&*pw.unwrap(), "key-passphrase");
}

#[test]
fn missing_lookup_id_returns_none_when_editor_empty() {
    let store = MockSecretStore::new();
    assert!(resolve_test_password(None, SecretKind::OneKeyPassword, "", &store).is_none());
}

#[test]
fn stale_test_cannot_replace_current_state() {
    assert!(should_apply_connection_test_result(7, 7));
    assert!(!should_apply_connection_test_result(8, 7));
}

#[test]
fn dirty_onekey_dialog_requires_save_discard_or_cancel() {
    assert_eq!(
        dirty_onekey_dialog_actions(),
        [
            SshServerAction::SaveManagedOneKeyCredentialAndContinue,
            SshServerAction::DiscardManagedOneKeyChanges,
            SshServerAction::CancelManagedOneKeyTransition,
        ]
    );
}

#[test]
fn dirty_dialog_requires_explicit_choice() {
    assert_eq!(dirty_onekey_dialog_actions().len(), 3);
}

#[test]
fn pending_selection_tracks_stable_credential_identity() {
    let credentials = vec![
        credential("first", "one", OneKeyCredentialKind::Password, None),
        credential("second", "two", OneKeyCredentialKind::Password, None),
    ];
    assert_eq!(
        onekey_selection_transition(Some(1), &credentials),
        OneKeyTransition::Select(Some("second".to_string()))
    );
}

#[test]
fn outside_click_does_not_discard_dirty_changes() {
    assert!(onekey_backdrop_dismiss_action().is_none());
}

#[test]
fn ssh_persistence_failure_remains_visible() {
    crate::i18n::init(Some("en"));
    let status = StatusBanner::Error("keychain is locked".to_string());
    assert_eq!(
        status_banner_content(Some(&status)),
        Some(("keychain is locked".to_string(), StatusTone::Error))
    );
}

#[test]
fn known_ssh_transport_errors_never_fall_through_to_raw_copy() {
    crate::i18n::init(Some("en"));
    let known = [
        "Connection timeout",
        "ssh: Could not resolve hostname devbox: Name or service not known",
        "ssh: connect to host devbox port 22: Connection refused",
        "Authentication failed: wrong password (Permission denied)",
        "ssh: connect to host devbox port 22: No route to host",
        "SSH host key changed; connection blocked",
        "Failed to spawn ssh: executable not found",
    ];

    for raw in known {
        assert_ne!(
            classify_ssh_transport_error(raw),
            SshTransportErrorKind::Other,
            "{raw}"
        );
        let humanized = humanize_ssh_transport_error(Some(raw));
        assert_ne!(humanized, raw);
        assert!(
            humanized.ends_with(raw),
            "diagnostic detail must remain available: {humanized}"
        );
    }
}
