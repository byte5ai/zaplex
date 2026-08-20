use std::collections::HashMap;
use std::path::PathBuf;

use zaplex_cockpit::{
    Account, AccountStatus, AccountUsage, CockpitSnapshot, Provider, ScanHealth, UsageProvenance,
    WindowTotals,
};

use super::*;

fn account(key: &str, config_dir: &str) -> Account {
    Account {
        provider: Provider::Claude,
        key: key.to_string(),
        config_dir: PathBuf::from(config_dir),
        label: "Account".to_string(),
        email: Some("agent@example.com".to_string()),
        org: None,
        role: None,
        plan_tier: None,
        is_default: false,
    }
}

fn route(provider: &str, account_id: &str) -> AgentLaunchRoute {
    AgentLaunchRoute {
        schema_version: ACCOUNT_ROUTING_SCHEMA_VERSION,
        provider: provider.to_string(),
        account_id: account_id.to_string(),
    }
}

fn cache(routes: AccountRoutes) -> AccountRouteCache {
    let mut cache = AccountRouteCache::default();
    cache.replace(routes);
    cache
}

fn usage(account: Account) -> AccountUsage {
    AccountUsage {
        account,
        block5h: WindowTotals::default(),
        today: WindowTotals::default(),
        today_by_session: Default::default(),
        week: WindowTotals::default(),
        reset5h: None,
        reset_week: None,
        heat: 0.25,
        heat_week: 0.5,
        heat_opus: None,
        heat_sonnet: None,
        sessions: Vec::new(),
        idle_sessions: Vec::new(),
        status: AccountStatus::Offline,
        provenance: UsageProvenance::Estimate,
    }
}

#[test]
fn stable_account_id_is_opaque_and_does_not_embed_a_config_path() {
    let mut left = account("claude:work", "/secret/first/.claude-work");
    left.email = Some(" Agent@Example.COM ".to_string());
    left.org = Some("Example Org".to_string());
    let mut right = account("claude:renamed", "/different/host/.claude-renamed");
    right.email = Some("agent@example.com".to_string());
    right.org = Some("example org".to_string());

    let id = stable_account_id(&left);
    assert_eq!(id, stable_account_id(&right));
    assert_eq!(id.len(), 32);
    assert!(!id.contains("secret"));
    assert!(!id.contains("claude-work"));
}

#[test]
fn colliding_account_identities_are_omitted_and_degrade_inventory() {
    let first = account("claude:first", "/daemon/.claude-first");
    let second = account("claude:second", "/daemon/.claude-second");
    let scan = inventory_from_snapshot(CockpitSnapshot {
        accounts: vec![usage(first), usage(second)],
        generated_at: chrono::Utc::now(),
        health: ScanHealth::Loaded,
    });

    assert!(scan.inventory.accounts.is_empty());
    assert!(scan.routes.is_empty());
    assert_eq!(scan.inventory.health, "degraded");
}

#[test]
fn selected_account_is_resolved_to_the_daemon_canonical_path() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("account");
    std::fs::create_dir(&config_dir).unwrap();
    let canonical = std::fs::canonicalize(&config_dir).unwrap();
    let account_id = "account-id";
    let mut routes = HashMap::new();
    routes.insert(
        AccountRouteKey {
            provider: "claude".to_string(),
            account_id: account_id.to_string(),
        },
        AccountRouteTarget {
            provider: "claude".to_string(),
            config_dir: Some(canonical.clone()),
        },
    );
    let mut env = HashMap::from([
        (CLAUDE_CONFIG_DIR.to_string(), "/client/path".to_string()),
        (CODEX_HOME.to_string(), "/another/client/path".to_string()),
    ]);

    prepare_launch_environment(&cache(routes), Some(&route("claude", account_id)), &mut env)
        .unwrap();

    assert_eq!(
        env.get(CLAUDE_CONFIG_DIR).map(String::as_str),
        canonical.to_str()
    );
    assert!(!env.contains_key(CODEX_HOME));
}

#[test]
fn default_account_clears_all_client_supplied_provider_paths() {
    let account_id = "default-id";
    let mut routes = HashMap::new();
    routes.insert(
        AccountRouteKey {
            provider: "codex".to_string(),
            account_id: account_id.to_string(),
        },
        AccountRouteTarget {
            provider: "codex".to_string(),
            config_dir: None,
        },
    );
    let mut env = HashMap::from([
        (CLAUDE_CONFIG_DIR.to_string(), "/client/claude".to_string()),
        (CODEX_HOME.to_string(), "/client/codex".to_string()),
    ]);

    prepare_launch_environment(&cache(routes), Some(&route("codex", account_id)), &mut env)
        .unwrap();

    assert!(!env.contains_key(CLAUDE_CONFIG_DIR));
    assert!(!env.contains_key(CODEX_HOME));
}

