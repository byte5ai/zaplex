use super::{
    discovery_failure_lifecycle, legacy_ssh_candidates, remote_candidates_for_resolved_ssh,
    remote_candidates_for_ssh, same_resume_target, selected_authentication_error, AccountIdentity,
    AgentLifecycle, ExplicitRuntimeHost, HostIdentity, InstallationIdentity, ProcessLocation,
    RoutePreferences, SubscriptionAgent, SubscriptionLocationPreference,
    SubscriptionSessionRegistry, SubscriptionTarget,
};
use crate::ai::subscription_agent::{ModelCapability, SessionIdentity};
use crate::remote_server::proto::{AgentAccountInfo, AgentAccountInventory};
use crate::terminal::ssh::util::InteractiveSshCommand;
use warp_ssh_manager::{
    AuthType, ResolvedSshConnection, SecretKind, SessionResilience, SshServerInfo,
};

fn target(agent: SubscriptionAgent) -> SubscriptionTarget {
    SubscriptionTarget {
        installation: InstallationIdentity {
            agent,
            host: HostIdentity {
                id: "local".to_string(),
                display_name: "Local".to_string(),
            },
            account: AccountIdentity {
                id: "account".to_string(),
                display_name: "Account".to_string(),
                provider_account_id: None,
                config_dir: None,
            },
            executable: agent.display_name().into(),
            version: "1.0.0".to_string(),
        },
        working_directory: "/workspace".into(),
        model: ModelCapability {
            id: "model".to_string(),
            display_name: "Model".to_string(),
            description: None,
            resolved_model: None,
            is_default: true,
            supported_efforts: Vec::new(),
            default_effort: None,
            context_window: None,
        },
        effort: None,
    }
}

fn remote_inventory(accounts: Vec<AgentAccountInfo>) -> AgentAccountInventory {
    AgentAccountInventory {
        schema_version: 1,
        accounts,
        health: "loaded".to_string(),
        health_message: String::new(),
    }
}

fn remote_account(
    provider: &str,
    route_id: &str,
    provider_account_id: Option<&str>,
) -> AgentAccountInfo {
    AgentAccountInfo {
        provider: provider.to_string(),
        account_id: route_id.to_string(),
        display_label: format!("{provider} account"),
        email: format!("{provider}@example.test"),
        is_default: true,
        health: "loaded".to_string(),
        provider_account_id: provider_account_id.map(str::to_string),
        ..Default::default()
    }
}

fn account_identity(id: &str, provider_account_id: &str) -> AccountIdentity {
    AccountIdentity {
        id: id.to_string(),
        display_name: id.to_string(),
        provider_account_id: Some(provider_account_id.to_string()),
        config_dir: None,
    }
}

#[test]
fn authentication_error_blocks_only_the_exact_selected_account() {
    let signed_out = account_identity("shared-route", "provider-signed-out");
    let healthy = account_identity("shared-route", "provider-healthy");
    let errors = vec![(
        SubscriptionAgent::ClaudeCode,
        signed_out.clone(),
        "Not logged in · Please run /login".to_string(),
    )];

    let no_account_selected = RoutePreferences {
        agent: Some(SubscriptionAgent::ClaudeCode),
        ..Default::default()
    };
    assert_eq!(
        selected_authentication_error(&errors, &no_account_selected),
        None
    );

    let healthy_selected = RoutePreferences {
        agent: Some(SubscriptionAgent::ClaudeCode),
        account_identity: Some(healthy),
        ..Default::default()
    };
    assert_eq!(
        selected_authentication_error(&errors, &healthy_selected),
        None
    );

    let signed_out_selected = RoutePreferences {
        agent: Some(SubscriptionAgent::ClaudeCode),
        account_identity: Some(signed_out),
        ..Default::default()
    };
    assert_eq!(
        selected_authentication_error(&errors, &signed_out_selected),
        Some("Not logged in · Please run /login")
    );
}

