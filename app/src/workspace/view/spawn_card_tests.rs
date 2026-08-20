use super::*;

fn host(id: &str, name: &str) -> HostOption {
    HostOption {
        id: id.to_string(),
        name: name.to_string(),
        managed_fleet_available: false,
    }
}

fn managed_host(id: &str, name: &str) -> HostOption {
    HostOption {
        managed_fleet_available: true,
        ..host(id, name)
    }
}

/// A stable id must route to the matching node — not the *first* node that
/// happens to share the display label. This is the regression the Codex
/// review flagged: clicking `+` on a later same-named host scoped the launch
/// to the wrong remote.
#[test]
fn scoped_id_resolves_past_same_named_hosts() {
    let hosts = vec![
        host("id-a", "devbox"),
        host("id-b", "devbox"), // same label, different node
    ];
    // Given the second node's id, we must land on index 1 despite the shared
    // "devbox" label that a name-only match would resolve to index 0.
    assert_eq!(
        resolve_scoped_host(&hosts, Some("id-b"), Some("devbox")),
        HostChoice::Remote(1),
    );
    assert_eq!(
        resolve_scoped_host(&hosts, Some("id-a"), Some("devbox")),
        HostChoice::Remote(0),
    );
}

/// With no id (e.g. a source that lacks a stable id), fall back to matching
/// by name — the first same-named node is the best we can do.
#[test]
fn name_fallback_when_no_id() {
    let hosts = vec![host("id-a", "alpha"), host("id-b", "beta")];
    assert_eq!(
        resolve_scoped_host(&hosts, None, Some("beta")),
        HostChoice::Remote(1),
    );
}

/// Local scoping (no id, no name) stays local — the unscoped / local case.
#[test]
fn no_scope_stays_local() {
    let hosts = vec![host("id-a", "alpha")];
    assert_eq!(resolve_scoped_host(&hosts, None, None), HostChoice::Local);
}

/// A stale/unknown id does not silently fall back to a name match (which
/// could route to the wrong host — the very bug we are fixing); it stays
/// local, the safe default.
#[test]
fn unknown_id_does_not_fall_back_to_name() {
    let hosts = vec![host("id-a", "devbox"), host("id-b", "devbox")];
    assert_eq!(
        resolve_scoped_host(&hosts, Some("id-missing"), Some("devbox")),
        HostChoice::Local,
    );
}

/// End-to-end contract at the spawn-card boundary (Codex review regression):
/// the Conductor scopes by the *daemon* `HostId`, which the workspace
/// translates to the SSH `node.id` before it reaches `resolve_scoped_host`.
///
/// * A successfully translated daemon id arrives here already as the SSH
///   node id, so it resolves to the matching remote node — past a same-named
///   sibling (a name-only match would have picked the wrong one).
/// Untranslatable daemon ids are rejected at the workspace boundary before
/// this resolver is called, so they cannot reach the name fallback.
#[test]
fn daemon_scoped_open_resolves_translated_node() {
    // `hosts` is keyed by SSH node.id; two nodes share the "devbox" label.
    let hosts = vec![host("node-1", "devbox"), host("node-2", "devbox")];

    // Translation succeeded: the workspace passes SSH node id "node-2"
    // (translated from the second host's daemon id). Must resolve to it,
    // not the first same-named node.
    assert_eq!(
        resolve_scoped_host(&hosts, Some("node-2"), Some("devbox")),
        HostChoice::Remote(1),
    );
}

fn provider(installed: bool) -> ProviderOptions {
    ProviderOptions {
        installed,
        models: [
            ("opus", "Opus"),
            ("sonnet", "Sonnet"),
            ("gpt-5-codex", "GPT Codex"),
        ]
        .into_iter()
        .map(|(id, display_name)| ModelCapability {
            id: id.to_string(),
            display_name: display_name.to_string(),
            description: None,
            resolved_model: Some(id.to_string()),
            is_default: false,
            supported_efforts: vec![crate::ai::subscription_agent::ModelEffort {
                id: "high".to_string(),
                display_name: "High".to_string(),
            }],
            default_effort: Some("high".to_string()),
            context_window: None,
        })
        .collect(),
        model_discovery: ModelDiscoveryState::Ready,
        ..Default::default()
    }
}

