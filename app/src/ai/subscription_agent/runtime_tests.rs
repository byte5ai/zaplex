use super::{
    discovery_failure_lifecycle, legacy_ssh_candidates, local_account_identity,
    local_candidates_from_inventory, remote_candidates_for_resolved_ssh, remote_candidates_for_ssh,
    route_target, same_resume_target, selected_authentication_error, AccountIdentity,
    AgentCapability, AgentLifecycle, ExplicitRuntimeHost, HostIdentity, InstallationIdentity,
    ProcessLocation, RoutePreferences, RouteResult, SubscriptionAgent,
    SubscriptionLocationPreference, SubscriptionSessionRegistry, SubscriptionTarget,
};
use crate::ai::subscription_agent::{ModelCapability, SessionIdentity};
use crate::remote_server::client::RemoteServerClient;
use crate::remote_server::proto::{AgentAccountInfo, AgentAccountInventory};
use crate::remote_server::transport::DaemonRuntimeRoute;
use crate::terminal::ssh::util::InteractiveSshCommand;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::sync::Arc;
use warp_ssh_manager::{
    AuthType, ResolvedSshConnection, SecretKind, SessionResilience, SshServerInfo,
};
use warpui::r#async::executor;
use zaplex_cockpit::{Account, AccountStatus, AccountUsage, Provider, UsageProvenance};

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

fn local_usage(provider: Provider, id: &str, heat: f64) -> AccountUsage {
    AccountUsage {
        account: Account {
            provider,
            key: id.to_string(),
            config_dir: format!("/accounts/{id}").into(),
            label: id.to_string(),
            provider_account_id: Some(format!("provider-{id}")),
            email: None,
            org: None,
            role: None,
            plan_tier: None,
            is_default: false,
        },
        block5h: Default::default(),
        today: Default::default(),
        today_by_session: Default::default(),
        week: Default::default(),
        reset5h: None,
        reset_week: None,
        heat,
        heat_week: heat,
        heat_opus: None,
        heat_sonnet: None,
        sessions: Vec::new(),
        idle_sessions: Vec::new(),
        status: AccountStatus::Live,
        provenance: UsageProvenance::Real,
    }
}

fn prepare_local_route(
    preferences: &mut RoutePreferences,
    accounts: &[AccountUsage],
    selected_account: Option<&str>,
    commands: &[&str],
) -> RouteResult {
    let candidates =
        local_candidates_from_inventory(preferences, accounts, selected_account, |command| {
            commands
                .contains(&command)
                .then(|| format!("/bin/{command}").into())
        });
    // Only the external CLI capability response is a fixture. Candidate
    // preparation and preference handling use the production runtime path.
    let capabilities = candidates.into_iter().map(|candidate| AgentCapability {
        models: vec![target(candidate.installation.agent).model],
        installation: candidate.installation,
    });
    route_target(capabilities, preferences, "/workspace".into())
}

#[test]
fn local_preparation_preserves_a_missing_preferred_agent() {
    let mut preferences = RoutePreferences {
        agent: Some(SubscriptionAgent::ClaudeCode),
        account_id: Some("missing-account".to_string()),
        model_id: Some("remembered-model".to_string()),
        effort: Some("high".to_string()),
        ..Default::default()
    };
    let original = preferences.clone();
    let accounts = [local_usage(Provider::Codex, "remaining-account", 0.0)];

    assert_eq!(
        prepare_local_route(
            &mut preferences,
            &accounts,
            Some("remaining-account"),
            &["codex"]
        ),
        RouteResult::NeedsAgentChoice(vec![SubscriptionAgent::Codex])
    );
    assert_eq!(preferences, original);
}

#[test]
fn local_preparation_preserves_a_missing_preferred_account() {
    let missing = local_usage(Provider::Claude, "missing-account", 0.9);
    let accounts = [local_usage(Provider::Claude, "remaining-account", 0.0)];
    for identity in [None, Some(local_account_identity(&missing.account))] {
        let mut preferences = RoutePreferences {
            agent: Some(SubscriptionAgent::ClaudeCode),
            account_id: Some(missing.account.key.clone()),
            account_identity: identity,
            model_id: Some("model".to_string()),
            ..Default::default()
        };
        let original = preferences.clone();

        assert_eq!(
            prepare_local_route(
                &mut preferences,
                &accounts,
                Some("remaining-account"),
                &["claude"]
            ),
            RouteResult::NeedsAccountChoice {
                agent: SubscriptionAgent::ClaudeCode,
                accounts: vec![local_account_identity(&accounts[0].account)],
            }
        );
        assert_eq!(preferences, original);
    }
}