#[test]
fn signed_out_agents_are_not_recoverable_even_with_a_session_identity() {
    for (agent, message, session) in [
        (
            SubscriptionAgent::ClaudeCode,
            "Not logged in · Please run /login",
            SessionIdentity::ClaudeCode("session-1".to_string()),
        ),
        (
            SubscriptionAgent::Codex,
            "Codex is not using a ChatGPT subscription account",
            SessionIdentity::Codex("thread-1".to_string()),
        ),
        (
            SubscriptionAgent::ClaudeCode,
            "Claude Code authenticated account does not match selected account Work",
            SessionIdentity::ClaudeCode("session-2".to_string()),
        ),
        (
            SubscriptionAgent::Codex,
            "Codex did not report an account ID for selected account Work",
            SessionIdentity::Codex("thread-2".to_string()),
        ),
    ] {
        let registry = SubscriptionSessionRegistry::default();
        registry.store("conversation".to_string(), target(agent), session);

        let lifecycle =
            discovery_failure_lifecycle(&[agent], message.to_string(), &registry, "conversation");

        assert_eq!(lifecycle, AgentLifecycle::NotSignedIn { agent });
        assert!(!lifecycle.accepts_prompt());
        assert!(!lifecycle.can_resume());
        assert!(registry.get("conversation").is_none());
    }
}

#[test]
fn legacy_ssh_fails_closed_without_remote_account_identity() {
    let error = legacy_ssh_candidates(&InteractiveSshCommand {
        host: Some("developer@ssh.example.test".to_string()),
        port: Some("2222".to_string()),
    })
    .err()
    .unwrap();

    assert_eq!(
        error.to_string(),
        "subscription agents on legacy SSH cannot verify the selected remote account; reconnect this host with the Zaplex remote daemon"
    );
}

#[test]
fn legacy_ssh_candidates_require_a_reusable_host() {
    let error = legacy_ssh_candidates(&InteractiveSshCommand::default())
        .err()
        .expect("missing SSH host must fail closed");

    assert_eq!(
        error.to_string(),
        "the active SSH session has no reusable host"
    );
}

#[test]
fn remote_onekey_key_uses_shared_credential() {
    let connection = ResolvedSshConnection {
        server: SshServerInfo {
            node_id: "host-1".to_string(),
            host: "example.test".to_string(),
            port: 22,
            username: "deploy".to_string(),
            auth_type: AuthType::Key,
            key_path: Some("/keys/deploy".to_string()),
            credential_id: Some("cred-1".to_string()),
            startup_command: None,
            notes: None,
            last_connected_at: None,
            session_resilience: SessionResilience::PersistOnly,
            ring_ceiling_mb: 0,
        },
        secret_lookup_id: "cred-1".to_string(),
        secret_kind: SecretKind::Passphrase,
    };

    let inventory = remote_inventory(vec![
        remote_account("claude", "opaque-claude", Some("claude-provider-42")),
        remote_account("codex", "opaque-codex", Some("codex-provider-42")),
    ]);
    let candidates =
        remote_candidates_for_resolved_ssh("daemon-1", "edge", &connection, &inventory).unwrap();
    for candidate in candidates {
        let ProcessLocation::Remote { ssh_argv } = candidate.location else {
            panic!("resolved OneKey host must remain remote");
        };
        assert!(ssh_argv
            .windows(2)
            .any(|args| args == ["-i", "/keys/deploy"]));
        assert_eq!(
            ssh_argv.last().map(String::as_str),
            Some("deploy@example.test")
        );
    }
}

#[test]
fn remote_candidates_keep_route_and_provider_identities_separate() {
    let inventory = remote_inventory(vec![remote_account(
        "codex",
        "daemon-opaque-route",
        Some("provider-account-42"),
    )]);

    let candidates = remote_candidates_for_ssh(
        "daemon-1",
        "edge",
        vec!["ssh".to_string(), "--".to_string(), "edge".to_string()],
        &inventory,
        Some(SubscriptionAgent::Codex),
    )
    .unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].installation.account.id, "daemon-opaque-route");
    assert_eq!(
        candidates[0]
            .installation
            .account
            .provider_account_id
            .as_deref(),
        Some("provider-account-42")
    );
    assert_eq!(candidates[0].installation.account.config_dir, None);
}