fn remote_provider(installed: bool, provider_name: &str) -> ProviderOptions {
    let account_id = format!("{provider_name}:work");
    remote_provider_for_node(
        installed,
        provider_name,
        "node-7",
        &[(account_id.as_str(), "work@example.com", 0.8, 0.7)],
    )
}

fn remote_provider_for_node(
    installed: bool,
    provider_name: &str,
    node_id: &str,
    accounts: &[(&str, &str, f64, f64)],
) -> ProviderOptions {
    let mut options = provider(installed);
    options.remote_accounts = accounts
        .iter()
        .map(
            |(account_id, email, capacity_5h, capacity_week)| RemoteAccountOption {
                route: remote_server::proto::AgentLaunchRoute {
                    schema_version: 1,
                    provider: provider_name.to_string(),
                    account_id: (*account_id).to_string(),
                },
                label: (*email).to_string(),
                email: Some((*email).to_string()),
                capacity_5h: *capacity_5h,
                capacity_week: *capacity_week,
                capacity_known: true,
            },
        )
        .collect();
    options.remote_account_discovery = RemoteAccountDiscoveryState::Ready {
        auto_routing_available: true,
    };
    options.remote_account_node_id = Some(node_id.to_string());
    options
}

fn remote_claude_card(
    claude: ProviderOptions,
    hosts: Vec<HostOption>,
    host: HostChoice,
    account: AccountChoice,
) -> SpawnCard {
    SpawnCard {
        cfg: SpawnCardConfig {
            claude,
            hosts,
            ..Default::default()
        },
        agent: CLIAgent::Claude,
        model: "opus".to_string(),
        effort: String::new(),
        managed_mode: ManagedLaunchMode::Ordinary,
        account,
        batch_accounts: BTreeSet::new(),
        select_all_accounts: false,
        show_accounts: false,
        host,
        project: None,
        folder_history: FolderHistory::empty(),
        folder_navigation: FolderNavigation::default(),
        folder_history_open: false,
        folder_validation: DirectoryValidationState::default(),
        history_validation: BTreeMap::new(),
        history_search_editor: None,
        bulk_launch: None,
        prompt: None,
        remote_dir_editor: None,
        chip_states: Default::default(),
        close_button: None,
    }
}

/// Codex review regression: when no supported agent is installed,
/// there must be no launchable agent — a phantom chip previously let
/// Confirm emit a `Launch` for a binary that isn't there.
#[test]
fn installed_agents_empty_when_none_installed() {
    let cfg = SpawnCardConfig {
        claude: provider(false),
        codex: provider(false),
        ..Default::default()
    };
    assert_eq!(installed_agents(&cfg), Vec::new());
}

/// Exactly one installed provider yields exactly that agent — the normal
/// single-CLI case must keep working unchanged.
#[test]
fn installed_agents_only_lists_installed_provider() {
    let claude_only = SpawnCardConfig {
        claude: provider(true),
        codex: provider(false),
        ..Default::default()
    };
    assert_eq!(installed_agents(&claude_only), vec![CLIAgent::Claude]);

    let codex_only = SpawnCardConfig {
        claude: provider(false),
        codex: provider(true),
        ..Default::default()
    };
    assert_eq!(installed_agents(&codex_only), vec![CLIAgent::Codex]);

    let antigravity_only = SpawnCardConfig {
        antigravity_installed: true,
        ..Default::default()
    };
    assert_eq!(
        installed_agents(&antigravity_only),
        vec![CLIAgent::Antigravity]
    );

    let grok_only = SpawnCardConfig {
        grok_installed: true,
        ..Default::default()
    };
    assert_eq!(installed_agents(&grok_only), vec![CLIAgent::Grok]);
}

/// Both installed: both agents are offered, in the stable Claude-then-Codex
/// order the row renders.
#[test]
fn installed_agents_lists_both_when_both_installed() {
    let cfg = SpawnCardConfig {
        claude: provider(true),
        codex: provider(true),
        ..Default::default()
    };
    assert_eq!(
        installed_agents(&cfg),
        vec![CLIAgent::Claude, CLIAgent::Codex]
    );
}