#[test]
fn local_preparation_preserves_identity_when_an_account_route_is_reused() {
    let account = local_usage(Provider::Claude, "shared-route", 0.0);
    let mut changed_provider = account.clone();
    changed_provider.account.provider_account_id = Some("different-provider-account".to_string());
    let mut changed_directory = account.clone();
    changed_directory.account.config_dir = "/accounts/different-root".into();
    for replacement in [changed_provider, changed_directory] {
        let expected_account = local_account_identity(&replacement.account);
        let mut preferences = RoutePreferences {
            agent: Some(SubscriptionAgent::ClaudeCode),
            account_id: Some(account.account.key.clone()),
            account_identity: Some(local_account_identity(&account.account)),
            ..Default::default()
        };
        let original = preferences.clone();

        assert_eq!(
            prepare_local_route(
                &mut preferences,
                &[replacement],
                Some("shared-route"),
                &["claude"]
            ),
            RouteResult::NeedsAccountChoice {
                agent: SubscriptionAgent::ClaudeCode,
                accounts: vec![expected_account],
            }
        );
        assert_eq!(preferences, original);
    }
}

#[test]
fn local_preparation_requires_account_choice_without_quota_ranking() {
    let accounts = [
        local_usage(Provider::Claude, "busy-account", 0.95),
        local_usage(Provider::Claude, "freest-account", 0.0),
    ];
    for agent in [None, Some(SubscriptionAgent::ClaudeCode)] {
        let mut preferences = RoutePreferences {
            agent,
            ..Default::default()
        };
        let original = preferences.clone();

        assert_eq!(
            prepare_local_route(&mut preferences, &accounts, None, &["claude"]),
            RouteResult::NeedsAccountChoice {
                agent: SubscriptionAgent::ClaudeCode,
                accounts: accounts
                    .iter()
                    .map(|usage| local_account_identity(&usage.account))
                    .collect(),
            }
        );
        assert_eq!(preferences, original);
    }
}

#[test]
fn local_preparation_keeps_an_explicit_cockpit_account_identity() {
    let accounts = [
        local_usage(Provider::Claude, "selected-account", 0.95),
        local_usage(Provider::Claude, "freest-account", 0.0),
    ];
    let mut preferences = RoutePreferences::default();
    let RouteResult::Ready(target) = prepare_local_route(
        &mut preferences,
        &accounts,
        Some("selected-account"),
        &["claude"],
    ) else {
        panic!("the explicitly selected account must remain routable");
    };

    assert_eq!(
        target.installation.account,
        local_account_identity(&accounts[0].account)
    );
    assert_eq!(
        preferences.account_identity,
        Some(target.installation.account)
    );
}

#[test]
fn local_preparation_requires_agent_choice_when_the_cockpit_selection_has_no_cli() {
    let accounts = [
        local_usage(Provider::Codex, "selected-codex", 0.0),
        local_usage(Provider::Claude, "available-claude", 0.0),
    ];
    let mut preferences = RoutePreferences::default();

    assert_eq!(
        prepare_local_route(
            &mut preferences,
            &accounts,
            Some("selected-codex"),
            &["claude"]
        ),
        RouteResult::NeedsAgentChoice(vec![SubscriptionAgent::ClaudeCode])
    );
    assert_eq!(
        preferences,
        RoutePreferences {
            agent: Some(SubscriptionAgent::Codex),
            account_id: Some("selected-codex".to_string()),
            account_identity: Some(local_account_identity(&accounts[0].account)),
            ..Default::default()
        }
    );
}

#[test]
fn local_preparation_ignores_an_unknown_cockpit_account_key() {
    let accounts = [local_usage(Provider::Claude, "available-account", 0.0)];
    let mut preferences = RoutePreferences::default();
    let RouteResult::Ready(target) = prepare_local_route(
        &mut preferences,
        &accounts,
        Some("unknown-account"),
        &["claude"],
    ) else {
        panic!("an unknown Cockpit key must not invent a routing preference");
    };

    assert_eq!(
        target.installation.account,
        local_account_identity(&accounts[0].account)
    );
    assert_eq!(preferences, RoutePreferences::default());
}