#[test]
fn old_remote_inventory_without_provider_identity_fails_closed_when_selected() {
    let inventory = remote_inventory(vec![remote_account("claude", "daemon-opaque-route", None)]);

    let error = remote_candidates_for_ssh(
        "daemon-1",
        "edge",
        vec!["ssh".to_string(), "--".to_string(), "edge".to_string()],
        &inventory,
        Some(SubscriptionAgent::ClaudeCode),
    )
    .err()
    .unwrap();

    assert_eq!(
        error.to_string(),
        "remote host cannot verify the selected Claude Code subscription account; update its Zaplex daemon or refresh that CLI login"
    );
}

#[test]
fn old_remote_inventory_without_provider_identity_requires_daemon_upgrade() {
    let inventory = remote_inventory(vec![remote_account("codex", "daemon-opaque-route", None)]);

    let error = remote_candidates_for_ssh(
        "daemon-1",
        "edge",
        vec!["ssh".to_string(), "--".to_string(), "edge".to_string()],
        &inventory,
        None,
    )
    .err()
    .unwrap();

    assert_eq!(
        error.to_string(),
        "remote host cannot verify its subscription account identity; update its Zaplex daemon or refresh the CLI login"
    );
}

#[test]
fn non_default_remote_account_is_not_launched_without_a_daemon_route() {
    let mut account = remote_account("codex", "daemon-opaque-route", Some("provider-account-42"));
    account.is_default = false;
    let inventory = remote_inventory(vec![account]);

    let error = remote_candidates_for_ssh(
        "daemon-1",
        "edge",
        vec!["ssh".to_string(), "--".to_string(), "edge".to_string()],
        &inventory,
        Some(SubscriptionAgent::Codex),
    )
    .err()
    .unwrap();

    assert_eq!(
        error.to_string(),
        "the selected Codex subscription account is unavailable on remote host edge"
    );
}

#[test]
fn resume_requires_the_same_installation_directory_model_and_effort() {
    let original = target(SubscriptionAgent::Codex);
    assert!(same_resume_target(&original, &original));

    let mut changed = original.clone();
    changed.installation.agent = SubscriptionAgent::ClaudeCode;
    assert!(!same_resume_target(&original, &changed));

    let mut changed = original.clone();
    changed.installation.host.id = "remote".to_string();
    assert!(!same_resume_target(&original, &changed));

    let mut changed = original.clone();
    changed.installation.account.id = "other-account".to_string();
    assert!(!same_resume_target(&original, &changed));

    let mut changed = original.clone();
    changed.installation.account.provider_account_id = Some("provider-account-2".to_string());
    assert!(!same_resume_target(&original, &changed));

    let mut changed = original.clone();
    changed.installation.account.config_dir = Some("/other-config".into());
    assert!(!same_resume_target(&original, &changed));

    let mut changed = original.clone();
    changed.installation.executable = "/other/codex".into();
    assert!(!same_resume_target(&original, &changed));

    let mut changed = original.clone();
    changed.installation.version = "2.0.0".to_string();
    assert!(!same_resume_target(&original, &changed));

    let mut changed = original.clone();
    changed.working_directory = "/other-workspace".into();
    assert!(!same_resume_target(&original, &changed));

    let mut changed = original.clone();
    changed.model.id = "other-model".to_string();
    assert!(!same_resume_target(&original, &changed));

    let mut changed = original.clone();
    changed.model.resolved_model = Some("concrete-model-version".to_string());
    assert!(!same_resume_target(&original, &changed));

    let mut changed = original.clone();
    changed.effort = Some("high".to_string());
    assert!(!same_resume_target(&original, &changed));
}

#[test]
fn explicit_host_route_uses_the_exact_stable_id_without_local_fallback() {
    let local = SubscriptionLocationPreference {
        host: HostIdentity {
            id: "local".to_string(),
            display_name: "Local machine".to_string(),
        },
        working_directory: "/workspace".into(),
    };
    let offline_remote = SubscriptionLocationPreference {
        host: HostIdentity {
            id: "offline-daemon-42".to_string(),
            display_name: "devhost".to_string(),
        },
        working_directory: ".".into(),
    };

    assert_eq!(
        super::explicit_runtime_host(&local),
        ExplicitRuntimeHost::Local
    );
    assert_eq!(
        super::explicit_runtime_host(&offline_remote),
        ExplicitRuntimeHost::Remote("offline-daemon-42")
    );
}