#[test]
fn default_account_identity_never_becomes_an_environment_pin() {
    let mut account = AccountOption {
        label: "default".to_string(),
        config_dir: PathBuf::from("/home/user/.claude"),
        is_default: true,
        heat_label: "10 %".to_string(),
        heat: 0.1,
        plan: None,
        provider: zaplex_cockpit::Provider::Claude,
    };
    assert_eq!(account_config_pin(&account), None);
    account.is_default = false;
    assert_eq!(
        account_config_pin(&account),
        Some(PathBuf::from("/home/user/.claude"))
    );
}

#[test]
fn antigravity_launch_uses_cli_defaults_without_provider_metadata() {
    let card = SpawnCard {
        cfg: SpawnCardConfig {
            antigravity_installed: true,
            ..Default::default()
        },
        agent: CLIAgent::Antigravity,
        model: String::new(),
        effort: String::new(),
        managed_mode: ManagedLaunchMode::Ordinary,
        account: AccountChoice::Freest,
        batch_accounts: BTreeSet::new(),
        select_all_accounts: false,
        show_accounts: false,
        host: HostChoice::Local,
        project: Some(PathBuf::from("/workspace")),
        folder_history: FolderHistory::empty(),
        folder_navigation: FolderNavigation::default(),
        folder_history_open: false,
        folder_validation: DirectoryValidationState::default(),
        history_validation: BTreeMap::new(),
        history_search_editor: None,
        bulk_launch: None,
        prompt: None,
        remote_dir_editor: None,
        chip_states: Default::default(),
        close_button: None,
    };

    match card
        .launch_payload()
        .expect("an installed Antigravity CLI must yield a launch")
    {
        SpawnCardEvent::Launch {
            agent,
            config_dir,
            model,
            effort,
            ..
        } => {
            assert_eq!(agent, CLIAgent::Antigravity);
            assert_eq!(config_dir, None);
            assert_eq!(model, None);
            assert_eq!(effort, None);
        }
        _ => panic!("Confirm must emit Launch"),
    }
}

/// Confirm emits a `Launch` that carries the current selection verbatim: the
/// chosen agent, model, effort and project cwd, and — for a local launch —
/// the resolved account config dir with no remote node. Positive counterpart
/// to the "nothing installed ⇒ no launch" guard.
#[test]
fn confirm_payload_carries_local_selection() {
    let card = SpawnCard {
        cfg: SpawnCardConfig {
            claude: provider(true),
            ..Default::default()
        },
        agent: CLIAgent::Claude,
        model: "sonnet".to_string(),
        effort: "low".to_string(),
        managed_mode: ManagedLaunchMode::Ordinary,
        account: AccountChoice::Freest,
        batch_accounts: BTreeSet::new(),
        select_all_accounts: false,
        show_accounts: false,
        host: HostChoice::Local,
        project: Some(PathBuf::from("/home/dev/projects/zaplex")),
        folder_history: FolderHistory::empty(),
        folder_navigation: FolderNavigation::default(),
        folder_history_open: false,
        folder_validation: DirectoryValidationState::default(),
        history_validation: BTreeMap::new(),
        history_search_editor: None,
        bulk_launch: None,
        prompt: None,
        // The pure tests build the card without a `ViewContext`, so there is
        // no editor view to construct — remote-dir prefill/read is exercised
        // at the Confirm site, not here.
        remote_dir_editor: None,
        chip_states: Default::default(),
        close_button: None,
    };

    match card
        .launch_payload()
        .expect("an installed agent must yield a Launch on Confirm")
    {
        SpawnCardEvent::Launch {
            agent,
            config_dir,
            cwd,
            node_id,
            model,
            effort,
            ..
        } => {
            assert_eq!(agent, CLIAgent::Claude);
            assert_eq!(model.as_deref(), Some("sonnet"));
            assert_eq!(
                effort, None,
                "Claude exposes only its real CLI-default effort"
            );
            assert_eq!(cwd, Some(PathBuf::from("/home/dev/projects/zaplex")));
            assert_eq!(node_id, None, "a local launch has no remote node");
            // Freest with no configured freest_dir means the default login.
            assert_eq!(config_dir, None);
        }
        _ => panic!("Confirm must emit Launch"),
    }
}