#[test]
fn local_preparation_ignores_an_unsupported_cockpit_provider() {
    let accounts = [
        local_usage(Provider::Antigravity, "unsupported-account", 0.0),
        local_usage(Provider::Claude, "available-account", 0.0),
    ];
    let mut preferences = RoutePreferences::default();
    let RouteResult::Ready(target) = prepare_local_route(
        &mut preferences,
        &accounts,
        Some("unsupported-account"),
        &["claude"],
    ) else {
        panic!("an unsupported provider must not seed subscription routing");
    };

    assert_eq!(
        target.installation.account,
        local_account_identity(&accounts[1].account)
    );
    assert_eq!(preferences, RoutePreferences::default());
}

#[test]
fn local_preparation_keeps_a_stored_account_identity_without_an_agent_preference() {
    let accounts = [
        local_usage(Provider::Claude, "remembered-account", 0.0),
        local_usage(Provider::Codex, "cockpit-account", 0.0),
    ];
    let mut preferences = RoutePreferences {
        account_identity: Some(local_account_identity(&accounts[0].account)),
        ..Default::default()
    };
    let original = preferences.clone();
    let RouteResult::Ready(target) = prepare_local_route(
        &mut preferences,
        &accounts,
        Some("cockpit-account"),
        &["claude"],
    ) else {
        panic!("the stored account identity must take precedence over the Cockpit selection");
    };

    assert_eq!(
        target.installation.account,
        local_account_identity(&accounts[0].account)
    );
    assert_eq!(preferences, original);
}

#[test]
fn local_preparation_preserves_live_selection_requests() {
    let accounts = [local_usage(Provider::Claude, "selected-account", 0.0)];
    for mut preferences in [
        RoutePreferences {
            require_agent_choice: true,
            ..Default::default()
        },
        RoutePreferences {
            agent: Some(SubscriptionAgent::ClaudeCode),
            require_account_choice: true,
            ..Default::default()
        },
    ] {
        let original = preferences.clone();
        let expected = if preferences.require_agent_choice {
            RouteResult::NeedsAgentChoice(vec![SubscriptionAgent::ClaudeCode])
        } else {
            RouteResult::NeedsAccountChoice {
                agent: SubscriptionAgent::ClaudeCode,
                accounts: vec![local_account_identity(&accounts[0].account)],
            }
        };

        assert_eq!(
            prepare_local_route(
                &mut preferences,
                &accounts,
                Some("selected-account"),
                &["claude"]
            ),
            expected
        );
        assert_eq!(preferences, original);
    }
}

