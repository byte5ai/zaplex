//! Secret-free daemon-local agent-account inventory and launch routing.
//!
//! The wire exposes opaque account ids and capacity metadata only. Provider
//! config directories remain in this daemon process and are resolved only when
//! an `OpenSession` request selects an id from the latest inventory. The
//! historical `AgentSessionInfo.config_dir` field remains decode-compatible for
//! peers that did not negotiate account routing.

use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use sha2::{Digest as _, Sha256};
use zaplex_cockpit::{
    Account, CockpitSnapshot, Provider, ScanHealth, SessionSnapshot, TranscriptScanCache,
    UsageProvenance, DEFAULT_BUDGET_5H, DEFAULT_BUDGET_WEEK,
};

use super::proto::{AgentAccountInfo, AgentAccountInventory, AgentLaunchRoute};

pub(crate) const ACCOUNT_ROUTING_SCHEMA_VERSION: u32 = 1;
const ACCOUNT_ROUTE_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const CLAUDE_CONFIG_DIR: &str = "CLAUDE_CONFIG_DIR";
const CODEX_HOME: &str = "CODEX_HOME";

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AccountRouteKey {
    provider: String,
    account_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccountRouteTarget {
    provider: String,
    /// `None` is the provider's default account and deliberately clears any
    /// client-supplied provider config path.
    config_dir: Option<PathBuf>,
}

pub(crate) type AccountRoutes = HashMap<AccountRouteKey, AccountRouteTarget>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountRouteIdentity {
    DefaultAccount,
    UnixDirectory { device: u64, inode: u64 },
}

#[derive(Debug, Default)]
pub(crate) struct AccountRouteCache {
    routes: AccountRoutes,
    refreshed_at: Option<Instant>,
}

impl AccountRouteCache {
    pub(crate) fn replace(&mut self, routes: AccountRoutes) {
        self.routes = routes;
        self.refreshed_at = Some(Instant::now());
    }

    fn current_routes(&self) -> Result<&AccountRoutes, String> {
        match self.refreshed_at {
            Some(refreshed_at) if refreshed_at.elapsed() <= ACCOUNT_ROUTE_CACHE_TTL => {
                Ok(&self.routes)
            }
            Some(_) => {
                Err("daemon account inventory is stale; refresh it before launch".to_string())
            }
            None => Err("daemon account inventory has not been loaded".to_string()),
        }
    }

    #[cfg(test)]
    pub(crate) fn replace_for_test(
        &mut self,
        provider: &str,
        account_id: &str,
        config_dir: Option<PathBuf>,
    ) {
        self.replace(HashMap::from([(
            AccountRouteKey {
                provider: provider.to_string(),
                account_id: account_id.to_string(),
            },
            AccountRouteTarget {
                provider: provider.to_string(),
                config_dir,
            },
        )]));
    }

    #[cfg(test)]
    pub(crate) fn routes_for_test(&self) -> &AccountRoutes {
        &self.routes
    }
}

#[cfg(unix)]
fn route_target_identity(target: &AccountRouteTarget) -> Result<AccountRouteIdentity, String> {
    let Some(config_dir) = target.config_dir.as_ref() else {
        return Ok(AccountRouteIdentity::DefaultAccount);
    };
    let canonical = std::fs::canonicalize(config_dir)
        .map_err(|_| "selected daemon account is no longer available".to_string())?;
    if canonical != *config_dir || !canonical.is_dir() {
        return Err("selected daemon account route changed".to_string());
    }
    let metadata = std::fs::symlink_metadata(&canonical)
        .map_err(|_| "selected daemon account route changed".to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("selected daemon account route changed".to_string());
    }
    Ok(AccountRouteIdentity::UnixDirectory {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(unix)]
pub(crate) fn current_account_route_identity(
    cache: &AccountRouteCache,
    provider: &str,
    account_id: &str,
) -> Result<AccountRouteIdentity, String> {
    fresh_account_route_identity(cache.current_routes()?, provider, account_id)
}

#[cfg(unix)]
pub(crate) fn fresh_account_route_identity(
    routes: &AccountRoutes,
    provider: &str,
    account_id: &str,
) -> Result<AccountRouteIdentity, String> {
    let key = AccountRouteKey {
        provider: provider.to_string(),
        account_id: account_id.to_string(),
    };
    let target = routes
        .get(&key)
        .ok_or_else(|| "unknown or ambiguous daemon account id".to_string())?;
    if target.provider != provider {
        return Err("agent account provider mismatch".to_string());
    }
    route_target_identity(target)
}

pub(crate) fn session_account_id(
    routes: &AccountRoutes,
    provider: &str,
    config_dir: Option<&str>,
) -> Option<String> {
    let config_dir = config_dir.map(PathBuf::from);
    let mut matches = routes.iter().filter(|(key, target)| {
        key.provider == provider && target.provider == provider && target.config_dir == config_dir
    });
    let (key, _) = matches.next()?;
    matches.next().is_none().then(|| key.account_id.clone())
}

pub(crate) struct AccountInventoryScan {
    pub(crate) inventory: AgentAccountInventory,
    pub(crate) routes: AccountRoutes,
    pub(crate) sessions: Vec<SessionSnapshot>,
}

fn provider_name(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Claude => Some("claude"),
        Provider::Codex => Some("codex"),
        Provider::Antigravity => None,
    }
}

/// Derive an opaque, deterministic id from the provider and normalized account
/// identity. Email + organization survive config-root renames and are preferred;
/// the provider-owned stable key is only the fallback for accounts without an
/// identity claim. Collisions remain fail-closed when the inventory is built.
fn stable_account_id(account: &Account) -> String {
    let email = account
        .email
        .as_deref()
        .map(str::trim)
        .filter(|email| !email.is_empty())
        .map(str::to_ascii_lowercase);
    let organization = account
        .org
        .as_deref()
        .map(str::trim)
        .filter(|organization| !organization.is_empty())
        .map(str::to_ascii_lowercase);
    let mut digest = Sha256::new();
    digest.update(b"zaplex-agent-account-route-v1\0");
    digest.update(account.provider.as_str().as_bytes());
    digest.update(b"\0");
    match email {
        Some(email) => {
            digest.update(b"identity\0");
            digest.update(email.as_bytes());
            digest.update(b"\0");
            if let Some(organization) = organization {
                digest.update(organization.as_bytes());
            }
        }
        None => {
            digest.update(b"key\0");
            digest.update(account.key.as_bytes());
        }
    }
    hex::encode(&digest.finalize()[..16])
}

fn remaining_capacity(heat: f64) -> f64 {
    if heat.is_finite() {
        (1.0 - heat).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn path_free_health(health: &ScanHealth, identity_collision: bool) -> (&'static str, String) {
    if identity_collision {
        return (
            "degraded",
            "agent account identity collision; ambiguous accounts were omitted".to_string(),
        );
    }
    match health {
        ScanHealth::Loaded => ("loaded", String::new()),
        ScanHealth::Pending | ScanHealth::Degraded(_) => (
            "degraded",
            "agent account discovery was incomplete on the daemon host".to_string(),
        ),
    }
}

pub(crate) fn scan_agent_accounts() -> AccountInventoryScan {
    let Some(home) = dirs::home_dir() else {
        return AccountInventoryScan {
            inventory: AgentAccountInventory {
                schema_version: ACCOUNT_ROUTING_SCHEMA_VERSION,
                accounts: Vec::new(),
                health: "degraded".to_string(),
                health_message: "daemon home directory is unavailable".to_string(),
            },
            routes: HashMap::new(),
            sessions: Vec::new(),
        };
    };

    let default_codex_home = home.join(".codex");
    let codex_home = std::env::var_os(CODEX_HOME)
        .map(PathBuf::from)
        .unwrap_or(default_codex_home);
    let claude_config_dir = std::env::var(CLAUDE_CONFIG_DIR).ok();
    let snapshot = zaplex_cockpit::build_snapshot(
        &home,
        &codex_home,
        claude_config_dir.as_deref(),
        chrono::Utc::now(),
        DEFAULT_BUDGET_5H,
        DEFAULT_BUDGET_WEEK,
        &zaplex_cockpit::PricingTable::default(),
    );
    inventory_from_snapshot(snapshot)
}

pub(crate) fn scan_agent_accounts_with_cache(
    transcript_cache: &mut TranscriptScanCache,
) -> AccountInventoryScan {
    let Some(home) = dirs::home_dir() else {
        return AccountInventoryScan {
            inventory: AgentAccountInventory {
                schema_version: ACCOUNT_ROUTING_SCHEMA_VERSION,
                accounts: Vec::new(),
                health: "degraded".to_string(),
                health_message: "daemon home directory is unavailable".to_string(),
            },
            routes: HashMap::new(),
            sessions: Vec::new(),
        };
    };
    let codex_home = std::env::var_os(CODEX_HOME)
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let claude_config_dir = std::env::var(CLAUDE_CONFIG_DIR).ok();
    let snapshot = zaplex_cockpit::build_snapshot_with_cache(
        &home,
        &codex_home,
        claude_config_dir.as_deref(),
        chrono::Utc::now(),
        DEFAULT_BUDGET_5H,
        DEFAULT_BUDGET_WEEK,
        &zaplex_cockpit::PricingTable::default(),
        transcript_cache,
    );
    inventory_from_snapshot(snapshot)
}

fn inventory_from_snapshot(snapshot: CockpitSnapshot) -> AccountInventoryScan {
    let capacity_known = snapshot.health.is_loaded();
    let account_health = if capacity_known { "loaded" } else { "degraded" };
    let mut candidates = Vec::new();
    let mut counts: HashMap<AccountRouteKey, usize> = HashMap::new();
    let mut sessions = Vec::new();

    for mut usage in snapshot.accounts {
        sessions.append(&mut usage.sessions);
        sessions.append(&mut usage.idle_sessions);
        let Some(provider) = provider_name(usage.account.provider) else {
            continue;
        };
        let account_id = stable_account_id(&usage.account);
        let key = AccountRouteKey {
            provider: provider.to_string(),
            account_id: account_id.clone(),
        };
        *counts.entry(key.clone()).or_default() += 1;
        let config_dir = usage.account.config_dir_pin().map(PathBuf::from);
        let info = AgentAccountInfo {
            provider: provider.to_string(),
            account_id,
            display_label: usage.account.label.clone(),
            email: usage.account.email.clone().unwrap_or_default(),
            organization: usage.account.org.clone().unwrap_or_default(),
            plan_tier: usage.account.plan_tier.clone().unwrap_or_default(),
            is_default: usage.account.is_default,
            capacity_5h: remaining_capacity(usage.heat),
            capacity_week: remaining_capacity(usage.heat_week),
            capacity_known,
            health: account_health.to_string(),
            usage_provenance: match usage.provenance {
                UsageProvenance::Real => "real",
                UsageProvenance::Estimate => "estimate",
            }
            .to_string(),
        };
        candidates.push((
            key,
            AccountRouteTarget {
                provider: provider.to_string(),
                config_dir,
            },
            info,
        ));
    }

    let identity_collision = counts.values().any(|count| *count != 1);
    let mut routes = HashMap::new();
    let mut accounts = Vec::new();
    for (key, target, info) in candidates {
        if counts.get(&key) == Some(&1) {
            routes.insert(key, target);
            accounts.push(info);
        }
    }
    accounts.sort_by(|left, right| {
        (&left.provider, &left.display_label, &left.account_id).cmp(&(
            &right.provider,
            &right.display_label,
            &right.account_id,
        ))
    });

    let (health, health_message) = path_free_health(&snapshot.health, identity_collision);
    AccountInventoryScan {
        inventory: AgentAccountInventory {
            schema_version: ACCOUNT_ROUTING_SCHEMA_VERSION,
            accounts,
            health: health.to_string(),
            health_message,
        },
        routes,
        sessions,
    }
}

fn reserved_provider_environment(env: &HashMap<String, String>) -> bool {
    env.contains_key(CLAUDE_CONFIG_DIR) || env.contains_key(CODEX_HOME)
}

/// Resolve one opaque launch route against the latest daemon-local inventory.
/// Unknown, ambiguous, stale, or malformed routes fail closed. The client is
/// never allowed to supply provider config paths directly.
pub(crate) fn prepare_launch_environment(
    cache: &AccountRouteCache,
    route: Option<&AgentLaunchRoute>,
    env: &mut HashMap<String, String>,
) -> Result<(), String> {
    if route.is_none() {
        if reserved_provider_environment(env) {
            return Err(
                "provider config paths are not accepted; select a daemon account id".to_string(),
            );
        }
        return Ok(());
    }
    prepare_launch_environment_from_routes(cache.current_routes()?, route, env)
}

pub(crate) fn prepare_launch_environment_from_routes(
    routes: &AccountRoutes,
    route: Option<&AgentLaunchRoute>,
    env: &mut HashMap<String, String>,
) -> Result<(), String> {
    let Some(route) = route else {
        if reserved_provider_environment(env) {
            return Err(
                "provider config paths are not accepted; select a daemon account id".to_string(),
            );
        }
        return Ok(());
    };
    if route.schema_version != ACCOUNT_ROUTING_SCHEMA_VERSION {
        return Err("unsupported agent account route version".to_string());
    }
    if !matches!(route.provider.as_str(), "claude" | "codex")
        || route.account_id.is_empty()
        || route.account_id.len() > 128
    {
        return Err("invalid agent account route".to_string());
    }

    let key = AccountRouteKey {
        provider: route.provider.clone(),
        account_id: route.account_id.clone(),
    };
    let target = routes
        .get(&key)
        .ok_or_else(|| "unknown or ambiguous daemon account id".to_string())?;
    if target.provider != route.provider {
        return Err("agent account provider mismatch".to_string());
    }

    env.remove(CLAUDE_CONFIG_DIR);
    env.remove(CODEX_HOME);
    let Some(config_dir) = target.config_dir.as_ref() else {
        return Ok(());
    };
    let canonical = std::fs::canonicalize(config_dir)
        .map_err(|_| "selected daemon account is no longer available".to_string())?;
    if canonical != *config_dir || !canonical.is_dir() {
        return Err("selected daemon account route changed".to_string());
    }
    let config_dir = canonical
        .to_str()
        .ok_or_else(|| "selected daemon account path is not UTF-8".to_string())?;
    let env_name = match route.provider.as_str() {
        "claude" => CLAUDE_CONFIG_DIR,
        "codex" => CODEX_HOME,
        _ => return Err("invalid agent account provider".to_string()),
    };
    env.insert(env_name.to_string(), config_dir.to_string());
    Ok(())
}

#[cfg(test)]
#[path = "agent_account_tests.rs"]
mod tests;