/// A remote-scoped launch routes to the selected host's stable node id and,
/// because account config dirs are local paths, carries no config dir.
#[test]
fn confirm_payload_routes_remote_launch_to_node_id() {
    let card = SpawnCard {
        cfg: SpawnCardConfig {
            claude: remote_provider(true, "claude"),
            hosts: vec![host("node-7", "devbox")],
            ..Default::default()
        },
        agent: CLIAgent::Claude,
        model: "opus".to_string(),
        effort: "high".to_string(),
        managed_mode: ManagedLaunchMode::Ordinary,
        account: AccountChoice::Freest,
        batch_accounts: BTreeSet::new(),
        select_all_accounts: false,
        show_accounts: false,
        host: HostChoice::Remote(0),
        project: None,
        folder_history: FolderHistory::empty(),
        folder_navigation: FolderNavigation::default(),
        folder_history_open: false,
        folder_validation: DirectoryValidationState::default(),
        history_validation: BTreeMap::new(),
        history_search_editor: None,
        bulk_launch: None,
        prompt: None,
        // The pure tests build the card without a `ViewContext`, so there is
        // no editor view to construct — remote-dir prefill/read is exercised
        // at the Confirm site, not here.
        remote_dir_editor: None,
        chip_states: Default::default(),
        close_button: None,
    };

    match card
        .launch_payload()
        .expect("an installed agent must yield a Launch on Confirm")
    {
        SpawnCardEvent::Launch {
            node_id,
            config_dir,
            agent_launch_route,
            remote_account_email,
            ..
        } => {
            assert_eq!(node_id.as_deref(), Some("node-7"));
            assert_eq!(
                config_dir, None,
                "remote launches use the host's own account"
            );
            assert_eq!(
                agent_launch_route.map(|route| route.account_id),
                Some("claude:work".to_string())
            );
            assert_eq!(remote_account_email.as_deref(), Some("work@example.com"));
        }
        _ => panic!("Confirm must emit Launch"),
    }
}

#[cfg(unix)]
#[test]
fn second_remote_claude_account_reaches_daemon_open_session_as_opaque_id() {
    let claude = remote_provider_for_node(
        true,
        "claude",
        "node-7",
        &[
            ("acct_opaque_primary", "primary@example.com", 0.9, 0.8),
            ("acct_opaque_second", "second@example.com", 0.4, 0.6),
        ],
    );
    let card = remote_claude_card(
        claude,
        vec![host("node-7", "devbox")],
        HostChoice::Remote(0),
        AccountChoice::Specific(1),
    );

    let SpawnCardEvent::Launch {
        node_id,
        config_dir,
        agent_launch_route: Some(route),
        remote_account_email,
        ..
    } = card
        .launch_payload()
        .expect("the selected remote account is ready")
    else {
        panic!("Confirm must emit a routed Launch");
    };
    assert_eq!(node_id.as_deref(), Some("node-7"));
    assert_eq!(config_dir, None);
    assert_eq!(route.provider, "claude");
    assert_eq!(route.account_id, "acct_opaque_second");
    assert_eq!(remote_account_email.as_deref(), Some("second@example.com"));

    let server = warp_ssh_manager::SshServerInfo::new_default("node-7".to_string());
    let open_params = super::super::daemon_open_session_params(&server, Some(&route), None);
    assert_eq!(open_params.agent_launch_route.as_ref(), Some(&route));
    assert!(open_params.env.is_empty());
    assert!(open_params.managed_launch.is_none());
}

#[test]
fn freest_remote_account_is_scoped_to_the_selected_host() {
    let claude = remote_provider_for_node(
        true,
        "claude",
        "node-a",
        &[
            ("node-a-primary", "a-primary@example.com", 0.99, 0.99),
            ("node-a-second", "a-second@example.com", 0.98, 0.98),
        ],
    );
    let mut card = remote_claude_card(
        claude,
        vec![host("node-a", "alpha"), host("node-b", "beta")],
        HostChoice::Remote(1),
        AccountChoice::Freest,
    );

    assert!(card.freest_remote_account().is_none());
    assert!(card.launch_payload().is_none());

    card.cfg.claude = remote_provider_for_node(
        true,
        "claude",
        "node-b",
        &[
            ("node-b-primary", "b-primary@example.com", 0.35, 0.9),
            ("node-b-second", "b-second@example.com", 0.8, 0.4),
        ],
    );
    assert_eq!(
        card.freest_remote_account()
            .map(|account| account.route.account_id.as_str()),
        Some("node-b-second")
    );
}