#[test]
fn local_preparation_routes_a_unique_unselected_account() {
    let accounts = [local_usage(Provider::Claude, "only-account", 0.95)];
    let mut preferences = RoutePreferences::default();
    let RouteResult::Ready(target) =
        prepare_local_route(&mut preferences, &accounts, None, &["claude"])
    else {
        panic!("a unique reachable target needs no additional choice");
    };

    assert_eq!(
        target.installation.account,
        local_account_identity(&accounts[0].account)
    );
    assert_eq!(preferences, RoutePreferences::default());
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
        let ProcessLocation::Remote { ssh_argv, .. } = candidate.location else {
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

#[test]
fn dispatch_with_a_changed_preflight_target_leaves_starting_and_keeps_selection() {
    futures_lite::future::block_on(async {
        let selected = target(SubscriptionAgent::Codex);
        let mut changed_provider = selected.installation.clone();
        changed_provider.account.provider_account_id = Some("new-account".to_string());
        let mut changed_config = selected.installation.clone();
        changed_config.account.config_dir = Some("/different/config".into());
        let mut changed_executable = selected.installation.clone();
        changed_executable.executable = "/different/codex".into();
        for (changed_field, replacement) in [
            ("provider", changed_provider),
            ("config", changed_config),
            ("executable", changed_executable),
        ] {
            let registry = SubscriptionSessionRegistry::default();
            registry.remember_target("conversation", &selected);
            registry.set_target("conversation", selected.clone());
            let identity = SessionIdentity::Codex("native-thread".to_string());
            registry.store(
                "conversation".to_string(),
                selected.clone(),
                identity.clone(),
            );
            registry.set_lifecycle("conversation", AgentLifecycle::Ready);
            let preferences = registry.preferences("conversation");
            let dispatch = super::SubscriptionDispatch {
                candidates: super::RuntimeCandidates::Ready(vec![super::RuntimeCandidate {
                    installation: replacement,
                    location: ProcessLocation::Local,
                }]),
                preferences: preferences.clone(),
                registry: registry.clone(),
                conversation_id: "conversation".to_string(),
                task_id: "task".to_string(),
                needs_create_task: false,
                prompt: "must not run on another target".to_string(),
                working_directory: selected.working_directory.clone(),
            };
            let (_sender, receiver) = futures::channel::oneshot::channel();
            let result = super::generate_subscription_output(dispatch, receiver).await;

            assert!(result.is_err(), "{changed_field}");
            assert!(registry.target("conversation").is_none());
            assert_eq!(registry.preferences("conversation"), preferences);
            assert_eq!(registry.get("conversation").unwrap().session, identity);
            let lifecycle = registry.lifecycle("conversation").unwrap();
            assert!(matches!(lifecycle, AgentLifecycle::RecoverableError { .. }));
            assert!(lifecycle.can_change_location());
            assert!(lifecycle.can_resume());
            assert!(!lifecycle.accepts_prompt());
        }
    });
}

fn connected_daemon(
    process_id: &str,
    registry_node: Option<&str>,
    historical: bool,
) -> super::ConnectedDaemon {
    let executor = executor::Background::default();
    let (client, _events) = RemoteServerClient::new(
        futures_lite::io::empty(),
        futures_lite::io::sink(),
        &executor,
    );
    super::ConnectedDaemon {
        host_label: "same-label".to_string(),
        host_id: process_id.to_string(),
        registry_node_id: registry_node.map(str::to_string),
        daemon_runtime: historical.then(|| {
            DaemonRuntimeRoute::new("server-v1.0.28.sock".to_string(), "1.0.28".to_string())
                .unwrap()
        }),
        client: Arc::new(client),
        features: Vec::new(),
    }
}

#[test]
fn subscription_location_survives_restart_but_excludes_historical_or_ambiguous_routes() {
    let original = connected_daemon("old-process", Some("registered-node"), false);
    let restarted = connected_daemon("new-process", Some("registered-node"), false);
    let identity = super::subscription_host_identity(&original).unwrap();
    assert_eq!(
        super::subscription_host_identity(&restarted),
        Some(identity.clone())
    );
    assert_ne!(identity.id, original.host_id);

    let historical = connected_daemon("historical-process", Some("registered-node"), true);
    let unrelated = connected_daemon("another-process", Some("another-node"), false);
    let daemons = vec![historical, restarted, unrelated];
    let selected = super::selected_remote_daemon(&identity.id, &daemons).unwrap();
    assert_eq!(selected.host_id, "new-process");
    assert!(super::selected_remote_daemon("historical-process", &daemons).is_err());
    assert!(super::selected_remote_daemon("same-label", &daemons).is_err());
    assert!(
        super::subscription_host_identity(&connected_daemon("unregistered", None, false)).is_none()
    );

    let duplicate = connected_daemon("duplicate-current", Some("registered-node"), false);
    let mut ambiguous = daemons;
    ambiguous.push(duplicate);
    assert!(super::selected_remote_daemon(&identity.id, &ambiguous).is_err());
}

#[test]
fn native_resume_after_restart_requires_the_complete_revalidated_target() {
    let registry = SubscriptionSessionRegistry::default();
    let before = connected_daemon("old-process", Some("registered-node"), false);
    let after = connected_daemon("new-process", Some("registered-node"), false);
    let mut previous = target(SubscriptionAgent::Codex);
    previous.installation.host = super::subscription_host_identity(&before).unwrap();
    let identity = SessionIdentity::Codex("native-thread".to_string());
    registry.store(
        "conversation".to_string(),
        previous.clone(),
        identity.clone(),
    );
    let mut revalidated = previous.clone();
    revalidated.installation.host = super::subscription_host_identity(&after).unwrap();
    assert_eq!(
        super::validated_resume_session(&registry, "conversation", &revalidated).unwrap(),
        Some(identity.clone())
    );

    let mut changed_version = revalidated.clone();
    changed_version.installation.version = "new-cli-version".to_string();
    let mut changed_model = revalidated.clone();
    changed_model.model.resolved_model = Some("changed-model-version".to_string());
    for changed in [changed_version, changed_model] {
        assert!(super::validated_resume_session(&registry, "conversation", &changed).is_err());
        let stored = registry.get("conversation").unwrap();
        assert_eq!(stored.target, previous);
        assert_eq!(stored.session, identity);
        let lifecycle = registry.lifecycle("conversation").unwrap();
        assert!(!lifecycle.accepts_prompt());
        assert!(lifecycle.can_resume());

        // An explicitly separate conversation may start on the new target without
        // discarding the previous conversation's native provider session.
        assert_eq!(
            super::validated_resume_session(&registry, "new-conversation", &changed).unwrap(),
            None
        );
        assert_eq!(registry.get("conversation").unwrap().session, identity);
    }
    assert_eq!(
        super::validated_resume_session(&registry, "conversation", &revalidated).unwrap(),
        Some(identity)
    );
}

#[test]
fn dispatch_revalidates_a_preflight_target_and_reports_discovery_failure() {
    futures_lite::future::block_on(async {
        let registry = SubscriptionSessionRegistry::default();
        let mut selected = target(SubscriptionAgent::Codex);
        selected.installation.executable = "/nonexistent/zaplex-test/removed-codex".into();
        registry.remember_target("conversation", &selected);
        registry.set_target("conversation", selected.clone());
        registry.set_lifecycle("conversation", AgentLifecycle::Ready);
        let identity = SessionIdentity::Codex("native-thread".to_string());
        registry.store(
            "conversation".to_string(),
            selected.clone(),
            identity.clone(),
        );
        let dispatch = super::SubscriptionDispatch {
            candidates: super::RuntimeCandidates::Ready(vec![super::RuntimeCandidate {
                installation: selected.installation,
                location: ProcessLocation::Local,
            }]),
            preferences: registry.preferences("conversation"),
            registry: registry.clone(),
            conversation_id: "conversation".to_string(),
            task_id: "task".to_string(),
            needs_create_task: false,
            prompt: "must revalidate the target".to_string(),
            working_directory: selected.working_directory,
        };
        let (_sender, receiver) = futures::channel::oneshot::channel();
        assert!(super::generate_subscription_output(dispatch, receiver)
            .await
            .is_err());
        assert!(registry.target("conversation").is_none());
        assert!(matches!(
            registry.lifecycle("conversation"),
            Some(AgentLifecycle::RecoverableError { .. })
        ));
        assert_eq!(registry.get("conversation").unwrap().session, identity);
    });
}

#[cfg(unix)]
async fn assert_dispatch_rejects_changed_session_target(
    current_version: &str,
    current_model: &str,
) {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("fake-claude");
    let invocations = directory.path().join("invocations");
    let capability = serde_json::json!({
        "type": "control_response",
        "response": {
            "request_id": "zaplex-initialize",
            "response": {
                "account": { "accountUuid": "claude-account-42" },
                "models": [{
                    "value": "model",
                    "displayName": "Model",
                    "resolvedModel": current_model,
                }],
            },
        },
    });
    let script = r#"#!/bin/sh
printf '%s\n' "$*" >> __INVOCATIONS__
case "$1" in
    --version) printf '%s\n' __VERSION__; exit 0 ;;
esac
IFS= read -r initialize
case "$initialize" in
    *zaplex-initialize*) printf '%s\n' __CAPABILITY__ ;;
    *) exit 91 ;;