#[test]
fn direct_provider_paths_and_unknown_ids_fail_closed() {
    let mut direct_env =
        HashMap::from([(CODEX_HOME.to_string(), "/local/client/path".to_string())]);
    assert!(
        prepare_launch_environment(&AccountRouteCache::default(), None, &mut direct_env).is_err()
    );

    let mut empty_env = HashMap::new();
    assert!(prepare_launch_environment(
        &cache(HashMap::new()),
        Some(&route("claude", "unknown")),
        &mut empty_env,
    )
    .is_err());
}

#[test]
fn stale_non_default_route_fails_closed() {
    let missing = PathBuf::from("/definitely/missing/zaplex-account");
    let account_id = "stale-id";
    let routes = HashMap::from([(
        AccountRouteKey {
            provider: "claude".to_string(),
            account_id: account_id.to_string(),
        },
        AccountRouteTarget {
            provider: "claude".to_string(),
            config_dir: Some(missing),
        },
    )]);

    assert!(prepare_launch_environment(
        &cache(routes),
        Some(&route("claude", account_id)),
        &mut HashMap::new(),
    )
    .is_err());
}

#[test]
fn route_use_before_inventory_scan_fails_closed_but_plain_default_start_works() {
    let empty_cache = AccountRouteCache::default();
    let mut env = HashMap::from([("TERM".to_string(), "xterm-256color".to_string())]);

    prepare_launch_environment(&empty_cache, None, &mut env).unwrap();
    assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
    assert!(prepare_launch_environment(
        &empty_cache,
        Some(&route("claude", "not-yet-loaded")),
        &mut env,
    )
    .is_err());
}

#[test]
fn expired_inventory_cache_fails_closed() {
    let account_id = "expired-id";
    let routes = HashMap::from([(
        AccountRouteKey {
            provider: "claude".to_string(),
            account_id: account_id.to_string(),
        },
        AccountRouteTarget {
            provider: "claude".to_string(),
            config_dir: None,
        },
    )]);
    let mut cache = cache(routes);
    cache.refreshed_at = Some(Instant::now() - ACCOUNT_ROUTE_CACHE_TTL - Duration::from_secs(1));

    assert!(prepare_launch_environment(
        &cache,
        Some(&route("claude", account_id)),
        &mut HashMap::new(),
    )
    .is_err());
}

#[test]
fn provider_mismatch_fails_closed() {
    let account_id = "provider-id";
    let routes = HashMap::from([(
        AccountRouteKey {
            provider: "claude".to_string(),
            account_id: account_id.to_string(),
        },
        AccountRouteTarget {
            provider: "claude".to_string(),
            config_dir: None,
        },
    )]);

    assert!(prepare_launch_environment(
        &cache(routes),
        Some(&route("codex", account_id)),
        &mut HashMap::new(),
    )
    .is_err());
}

#[test]
fn pinned_session_maps_to_the_same_opaque_route_as_inventory() {
    let pinned = PathBuf::from("/daemon/.claude-work");
    let routes = HashMap::from([(
        AccountRouteKey {
            provider: "claude".to_string(),
            account_id: "opaque-work".to_string(),
        },
        AccountRouteTarget {
            provider: "claude".to_string(),
            config_dir: Some(pinned),
        },
    )]);

    assert_eq!(
        session_account_id(&routes, "claude", Some("/daemon/.claude-work")).as_deref(),
        Some("opaque-work")
    );
    assert_eq!(
        session_account_id(&routes, "claude", Some("/daemon/.claude-other")),
        None
    );
}

#[test]
fn ambiguous_session_route_has_no_opaque_identity() {
    let target = AccountRouteTarget {
        provider: "codex".to_string(),
        config_dir: Some(PathBuf::from("/daemon/.codex-work")),
    };
    let routes = HashMap::from([
        (
            AccountRouteKey {
                provider: "codex".to_string(),
                account_id: "opaque-a".to_string(),
            },
            target.clone(),
        ),
        (
            AccountRouteKey {
                provider: "codex".to_string(),
                account_id: "opaque-b".to_string(),
            },
            target,
        ),
    ]);

    assert_eq!(
        session_account_id(&routes, "codex", Some("/daemon/.codex-work")),
        None
    );
}