#[cfg(unix)]
#[test]
fn local_config_paths_never_cross_the_remote_host_boundary() {
    let local_config = PathBuf::from("/home/local/.claude-work");
    let mut claude = remote_provider_for_node(
        true,
        "claude",
        "node-7",
        &[("opaque-remote-account", "remote@example.com", 0.7, 0.7)],
    );
    claude.freest_dir = Some(local_config.clone());
    claude.accounts = vec![AccountOption {
        label: "local work".to_string(),
        config_dir: local_config.clone(),
        is_default: false,
        heat_label: "10 %".to_string(),
        heat: 0.1,
        plan: None,
        provider: zaplex_cockpit::Provider::Claude,
    }];
    let card = remote_claude_card(
        claude,
        vec![host("node-7", "devbox")],
        HostChoice::Remote(0),
        AccountChoice::Specific(0),
    );

    let SpawnCardEvent::Launch {
        config_dir,
        agent_launch_route: Some(route),
        ..
    } = card.launch_payload().expect("the remote account is ready")
    else {
        panic!("Confirm must emit a routed Launch");
    };
    assert_eq!(config_dir, None);
    assert_eq!(route.account_id, "opaque-remote-account");

    let server = warp_ssh_manager::SshServerInfo::new_default("node-7".to_string());
    let open_params = super::super::daemon_open_session_params(&server, Some(&route), None);
    assert_eq!(open_params.cwd, None);
    assert!(!open_params.env.contains_key("CLAUDE_CONFIG_DIR"));
    assert!(!open_params.env.contains_key("CODEX_HOME"));
    assert_ne!(
        open_params
            .agent_launch_route
            .as_ref()
            .map(|selected| PathBuf::from(&selected.account_id)),
        Some(local_config)
    );
}

#[test]
fn degraded_remote_inventory_never_drives_auto_routing() {
    let mut claude = remote_provider(true, "claude");
    claude.remote_account_discovery = RemoteAccountDiscoveryState::Ready {
        auto_routing_available: false,
    };
    let mut card = SpawnCard {
        cfg: SpawnCardConfig {
            claude,
            hosts: vec![host("node-7", "devbox")],
            ..Default::default()
        },
        agent: CLIAgent::Claude,
        model: "opus".to_string(),
        effort: String::new(),
        managed_mode: ManagedLaunchMode::Ordinary,
        account: AccountChoice::Freest,
        batch_accounts: BTreeSet::new(),
        select_all_accounts: false,
        show_accounts: false,
        host: HostChoice::Remote(0),
        project: None,
        folder_history: FolderHistory::empty(),
        folder_navigation: FolderNavigation::default(),
        folder_history_open: false,
        folder_validation: DirectoryValidationState::default(),
        history_validation: BTreeMap::new(),
        history_search_editor: None,
        bulk_launch: None,
        prompt: None,
        remote_dir_editor: None,
        chip_states: Default::default(),
        close_button: None,
    };

    assert!(card.launch_payload().is_none());
    card.account = AccountChoice::Specific(0);
    assert!(card.launch_payload().is_some());
}

/// Confirm is inert when nothing is installed — no phantom launch of a
/// missing binary (the guard the pane's Confirm relies on).
#[test]
fn confirm_payload_none_when_nothing_installed() {
    let card = SpawnCard {
        cfg: SpawnCardConfig {
            claude: provider(false),
            codex: provider(false),
            ..Default::default()
        },
        agent: CLIAgent::Claude,
        model: "opus".to_string(),
        effort: "high".to_string(),
        managed_mode: ManagedLaunchMode::Ordinary,
        account: AccountChoice::Freest,
        batch_accounts: BTreeSet::new(),
        select_all_accounts: false,
        show_accounts: false,
        host: HostChoice::Local,
        project: None,
        folder_history: FolderHistory::empty(),
        folder_navigation: FolderNavigation::default(),
        folder_history_open: false,
        folder_validation: DirectoryValidationState::default(),
        history_validation: BTreeMap::new(),
        history_search_editor: None,
        bulk_launch: None,
        prompt: None,
        // The pure tests build the card without a `ViewContext`, so there is
        // no editor view to construct — remote-dir prefill/read is exercised
        // at the Confirm site, not here.
        remote_dir_editor: None,
        chip_states: Default::default(),
        close_button: None,
    };
    assert!(card.launch_payload().is_none());
}

