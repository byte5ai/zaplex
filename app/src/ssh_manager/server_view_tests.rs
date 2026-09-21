/// Unit tests for resolve_test_password
/// author: logic
/// date: 2026/06/01
use super::*;
use pathfinder_geometry::vector::vec2f;
use remote_server::proto::SessionList;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{App, WindowInvalidation};

use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::view_components::dropdown::DropdownAction;

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

struct BlockingSecretStore {
    started: Arc<Barrier>,
    release: Arc<Barrier>,
    blocked_once: AtomicBool,
}

impl SshSecretStore for BlockingSecretStore {
    fn set(
        &self,
        _node_id: &str,
        _kind: SecretKind,
        _secret: &str,
    ) -> Result<(), SshSecretStoreError> {
        Ok(())
    }

    fn get(
        &self,
        _node_id: &str,
        _kind: SecretKind,
    ) -> Result<Option<Zeroizing<String>>, SshSecretStoreError> {
        if !self.blocked_once.swap(true, Ordering::SeqCst) {
            self.started.wait();
            self.release.wait();
        }
        Ok(None)
    }

    fn delete(&self, _node_id: &str, _kind: SecretKind) -> Result<(), SshSecretStoreError> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_returns_while_keychain_get_is_blocked() {
    let started = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let store: Arc<dyn SshSecretStore> = Arc::new(BlockingSecretStore {
        started: started.clone(),
        release: release.clone(),
        blocked_once: AtomicBool::new(false),
    });
    let server = SshServerInfo::new_default("server-1".to_string());

    let worker = tokio::task::spawn_blocking(move || {
        load_server_secret_presence(Some(&server), store.as_ref())
    });
    started.wait();

    assert!(!worker.is_finished());
    release.wait();
    assert_eq!(worker.await.unwrap().unwrap(), (false, false));
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
fn startup_command_is_optional_but_must_be_one_line() {
    assert_eq!(validated_startup_command("  "), Ok(None));
    assert_eq!(
        validated_startup_command("  tmux attach  "),
        Ok(Some("tmux attach".to_string()))
    );
    assert_eq!(validated_startup_command("first\nsecond"), Err(()));
    assert_eq!(validated_startup_command("first\r\nsecond"), Err(()));
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
fn selecting_onekey_dropdown_item_does_not_rebuild_dropdown_while_it_is_borrowed() {
    App::test((), |mut app| async move {
        crate::i18n::init(Some("en"));
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
        app.add_singleton_model(|_| SshTreeChangedNotifier::new());

        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut view = SshServerView::new("server-1".to_string(), ctx);
            view.node = Some(SshNode {
                id: "server-1".to_string(),
                parent_id: None,
                kind: NodeKind::Server,
                name: "server".to_string(),
                sort_order: 0,
                created_at: chrono::Utc::now().naive_utc(),
                updated_at: chrono::Utc::now().naive_utc(),
                is_collapsed: false,
            });
            view.auth_type = AuthType::OneKey;
            view.onekey_credentials = vec![credential(
                "cred-1",
                "shared-user",
                OneKeyCredentialKind::Password,
                None,
            )];
            view.rebuild_onekey_credential_dropdown(ctx);
            view
        });
        let presenter = app.presenter(window_id).unwrap();
        let mut updated = std::collections::HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        app.update(|ctx| {
            let mut presenter = presenter.borrow_mut();
            presenter.invalidate(
                WindowInvalidation {
                    updated,
                    ..Default::default()
                },
                ctx,
            );
            presenter.build_scene(vec2f(640., 480.), 1., None, ctx);
        });

        let dropdown = view.read(&app, |view, _| view.onekey_credential_dropdown.clone());
        dropdown.update(&mut app, |dropdown, ctx| {
            dropdown.handle_action(
                &DropdownAction::SelectActionAndClose(SshServerAction::SelectOneKeyCredential(
                    Some(0),
                )),
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert_eq!(
                view.selected_onekey_credential_id.as_deref(),
                Some("cred-1")
            );
        });
    });
}

#[test]
fn stale_test_completion_cannot_overwrite_current_endpoint_state() {
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

#[test]
fn ring_ceiling_presets_do_not_exceed_the_daemon_limit() {
    assert_eq!(RING_CEILING_PRESETS.last(), Some(&(256, "256 MB")));
    assert!(RING_CEILING_PRESETS
        .iter()
        .all(|(megabytes, _)| *megabytes <= 256));
}

#[test]
fn only_connection_defining_editor_fields_invalidate_runtime_diagnostics() {
    for field in [
        ServerFormField::Host,
        ServerFormField::Port,
        ServerFormField::User,
        ServerFormField::Password,
        ServerFormField::KeyPath,
    ] {
        assert!(field.defines_connection(), "{field:?}");
    }
    for field in [
        ServerFormField::Name,
        ServerFormField::OneKeyLabel,
        ServerFormField::OneKeyUser,
        ServerFormField::OneKeyKeyPath,
        ServerFormField::OneKeySecret,
        ServerFormField::RootPassword,
        ServerFormField::StartupCommand,
        ServerFormField::Notes,
    ] {
        assert!(!field.defines_connection(), "{field:?}");
    }
}

#[test]
fn connection_edit_clears_diagnostics_and_saved_refresh_target() {
    App::test((), |mut app| async move {
        crate::i18n::init(Some("en"));
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
        app.add_singleton_model(|_| SshTreeChangedNotifier::new());

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut view = SshServerView::new("server-1".to_string(), ctx);
            view.baseline_snapshot = Some(view.current_form_snapshot(ctx));
            view.runtime_diagnostics_server =
                Some(SshServerInfo::new_default("server-1".to_string()));
            view.runtime_diagnostics = vec![runtime_diagnostic_fixture()];
            let refresh_button = view.runtime_diagnostics_refresh_button.clone();
            refresh_button.update(ctx, |button, ctx| button.set_disabled(false, ctx));
            let generation = view.runtime_diagnostics_generation;

            view.handle_server_form_edit(ServerFormField::Notes, ctx);
            assert_eq!(view.runtime_diagnostics.len(), 1);
            assert!(view.runtime_diagnostics_server.is_some());
            assert_eq!(view.runtime_diagnostics_generation, generation);
            assert!(!refresh_button.as_ref(ctx).is_disabled());

            view.handle_server_form_edit(ServerFormField::Host, ctx);
            assert!(view.runtime_diagnostics.is_empty());
            assert!(view.runtime_diagnostics_server.is_none());
            assert_eq!(view.runtime_diagnostics_generation, generation + 1);
            assert!(refresh_button.as_ref(ctx).is_disabled());
            view.handle_action(&SshServerAction::RefreshRuntimeDiagnostics, ctx);
            assert_eq!(view.runtime_diagnostics_generation, generation + 1);
            assert!(view.runtime_diagnostics_error.is_none());
            view
        });
    });
}

#[test]
fn runtime_diagnostic_units_name_binary_measurements_truthfully() {
    assert_eq!(format_diagnostic_bytes(0), "0 B");
    assert_eq!(format_diagnostic_bytes(512), "512 B");
    assert_eq!(format_diagnostic_bytes(KIB_BYTES), "1.0 KiB");
    assert_eq!(format_diagnostic_bytes(64 * MIB_BYTES), "64.0 MiB");
    assert_eq!(
        format_diagnostic_bytes(3 * GIB_BYTES + GIB_BYTES / 2),
        "3.5 GiB"
    );
}

fn measured_memory(bytes: u64, provenance: &str) -> MemoryMeasurement {
    MemoryMeasurement {
        status: MemoryMeasurementStatus::Measured.into(),
        bytes: Some(bytes),
        provenance: provenance.to_string(),
        diagnostic_code: String::new(),
    }
}

fn runtime_diagnostic_fixture() -> DaemonRuntimeDiagnostics {
    let route = remote_server::transport::DaemonRuntimeRoute::new(
        "server-v1.0.28.sock".to_string(),
        "v1.0.28".to_string(),
    )
    .unwrap();
    DaemonRuntimeDiagnostics {
        runtime_filename: route.runtime_filename().to_string(),
        server_version: Some(route.server_version().to_string()),
        route: Some(route),
        observed_at: Some(Instant::now()),
        status: DaemonRuntimeDiagnosticsStatus::Available(SessionList {
            sessions: vec![SessionInfo {
                session_id: "pty-123456789".to_string(),
                ring_bytes: 64 * MIB_BYTES,
                process_memory: Some(measured_memory(384 * MIB_BYTES, "linux-proc-smaps-rollup")),
                ..Default::default()
            }],
            host_ring_cap_bytes: 256 * MIB_BYTES,
            host_available_memory: Some(measured_memory(6 * GIB_BYTES, "linux-proc-memavailable")),
            daemon_min_available_bytes: 2 * GIB_BYTES,
            collected_at_epoch_millis: 10_000,
            ..Default::default()
        }),
    }
}

#[test]
fn runtime_diagnostics_preserve_route_and_each_measured_quantity() {
    crate::i18n::init(Some("en"));
    let diagnostic = runtime_diagnostic_fixture();
    let route = diagnostic.route.as_ref().expect("historical runtime route");
    assert_eq!(route.runtime_filename(), diagnostic.runtime_filename);
    assert_eq!(
        diagnostic.server_version.as_deref(),
        Some(route.server_version())
    );

    let presentations = runtime_diagnostic_presentations(&[diagnostic], false);
    let runtime = &presentations[0];
    assert_eq!(runtime.state, DiagnosticValueState::Measured);
    assert_eq!(runtime.runtime, "server-v1.0.28.sock");
    assert_eq!(runtime.version, "v1.0.28");
    assert_eq!(runtime.rows.len(), 5);
    assert_eq!(runtime.rows[0].label, "Output-ring capacity");
    assert_eq!(runtime.rows[0].value, "256.0 MiB");
    assert_eq!(runtime.rows[1].label, "Host available memory");
    assert_eq!(runtime.rows[1].value, "6.0 GiB");
    assert_eq!(runtime.rows[2].value, "2.0 GiB");
    assert_eq!(runtime.rows[3].value, "64.0 MiB");
    assert_eq!(runtime.rows[4].value, "384.0 MiB");
    assert_eq!(runtime.rows[4].hint, "Proportional set size (PSS) · Linux");
}

#[test]
fn stale_runtime_diagnostics_retain_values_with_truthful_labels() {
    crate::i18n::init(Some("en"));
    let presentations = runtime_diagnostic_presentations(&[runtime_diagnostic_fixture()], true);
    let runtime = &presentations[0];
    assert_eq!(runtime.state, DiagnosticValueState::Stale);
    assert_eq!(runtime.rows[0].value, "256.0 MiB");
    assert_eq!(runtime.rows[1].value, "6.0 GiB");
    assert_eq!(runtime.rows[2].value, "2.0 GiB");
    assert_eq!(runtime.rows[3].value, "64.0 MiB");
    assert_eq!(runtime.rows[4].value, "384.0 MiB");
    assert!(runtime
        .rows
        .iter()
        .all(|row| row.state == DiagnosticValueState::Stale && row.hint == "Measurement stale"));
}

#[test]
fn daemon_clock_skew_does_not_change_locally_observed_freshness() {
    crate::i18n::init(Some("en"));
    let mut future_clock = runtime_diagnostic_fixture();
    let DaemonRuntimeDiagnosticsStatus::Available(snapshot) = &mut future_clock.status else {
        panic!("fixture must contain diagnostics");
    };
    snapshot.collected_at_epoch_millis = u64::MAX;

    let presentations = runtime_diagnostic_presentations(&[future_clock], false);
    assert_eq!(presentations[0].state, DiagnosticValueState::Measured);
    assert!(presentations[0]
        .rows
        .iter()
        .all(|row| row.state == DiagnosticValueState::Measured));

    let old_local_observation =
        Instant::now() - Duration::from_millis(DIAGNOSTIC_MAX_AGE_MILLIS + 1);
    assert!(runtime_diagnostics_are_stale(
        Some(old_local_observation),
        false
    ));
    assert!(!runtime_diagnostics_are_stale(Some(Instant::now()), false));
    assert!(runtime_diagnostics_are_stale(None, false));
}

#[test]
fn each_runtime_uses_its_session_list_observation_time() {
    crate::i18n::init(Some("en"));
    let mut early = runtime_diagnostic_fixture();
    early.runtime_filename = "server-v1.0.27.sock".to_string();
    early.observed_at = Some(Instant::now() - Duration::from_millis(DIAGNOSTIC_MAX_AGE_MILLIS + 1));
    let recent = runtime_diagnostic_fixture();

    let presentations = runtime_diagnostic_presentations(&[early, recent], false);

    assert_eq!(presentations[0].state, DiagnosticValueState::Stale);
    assert!(presentations[0]
        .rows
        .iter()
        .all(|row| row.state == DiagnosticValueState::Stale));
    assert_eq!(presentations[1].state, DiagnosticValueState::Measured);
    assert!(presentations[1]
        .rows
        .iter()
        .all(|row| row.state == DiagnosticValueState::Measured));
}

#[test]
fn diagnostic_zero_values_follow_field_semantics() {
    crate::i18n::init(Some("en"));
    let mut diagnostic = runtime_diagnostic_fixture();
    let DaemonRuntimeDiagnosticsStatus::Available(snapshot) = &mut diagnostic.status else {
        panic!("fixture must contain diagnostics");
    };
    snapshot.host_ring_cap_bytes = 0;
    snapshot.daemon_min_available_bytes = 0;
    snapshot.host_available_memory = Some(measured_memory(0, "linux-proc-memavailable"));
    snapshot.sessions[0].ring_bytes = 0;
    snapshot.sessions[0].process_memory = Some(measured_memory(0, "linux-proc-smaps-rollup"));

    let presentations = runtime_diagnostic_presentations(&[diagnostic], false);
    let runtime = &presentations[0];
    assert_eq!(runtime.rows[0].value, "—");
    assert_eq!(runtime.rows[0].state, DiagnosticValueState::Unavailable);
    assert_eq!(runtime.rows[1].value, "0 B");
    assert_eq!(runtime.rows[1].state, DiagnosticValueState::Measured);
    assert_eq!(runtime.rows[2].value, "Not configured");
    assert_eq!(runtime.rows[2].state, DiagnosticValueState::Measured);
    assert_eq!(runtime.rows[3].value, "0 B");
    assert_eq!(runtime.rows[3].state, DiagnosticValueState::Measured);
    assert_eq!(runtime.rows[4].value, "0 B");
    assert_eq!(runtime.rows[4].state, DiagnosticValueState::Measured);
}

#[test]
fn unavailable_and_unsupported_memory_never_leak_numeric_payloads() {
    crate::i18n::init(Some("en"));
    let unavailable = MemoryMeasurement {
        status: MemoryMeasurementStatus::Unavailable.into(),
        bytes: Some(123 * MIB_BYTES),
        provenance: "linux-proc-memavailable".to_string(),
        diagnostic_code: "read-failed".to_string(),
    };
    let unsupported = MemoryMeasurement {
        status: MemoryMeasurementStatus::Unsupported.into(),
        bytes: None,
        provenance: "unsupported-platform".to_string(),
        diagnostic_code: "unsupported-platform".to_string(),
    };

    let unavailable_row = diagnostic_memory_row(
        "Host available memory".to_string(),
        Some(&unavailable),
        "linux-proc-memavailable",
        "MemAvailable · Linux".to_string(),
        false,
    );
    let unsupported_row = diagnostic_memory_row(
        "Process memory".to_string(),
        Some(&unsupported),
        "linux-proc-smaps-rollup",
        "PSS · Linux".to_string(),
        false,
    );

    assert_eq!(unavailable_row.value, "—");
    assert_eq!(unavailable_row.state, DiagnosticValueState::Unavailable);
    assert_eq!(unsupported_row.value, "—");
    assert_eq!(unsupported_row.state, DiagnosticValueState::Unsupported);
}

#[test]
fn retained_snapshot_is_explicitly_stale_during_failed_refresh() {
    crate::i18n::init(Some("en"));
    let presentations = runtime_diagnostic_presentations(&[runtime_diagnostic_fixture()], true);
    assert_eq!(presentations[0].state, DiagnosticValueState::Stale);
    assert!(presentations[0].rows.iter().all(|row| {
        row.state == DiagnosticValueState::Stale
            && row.value != "—"
            && row.hint == "Measurement stale"
    }));
}

#[test]
fn unreachable_runtime_is_distinct_from_zero_measurements() {
    crate::i18n::init(Some("en"));
    let diagnostic = DaemonRuntimeDiagnostics {
        runtime_filename: "server-v1.0.27.sock".to_string(),
        server_version: None,
        route: None,
        observed_at: None,
        status: DaemonRuntimeDiagnosticsStatus::Unavailable,
    };
    let presentations = runtime_diagnostic_presentations(&[diagnostic], false);
    assert_eq!(presentations[0].state, DiagnosticValueState::Unavailable);
    assert!(presentations[0].rows.is_empty());
}