esac
while IFS= read -r unexpected_prompt; do
    exit 92
done
"#
    .replace(
        "__INVOCATIONS__",
        &shell_words::quote(invocations.to_str().unwrap()),
    )
    .replace("__VERSION__", &shell_words::quote(current_version))
    .replace(
        "__CAPABILITY__",
        &shell_words::quote(&capability.to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let registry = SubscriptionSessionRegistry::default();
    let mut selected = target(SubscriptionAgent::ClaudeCode);
    selected.installation.executable = executable;
    selected.installation.account.provider_account_id = Some("claude-account-42".to_string());
    selected.working_directory = directory.path().to_path_buf();
    selected.model.resolved_model = Some("model-before".to_string());
    registry.remember_target("conversation", &selected);
    registry.set_target("conversation", selected.clone());
    registry.set_lifecycle("conversation", AgentLifecycle::Ready);
    let identity = SessionIdentity::ClaudeCode("native-session-before".to_string());
    registry.store(
        "conversation".to_string(),
        selected.clone(),
        identity.clone(),
    );
    let preferences = registry.preferences("conversation");
    let dispatch = super::SubscriptionDispatch {
        candidates: super::RuntimeCandidates::Ready(vec![super::RuntimeCandidate {
            installation: selected.installation.clone(),
            location: ProcessLocation::Local,
        }]),
        preferences: preferences.clone(),
        registry: registry.clone(),
        conversation_id: "conversation".to_string(),
        task_id: "task".to_string(),
        needs_create_task: false,
        prompt: "must not silently start another native session".to_string(),
        working_directory: selected.working_directory.clone(),
    };
    let (_sender, receiver) = futures::channel::oneshot::channel();
    assert!(super::generate_subscription_output(dispatch, receiver)
        .await
        .is_err());

    // Only version and capability discovery may run. A third invocation would
    // start or resume a provider session before the identity mismatch is resolved.
    let invocations = std::fs::read_to_string(invocations).unwrap();
    assert_eq!(invocations.lines().count(), 2, "{invocations}");
    assert_eq!(invocations.lines().next(), Some("--version"));
    assert!(registry.target("conversation").is_none());
    assert_eq!(registry.preferences("conversation"), preferences);
    let stored = registry.get("conversation").unwrap();
    assert_eq!(stored.target, selected);
    assert_eq!(stored.session, identity);
    assert_eq!(
        registry.lifecycle("conversation"),
        Some(AgentLifecycle::RecoverableError {
            message: crate::t!("ai-footer-subscription-target-changed"),
            session: Some(identity),
        })
    );
}

#[cfg(unix)]
#[test]
fn dispatch_preserves_native_session_when_the_cli_version_changes() {
    futures_lite::future::block_on(assert_dispatch_rejects_changed_session_target(
        "2.0.0",
        "model-before",
    ));
}

#[cfg(unix)]
#[test]
fn dispatch_preserves_native_session_when_the_resolved_model_changes() {
    futures_lite::future::block_on(assert_dispatch_rejects_changed_session_target(
        "1.0.0",
        "model-after",
    ));
}

#[cfg(unix)]
fn remote_probe_candidates() -> (
    tempfile::TempDir,
    super::RuntimeCandidate,
    super::RuntimeCandidate,
) {
    let directory = tempfile::tempdir().unwrap();
    let failed_ssh = directory.path().join("failed-ssh");
    let healthy_ssh = directory.path().join("healthy-ssh");
    for (executable, script) in [
        (
            &failed_ssh,
            "#!/bin/sh\nprintf '%s\\n' ZAPLEX_CLI_NOT_FOUND >&2\nexit 127\n",
        ),
        (
            &healthy_ssh,
            "#!/bin/sh\nprintf '%s\\0%s\\0' /remote/bin/agent /remote/bin:/usr/bin:/bin\n",
        ),
    ] {
        std::fs::write(executable, script).unwrap();
        let mut permissions = std::fs::metadata(executable).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(executable, permissions).unwrap();
    }
    let candidate = |agent, ssh: std::path::PathBuf, provider_account_id: &str| {
        let mut installation = target(agent).installation;
        installation.host.id = "ssh-registry:remote-node".to_string();
        installation.account.provider_account_id = Some(provider_account_id.to_string());
        super::RuntimeCandidate {
            installation,
            location: ProcessLocation::Remote {
                ssh_argv: vec![
                    ssh.to_str().unwrap().to_string(),
                    "--".to_string(),
                    "remote.test".to_string(),
                ],
                environment_path: None,
            },
        }
    };
    (
        directory,
        candidate(
            SubscriptionAgent::ClaudeCode,
            failed_ssh,
            "selected-provider",
        ),
        candidate(SubscriptionAgent::Codex, healthy_ssh, "other-provider"),
    )
}

#[cfg(unix)]
#[test]
fn dispatch_preserves_the_preferred_cli_probe_error_when_another_agent_resolves() {
    futures_lite::future::block_on(async {
        let (_directory, mut failed, healthy) = remote_probe_candidates();
        let mut selected = target(SubscriptionAgent::ClaudeCode);
        selected.installation = failed.installation.clone();
        let registry = SubscriptionSessionRegistry::default();
        registry.remember_target("conversation", &selected);
        registry.set_target("conversation", selected.clone());
        registry.set_lifecycle("conversation", AgentLifecycle::Ready);
        let identity = SessionIdentity::ClaudeCode("native-session-before".to_string());
        registry.store(
            "conversation".to_string(),
            selected.clone(),
            identity.clone(),
        );
        let preferences = registry.preferences("conversation");
        // A refreshed display label is not a different provider/config identity.
        failed.installation.account.display_name = "Updated account label".to_string();
        let dispatch = super::SubscriptionDispatch {
            candidates: super::RuntimeCandidates::Ready(vec![healthy, failed]),
            preferences: preferences.clone(),
            registry: registry.clone(),
            conversation_id: "conversation".to_string(),
            task_id: "task".to_string(),
            needs_create_task: false,
            prompt: "keep the selected CLI diagnosis".to_string(),
            working_directory: selected.working_directory.clone(),
        };
        let (_sender, receiver) = futures::channel::oneshot::channel();
        let result = super::generate_subscription_output(dispatch, receiver).await;
        assert!(result.is_err());
        let Some(AgentLifecycle::RecoverableError { message, session }) =
            registry.lifecycle("conversation")
        else {
            panic!("the selected CLI lookup failure must remain recoverable");
        };
        assert!(message.contains("Could not locate Claude Code on the remote host"));
        assert!(message.contains("The CLI is not executable on the login shell PATH"));
        assert!(!message.contains("ZAPLEX_CLI_NOT_FOUND"));
        assert_eq!(session, Some(identity.clone()));
        assert_eq!(registry.preferences("conversation"), preferences);
        assert!(registry.target("conversation").is_none());
        let stored = registry.get("conversation").unwrap();
        assert_eq!(stored.target, selected);
        assert_eq!(stored.session, identity);
    });
}

#[cfg(unix)]
#[test]
fn unselected_discovery_keeps_a_resolvable_cli_when_another_probe_fails() {
    futures_lite::future::block_on(async {
        let (_directory, failed, healthy) = remote_probe_candidates();
        let registry = SubscriptionSessionRegistry::default();
        let resolved = super::resolve_runtime_candidates(
            super::RuntimeCandidates::Ready(vec![healthy, failed]),
            &RoutePreferences::default(),
            &registry,
            "conversation",
        )
        .await
        .unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].installation.agent, SubscriptionAgent::Codex);
        assert_eq!(
            resolved[0].installation.executable,
            std::path::PathBuf::from("/remote/bin/agent")
        );
        assert_eq!(registry.lifecycle("conversation"), None);
    });
}

#[cfg(unix)]
#[test]
fn remote_probe_failure_does_not_block_a_different_selected_account_with_the_same_route_id() {
    futures_lite::future::block_on(async {
        let (_directory, failed, mut healthy) = remote_probe_candidates();
        healthy.installation.agent = SubscriptionAgent::ClaudeCode;
        assert_eq!(
            failed.installation.account.id,
            healthy.installation.account.id
        );
        let selected_account = healthy.installation.account.clone();
        let preferences = RoutePreferences {
            agent: Some(SubscriptionAgent::ClaudeCode),
            account_id: Some(selected_account.id.clone()),
            account_identity: Some(selected_account.clone()),
            ..Default::default()
        };
        let registry = SubscriptionSessionRegistry::default();
        let resolved = super::resolve_runtime_candidates(
            super::RuntimeCandidates::Ready(vec![healthy, failed]),
            &preferences,
            &registry,
            "conversation",
        )
        .await
        .unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].installation.account, selected_account);
        assert_eq!(registry.lifecycle("conversation"), None);
    });
}