/// The remote-dir text input maps to the launch cwd: a blank field means the
/// host's home (`None`); a typed absolute path becomes the cwd, trimmed so
/// stray whitespace never produces a bogus path. A local host never uses the
/// typed field. Pure — no editor/ctx needed (the Confirm site reads the
/// editor into `raw` and delegates the mapping here).
#[test]
fn relative_remote_launch_directory_is_rejected() {
    let remote = HostChoice::Remote(0);
    assert_eq!(
        remote_cwd_from_input(remote, "srv/app"),
        Err(RemoteCwdError::RelativePath),
    );
    assert_eq!(
        remote_cwd_from_input(remote, "../x"),
        Err(RemoteCwdError::RelativePath),
    );

    let card = SpawnCard {
        cfg: SpawnCardConfig {
            codex: remote_provider(true, "codex"),
            hosts: vec![host("node-7", "devbox")],
            ..Default::default()
        },
        agent: CLIAgent::Codex,
        model: "gpt-5-codex".to_string(),
        effort: "high".to_string(),
        managed_mode: ManagedLaunchMode::Ordinary,
        account: AccountChoice::Freest,
        batch_accounts: BTreeSet::new(),
        select_all_accounts: false,
        show_accounts: false,
        host: remote,
        project: None,
        folder_history: FolderHistory::empty(),
        folder_navigation: FolderNavigation::default(),
        folder_history_open: false,
        folder_validation: DirectoryValidationState::default(),
        history_validation: BTreeMap::new(),
        history_search_editor: None,
        bulk_launch: None,
        prompt: None,
        remote_dir_editor: None,
        chip_states: Default::default(),
        close_button: None,
    };
    assert!(card
        .launch_payload_for_remote_input(Some("srv/app"))
        .is_none());
}

#[test]
fn empty_remote_launch_directory_maps_explicitly_to_home() {
    let remote = HostChoice::Remote(0);
    assert_eq!(remote_cwd_from_input(remote, ""), Ok(None));
    assert_eq!(remote_cwd_from_input(remote, "   "), Ok(None));
    assert_eq!(
        remote_cwd_from_input(remote, "  /srv/app  "),
        Ok(Some(PathBuf::from("/srv/app"))),
    );
    // A local host never uses the typed field, regardless of input.
    assert_eq!(
        remote_cwd_from_input(HostChoice::Local, "/srv/app"),
        Ok(None)
    );
}

#[test]
fn model_and_effort_options_come_from_discovered_capability() {
    let options = provider(true);
    let codex = options
        .models
        .iter()
        .find(|model| model.id == "gpt-5-codex")
        .unwrap();

    assert_eq!(codex.display_name, "GPT Codex");
    assert_eq!(
        codex
            .supported_efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect::<Vec<_>>(),
        vec!["high"]
    );
}

#[test]
fn effort_payload_matches_cli_capability() {
    let mut card = SpawnCard {
        cfg: SpawnCardConfig {
            claude: provider(true),
            codex: provider(true),
            ..Default::default()
        },
        agent: CLIAgent::Codex,
        model: "gpt-5-codex".to_string(),
        effort: "high".to_string(),
        managed_mode: ManagedLaunchMode::Ordinary,
        account: AccountChoice::Freest,
        batch_accounts: BTreeSet::new(),
        select_all_accounts: false,
        show_accounts: false,
        host: HostChoice::Local,
        project: None,
        folder_history: FolderHistory::empty(),
        folder_navigation: FolderNavigation::default(),
        folder_history_open: false,
        folder_validation: DirectoryValidationState::default(),
        history_validation: BTreeMap::new(),
        history_search_editor: None,
        bulk_launch: None,
        prompt: None,
        remote_dir_editor: None,
        chip_states: Default::default(),
        close_button: None,
    };
    let SpawnCardEvent::Launch { effort, .. } = card.launch_payload().expect("Codex is installed")
    else {
        panic!("expected launch payload");
    };
    assert_eq!(effort.as_deref(), Some("high"));

    card.agent = CLIAgent::Claude;
    card.effort = "high".to_string();
    let SpawnCardEvent::Launch { effort, .. } = card.launch_payload().expect("Claude is installed")
    else {
        panic!("expected launch payload");
    };
    assert_eq!(effort, None);
}

