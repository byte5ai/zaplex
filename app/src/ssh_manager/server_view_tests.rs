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