#[test]
fn claude_spawn_card_exposes_only_cli_default_effort() {
    let card = SpawnCard {
        cfg: SpawnCardConfig {
            claude: provider(true),
            ..Default::default()
        },
        agent: CLIAgent::Claude,
        model: "sonnet".to_string(),
        effort: "high".to_string(),
        managed_mode: ManagedLaunchMode::Ordinary,
        account: AccountChoice::Freest,
        batch_accounts: BTreeSet::new(),
        select_all_accounts: false,
        show_accounts: false,
        host: HostChoice::Local,
        project: None,
        folder_history: FolderHistory::empty(),
        folder_navigation: FolderNavigation::default(),
        folder_history_open: false,
        folder_validation: DirectoryValidationState::default(),
        history_validation: BTreeMap::new(),
        history_search_editor: None,
        bulk_launch: None,
        prompt: None,
        remote_dir_editor: None,
        chip_states: Default::default(),
        close_button: None,
    };

    let SpawnCardEvent::Launch { effort, .. } = card.launch_payload().expect("Claude is installed")
    else {
        panic!("expected launch payload");
    };
    assert_eq!(effort, None);
}

#[test]
fn absolute_remote_launch_directory_reaches_request_unchanged() {
    let card = SpawnCard {
        cfg: SpawnCardConfig {
            codex: remote_provider(true, "codex"),
            hosts: vec![host("node-7", "devbox")],
            ..Default::default()
        },
        agent: CLIAgent::Codex,
        model: "gpt-5-codex".to_string(),
        effort: "high".to_string(),
        managed_mode: ManagedLaunchMode::Ordinary,
        account: AccountChoice::Freest,
        batch_accounts: BTreeSet::new(),
        select_all_accounts: false,
        show_accounts: false,
        host: HostChoice::Remote(0),
        project: None,
        folder_history: FolderHistory::empty(),
        folder_navigation: FolderNavigation::default(),
        folder_history_open: false,
        folder_validation: DirectoryValidationState::default(),
        history_validation: BTreeMap::new(),
        history_search_editor: None,
        bulk_launch: None,
        prompt: None,
        remote_dir_editor: None,
        chip_states: Default::default(),
        close_button: None,
    };

    let remote_path = "/srv/projects/a path;$(not-shell-source)";
    let SpawnCardEvent::Launch { cwd, .. } = card
        .launch_payload_for_remote_input(Some(remote_path))
        .expect("absolute remote cwd is launchable")
    else {
        panic!("expected launch payload");
    };
    assert_eq!(cwd, Some(PathBuf::from(remote_path)));
}

#[test]
fn managed_launch_is_explicit_exact_and_does_not_claim_unsupported_settings() {
    let card = SpawnCard {
        cfg: SpawnCardConfig {
            claude: remote_provider(true, "claude"),
            hosts: vec![managed_host("node-7", "devbox")],
            ..Default::default()
        },
        agent: CLIAgent::Claude,
        model: "opus".to_string(),
        effort: "high".to_string(),
        managed_mode: ManagedLaunchMode::ClaudeRemoteControl,
        account: AccountChoice::Freest,
        batch_accounts: BTreeSet::new(),
        select_all_accounts: false,
        show_accounts: false,
        host: HostChoice::Remote(0),
        project: None,
        folder_history: FolderHistory::empty(),
        folder_navigation: FolderNavigation::default(),
        folder_history_open: false,
        folder_validation: DirectoryValidationState::default(),
        history_validation: BTreeMap::new(),
        history_search_editor: None,
        bulk_launch: None,
        prompt: Some("must not be sent".to_string()),
        remote_dir_editor: None,
        chip_states: Default::default(),
        close_button: None,
    };

    let SpawnCardEvent::Launch {
        cwd,
        model,
        effort,
        prompt,
        managed_mode,
        managed_launch_id,
        ..
    } = card
        .launch_payload_for_remote_input(Some("/srv/zaplex"))
        .expect("negotiated remote host is managed-launchable")
    else {
        panic!("expected launch payload");
    };
    assert_eq!(cwd, Some(PathBuf::from("/srv/zaplex")));
    assert_eq!(managed_mode, ManagedLaunchMode::ClaudeRemoteControl);
    assert!(managed_launch_id.is_some_and(|id| !id.is_empty()));
    assert_eq!(model, None);
    assert_eq!(effort, None);
    assert_eq!(prompt, None);
}
