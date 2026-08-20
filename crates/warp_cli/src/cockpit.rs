//! Versioned, privacy-safe Cockpit snapshot output for scripts and diagnostics.

#[cfg(not(target_family = "wasm"))]
use std::{
    collections::HashSet,
    env,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use crate::control::{ControlAuth, ControlFailure, ControlFailureCode};
#[cfg(not(target_family = "wasm"))]
use anyhow::Context as _;
use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
#[cfg(not(target_family = "wasm"))]
use zaplex_cockpit::{
    Account, AccountStatus, AccountUsage, AgentInventoryStatus, CockpitSnapshot, FleetTree,
    HostAvailability, PricingTable, Provider, ScanHealth, SessionSnapshot, SessionState,
    UsageProvenance, WindowTotals, DEFAULT_BUDGET_5H, DEFAULT_BUDGET_WEEK,
};

pub const COCKPIT_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const COCKPIT_SNAPSHOT_PROTOCOL_VERSION: u32 = 1;
pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_HARD_ERROR: i32 = 1;
pub const EXIT_PARTIAL: i32 = 3;

#[cfg(not(target_family = "wasm"))]
const REMOTE_UNAVAILABLE_DETAIL: &str =
    "Connected remote hosts are available only from a running Zaplex surface";

#[derive(Debug, Clone, Subcommand)]
pub enum CockpitCommand {
    /// Print a versioned, privacy-safe Cockpit snapshot.
    Snapshot(CockpitSnapshotArgs),
}

#[derive(Debug, Clone, Args)]
pub struct CockpitSnapshotArgs {
    /// Emit the snapshot as JSON.
    #[arg(long, required = true)]
    pub json: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotStatus {
    Loaded,
    Degraded,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    Loaded,
    Degraded,
    Error,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotSource {
    pub status: SourceStatus,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotSources {
    pub local: SnapshotSource,
    pub remote_hosts: SnapshotSource,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CockpitSnapshotDocument {
    pub schema_version: u32,
    pub generated_at: DateTime<Utc>,
    pub status: SnapshotStatus,
    pub sources: SnapshotSources,
    pub accounts: Vec<AccountDocument>,
    pub hosts: Vec<HostDocument>,
    pub attention: Vec<AttentionDocument>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountDocument {
    pub id: String,
    pub host_id: String,
    pub provider: String,
    pub label: String,
    pub email: Option<String>,
    pub organization: Option<String>,
    pub role: Option<String>,
    pub plan: Option<String>,
    pub is_default: bool,
    pub health: String,
    pub status: String,
    pub usage_provenance: String,
    pub usage: Option<UsageDocument>,
    pub sessions: Vec<SessionDocument>,
}

/// One fresh, capability-gated account inventory from an exact connected daemon.
/// This is projection input only; raw daemon account ids are hashed before JSON output.
#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteAccountInventorySnapshot {
    pub host_id: String,
    pub schema_version: u32,
    pub status: RemoteAccountInventoryStatus,
    pub accounts: Vec<RemoteAccountSnapshot>,
}

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteAccountInventoryStatus {
    Loaded,
    Degraded,
    Unsupported,
    Unavailable,
    Invalid,
}

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteAccountSnapshot {
    pub provider: String,
    pub account_id: String,
    pub display_label: String,
    pub email: String,
    pub organization: String,
    pub plan_tier: String,
    pub is_default: bool,
    pub capacity_5h: f64,
    pub capacity_week: f64,
    pub capacity_known: bool,
    pub health: String,
    pub usage_provenance: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageDocument {
    pub block_5h: UsageWindowDocument,
    pub today: UsageWindowDocument,
    pub week: UsageWindowDocument,
    pub reset_5h: Option<DateTime<Utc>>,
    pub reset_week: Option<DateTime<Utc>>,
    pub heat_5h: f64,
    pub heat_week: f64,
    pub heat_opus: Option<f64>,
    pub heat_sonnet: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageWindowDocument {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_create_tokens: u64,
    pub cache_read_tokens: u64,
    pub reasoning_tokens: u64,
    pub work_tokens: u64,
    pub total_tokens: u64,
    pub messages: u64,
    pub estimated_cost_usd: Option<f64>,
    pub has_unpriced_usage: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDocument {
    pub id: String,
    pub account_id: String,
    pub host_id: String,
    pub provider: String,
    pub lifecycle: String,
    pub state: String,
    pub name: Option<String>,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub worktree: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub context_tokens: Option<u64>,
    pub last_activity: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostDocument {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub state: String,
    pub session_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionDocument {
    pub host_id: String,
    pub account_id: String,
    pub session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CockpitSnapshotRequest {
    pub version: u32,
    pub auth: ControlAuth,
}

impl CockpitSnapshotRequest {
    pub fn new(auth: ControlAuth) -> Result<Self> {
        auth.validate()?;
        Ok(Self {
            version: COCKPIT_SNAPSHOT_PROTOCOL_VERSION,
            auth,
        })
    }

    pub fn validate(&self) -> Result<()> {
        self.auth.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CockpitSnapshotResponse {
    pub result: std::result::Result<CockpitSnapshotDocument, ControlFailure>,
}

impl CockpitSnapshotResponse {
    pub fn success(document: CockpitSnapshotDocument) -> Self {
        Self {
            result: Ok(document),
        }
    }

    pub fn failure(code: ControlFailureCode, message: impl Into<String>) -> Self {
        Self {
            result: Err(ControlFailure {
                code,
                message: message.into(),
            }),
        }
    }
}

pub struct CockpitSnapshotService;

#[async_trait::async_trait]
impl ipc::Service for CockpitSnapshotService {
    type Request = CockpitSnapshotRequest;
    type Response = CockpitSnapshotResponse;
}

#[cfg(not(target_family = "wasm"))]
impl CockpitSnapshotDocument {
    pub fn from_local(snapshot: CockpitSnapshot) -> Self {
        let (local_status, local_detail) = local_source(&snapshot.health);
        let usage_available = matches!(&snapshot.health, ScanHealth::Loaded);
        let mut accounts = snapshot
            .accounts
            .iter()
            .map(|usage| account_document(usage, usage_available, true))
            .collect::<Vec<_>>();
        sort_accounts(&mut accounts);

        let mut session_ids = accounts
            .iter()
            .flat_map(|account| {
                account
                    .sessions
                    .iter()
                    .filter(|session| session.lifecycle == "live")
                    .map(|session| session.id.clone())
            })
            .collect::<Vec<_>>();
        session_ids.sort();

        let mut attention = accounts
            .iter()
            .flat_map(|account| {
                account
                    .sessions
                    .iter()
                    .filter(|session| session.lifecycle == "live" && session.state == "waiting")
                    .map(|session| AttentionDocument {
                        host_id: "local".to_string(),
                        account_id: account.id.clone(),
                        session_id: session.id.clone(),
                    })
            })
            .collect::<Vec<_>>();
        sort_attention(&mut attention);

        let status = if local_status == SourceStatus::Error {
            SnapshotStatus::Error
        } else {
            // A CLI-local scan cannot observe the running surface's remote fleet.
            SnapshotStatus::Degraded
        };

        Self {
            schema_version: COCKPIT_SNAPSHOT_SCHEMA_VERSION,
            generated_at: snapshot.generated_at,
            status,
            sources: SnapshotSources {
                local: SnapshotSource {
                    status: local_status,
                    detail: local_detail,
                },
                remote_hosts: SnapshotSource {
                    status: SourceStatus::Unavailable,
                    detail: Some(REMOTE_UNAVAILABLE_DETAIL.to_string()),
                },
            },
            accounts,
            hosts: vec![HostDocument {
                id: "local".to_string(),
                label: "local".to_string(),
                kind: "local".to_string(),
                state: "connected".to_string(),
                session_ids,
            }],
            attention,
        }
    }

    /// Export the loaded application model plus exact, freshly queried remote accounts.
    pub fn from_runtime(
        snapshot: &CockpitSnapshot,
        fleet: &FleetTree,
        remote_account_inventories: &[RemoteAccountInventorySnapshot],
    ) -> Self {
        let (local_status, local_detail) = local_source(&snapshot.health);
        let usage_available = matches!(&snapshot.health, ScanHealth::Loaded);
        let mut accounts = snapshot
            .accounts
            .iter()
            .map(|usage| account_document(usage, usage_available, false))
            .collect::<Vec<_>>();
        let mut hosts = Vec::new();
        let mut attention = Vec::new();
        let mut remote_accounts_degraded = false;

        for host in &fleet.hosts {
            let host_identity = host
                .host_id
                .as_deref()
                .or(host.registry_node_id.as_deref())
                .unwrap_or(&host.host);
            let host_id = host_document_id(host.is_local, host_identity);
            let mut host_session_ids = Vec::new();

            if !host.is_local {
                let mut matching_inventories = remote_account_inventories
                    .iter()
                    .filter(|inventory| inventory.host_id == host_identity);
                match matching_inventories.next() {
                    Some(inventory) if matching_inventories.next().is_none() => {
                        match remote_account_documents(inventory, &host_id) {
                            Ok(mut remote_accounts) => accounts.append(&mut remote_accounts),
                            Err(()) => remote_accounts_degraded = true,
                        }
                        remote_accounts_degraded |=
                            inventory.status != RemoteAccountInventoryStatus::Loaded;
                        remote_accounts_degraded |= inventory
                            .accounts
                            .iter()
                            .any(|account| account.health != "loaded" || !account.capacity_known);
                    }
                    Some(_) | None => remote_accounts_degraded = true,
                }
            }

            for session in host
                .projects
                .iter()
                .flat_map(|project| project.sessions.iter())
            {
                let resolved_account_id = matching_account(snapshot, session, host.is_local)
                    .map(|usage| stable_account_id(&usage.account))
                    .or_else(|| {
                        (!host.is_local)
                            .then(|| session.account_id.as_deref())
                            .flatten()
                            .map(|account_id| {
                                remote_account_document_id(
                                    host_identity,
                                    session.provider,
                                    account_id,
                                )
                            })
                            .filter(|account_id| {
                                accounts.iter().any(|account| account.id == *account_id)
                            })
                    });
                if !host.is_local && resolved_account_id.is_none() {
                    remote_accounts_degraded = true;
                }
                let account_id = resolved_account_id.unwrap_or_else(|| {
                    synthetic_account_id(
                        session,
                        &host_id,
                        (!host.is_local).then_some(host_identity),
                        &mut accounts,
                    )
                });
                let session = session_document(&account_id, &host_id, session, "live");
                if host_session_ids.iter().any(|id| id == &session.id) {
                    continue;
                }
                if let Some(account) = accounts.iter_mut().find(|account| account.id == account_id)
                {
                    if account.sessions.iter().all(|known| known.id != session.id) {
                        account.sessions.push(session.clone());
                    }
                }
                if session.state == "waiting" {
                    attention.push(AttentionDocument {
                        host_id: host_id.clone(),
                        account_id,
                        session_id: session.id.clone(),
                    });
                }
                host_session_ids.push(session.id);
            }

            host_session_ids.sort();
            hosts.push(HostDocument {
                id: host_id,
                label: host.host.clone(),
                kind: if host.is_local { "local" } else { "remote" }.to_string(),
                state: if !host.is_local && host.host_id.is_none() {
                    "unavailable"
                } else {
                    host_state(host.availability, host.inventory_status)
                }
                .to_string(),
                session_ids: host_session_ids,
            });
        }

        if hosts.iter().all(|host| host.kind != "local") {
            hosts.push(HostDocument {
                id: "local".to_string(),
                label: "local".to_string(),
                kind: "local".to_string(),
                state: "connected".to_string(),
                session_ids: Vec::new(),
            });
        }

        for account in &mut accounts {
            account
                .sessions
                .sort_by(|left, right| left.id.cmp(&right.id));
            account.status = document_account_status(&account.sessions).to_string();
        }
        sort_accounts(&mut accounts);
        hosts.sort_by(|left, right| {
            (left.kind != "local", &left.id).cmp(&(right.kind != "local", &right.id))
        });
        sort_attention(&mut attention);

        remote_accounts_degraded |= remote_account_inventories.iter().any(|inventory| {
            fleet.hosts.iter().all(|host| {
                host.is_local
                    || host.host_id.as_deref().or(host.registry_node_id.as_deref())
                        != Some(inventory.host_id.as_str())
            })
        });
        let remote_degraded = remote_accounts_degraded
            || fleet.hosts.iter().any(|host| {
                !host.is_local
                    && (host.host_id.is_none()
                        || host.availability != HostAvailability::Available
                        || host.inventory_status != AgentInventoryStatus::Ready)
            });
        let remote_status = if remote_degraded {
            SourceStatus::Degraded
        } else {
            SourceStatus::Loaded
        };
        let status =
            if local_status == SourceStatus::Loaded && remote_status == SourceStatus::Loaded {
                SnapshotStatus::Loaded
            } else {
                SnapshotStatus::Degraded
            };

        Self {
            schema_version: COCKPIT_SNAPSHOT_SCHEMA_VERSION,
            generated_at: snapshot.generated_at,
            status,
            sources: SnapshotSources {
                local: SnapshotSource {
                    status: local_status,
                    detail: local_detail,
                },
                remote_hosts: SnapshotSource {
                    status: remote_status,
                    detail: remote_degraded.then(|| {
                        "One or more connected hosts have incomplete agent or account inventory"
                            .to_string()
                    }),
                },
            },
            accounts,
            hosts,
            attention,
        }
    }

    fn hard_error(now: DateTime<Utc>, detail: String) -> Self {
        Self {
            schema_version: COCKPIT_SNAPSHOT_SCHEMA_VERSION,
            generated_at: now,
            status: SnapshotStatus::Error,
            sources: SnapshotSources {
                local: SnapshotSource {
                    status: SourceStatus::Error,
                    detail: Some(detail),
                },
                remote_hosts: SnapshotSource {
                    status: SourceStatus::Unavailable,
                    detail: Some(REMOTE_UNAVAILABLE_DETAIL.to_string()),
                },
            },
            accounts: Vec::new(),
            hosts: Vec::new(),
            attention: Vec::new(),
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self.status {
            SnapshotStatus::Loaded => EXIT_SUCCESS,
            SnapshotStatus::Degraded => EXIT_PARTIAL,
            SnapshotStatus::Error => EXIT_HARD_ERROR,
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn local_source(health: &ScanHealth) -> (SourceStatus, Option<String>) {
    match health {
        ScanHealth::Pending => (
            SourceStatus::Degraded,
            Some("Local Cockpit scan has not completed".to_string()),
        ),
        ScanHealth::Loaded => (SourceStatus::Loaded, None),
        ScanHealth::Degraded(detail) => (SourceStatus::Degraded, Some(detail.clone())),
    }
}

#[cfg(not(target_family = "wasm"))]
/// Executes a fresh local filesystem scan.
///
/// The standalone fallback cannot observe the running application's connected
/// daemon inventory, so it reports that source as unavailable and returns
/// [`EXIT_PARTIAL`]. A Zaplex-managed terminal uses the authenticated runtime
/// export through [`run_with_document`] instead.
pub fn run(command: CockpitCommand) -> Result<i32> {
    match command {
        CockpitCommand::Snapshot(args) => run_snapshot(args),
    }
}

#[cfg(target_family = "wasm")]
pub fn run(_command: CockpitCommand) -> Result<i32> {
    anyhow::bail!("Cockpit snapshots are unavailable on WebAssembly")
}

#[cfg(not(target_family = "wasm"))]
fn run_snapshot(args: CockpitSnapshotArgs) -> Result<i32> {
    debug_assert!(args.json, "clap requires --json");
    let now = Utc::now();
    let document = match dirs::home_dir() {
        Some(home) => CockpitSnapshotDocument::from_local(build_local_snapshot(&home, now)),
        None => CockpitSnapshotDocument::hard_error(
            now,
            "The current user's home directory could not be resolved".to_string(),
        ),
    };
    write_document(args, &document)
}

#[cfg(not(target_family = "wasm"))]
pub fn run_with_document(
    command: CockpitCommand,
    document: CockpitSnapshotDocument,
) -> Result<i32> {
    match command {
        CockpitCommand::Snapshot(args) => write_document(args, &document),
    }
}

#[cfg(not(target_family = "wasm"))]
fn write_document(args: CockpitSnapshotArgs, document: &CockpitSnapshotDocument) -> Result<i32> {
    debug_assert!(args.json, "clap requires --json");
    let exit_code = document.exit_code();
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, document)
        .context("failed to serialize Cockpit snapshot")?;
    writeln!(stdout).context("failed to write Cockpit snapshot")?;
    stdout.flush().context("failed to flush Cockpit snapshot")?;
    Ok(exit_code)
}

#[cfg(not(target_family = "wasm"))]
fn build_local_snapshot(home: &Path, now: DateTime<Utc>) -> CockpitSnapshot {
    let codex_home = env::var_os("CODEX_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let claude_config_dir = env::var("CLAUDE_CONFIG_DIR").ok();
    zaplex_cockpit::build_snapshot(
        home,
        &codex_home,
        claude_config_dir.as_deref(),
        now,
        DEFAULT_BUDGET_5H,
        DEFAULT_BUDGET_WEEK,
        &PricingTable::default(),
    )
}

#[cfg(not(target_family = "wasm"))]
fn account_document(
    usage: &AccountUsage,
    usage_available: bool,
    include_live: bool,
) -> AccountDocument {
    let account_id = stable_account_id(&usage.account);
    let live = include_live
        .then_some(usage.sessions.iter())
        .into_iter()
        .flatten()
        .map(|session| session_document(&account_id, "local", session, "live"));
    let mut sessions = live
        .chain(
            usage
                .idle_sessions
                .iter()
                .map(|session| session_document(&account_id, "local", session, "dormant")),
        )
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| left.id.cmp(&right.id));

    AccountDocument {
        id: account_id,
        host_id: "local".to_string(),
        provider: provider_name(usage.account.provider).to_string(),
        label: usage.account.label.clone(),
        email: usage.account.email.clone(),
        organization: usage.account.org.clone(),
        role: usage.account.role.clone(),
        plan: usage.account.plan_tier.clone(),
        is_default: usage.account.is_default,
        health: if usage_available {
            "loaded"
        } else {
            "degraded"
        }
        .to_string(),
        status: account_status_name(usage.status).to_string(),
        usage_provenance: if usage_available {
            provenance_name(usage.provenance).to_string()
        } else {
            "unknown".to_string()
        },
        usage: usage_available.then(|| UsageDocument {
            block_5h: window_document(usage.block5h),
            today: window_document(usage.today),
            week: window_document(usage.week),
            reset_5h: usage.reset5h,
            reset_week: usage.reset_week,
            heat_5h: usage.heat,
            heat_week: usage.heat_week,
            heat_opus: usage.heat_opus,
            heat_sonnet: usage.heat_sonnet,
        }),
        sessions,
    }
}

#[cfg(not(target_family = "wasm"))]
fn remote_account_documents(
    inventory: &RemoteAccountInventorySnapshot,
    host_id: &str,
) -> std::result::Result<Vec<AccountDocument>, ()> {
    const MAX_ACCOUNTS_PER_HOST: usize = 256;
    const MAX_ACCOUNT_ID_BYTES: usize = 256;
    const MAX_DISPLAY_BYTES: usize = 512;

    if inventory.schema_version != 1
        || !matches!(
            inventory.status,
            RemoteAccountInventoryStatus::Loaded | RemoteAccountInventoryStatus::Degraded
        )
        || inventory.accounts.len() > MAX_ACCOUNTS_PER_HOST
    {
        return Err(());
    }

    let mut identities = HashSet::new();
    let mut accounts = Vec::with_capacity(inventory.accounts.len());
    for account in &inventory.accounts {
        let provider = remote_provider(&account.provider).ok_or(())?;
        if !valid_remote_value(&account.account_id, MAX_ACCOUNT_ID_BYTES, false)
            || !valid_remote_value(&account.display_label, MAX_DISPLAY_BYTES, true)
            || !valid_remote_value(&account.email, MAX_DISPLAY_BYTES, true)
            || !valid_remote_value(&account.organization, MAX_DISPLAY_BYTES, true)
            || !valid_remote_value(&account.plan_tier, MAX_DISPLAY_BYTES, true)
            || !matches!(account.health.as_str(), "loaded" | "degraded")
            || !matches!(account.usage_provenance.as_str(), "real" | "estimate")
            || (account.capacity_known
                && (!valid_capacity(account.capacity_5h) || !valid_capacity(account.capacity_week)))
            || !identities.insert((provider, account.account_id.as_str()))
        {
            return Err(());
        }

        accounts.push(AccountDocument {
            id: remote_account_document_id(&inventory.host_id, provider, &account.account_id),
            host_id: host_id.to_string(),
            provider: provider_name(provider).to_string(),
            label: if account.display_label.is_empty() {
                format!("Remote {}", provider_display_name(provider))
            } else {
                account.display_label.clone()
            },
            email: nonempty(&account.email),
            organization: nonempty(&account.organization),
            role: None,
            plan: nonempty(&account.plan_tier),
            is_default: account.is_default,
            health: account.health.clone(),
            status: "offline".to_string(),
            usage_provenance: if account.capacity_known && account.health == "loaded" {
                account.usage_provenance.clone()
            } else {
                "unknown".to_string()
            },
            usage: None,
            sessions: Vec::new(),
        });
    }
    Ok(accounts)
}

#[cfg(not(target_family = "wasm"))]
fn remote_provider(value: &str) -> Option<Provider> {
    match value {
        "claude" => Some(Provider::Claude),
        "codex" => Some(Provider::Codex),
        _ => None,
    }
}

#[cfg(not(target_family = "wasm"))]
fn valid_remote_value(value: &str, max_bytes: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
}

#[cfg(not(target_family = "wasm"))]
fn valid_capacity(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

#[cfg(not(target_family = "wasm"))]
fn remote_account_document_id(host_id: &str, provider: Provider, account_id: &str) -> String {
    opaque_id(
        "remote-account",
        &[host_id, provider_name(provider), account_id],
    )
}

#[cfg(not(target_family = "wasm"))]
fn matching_account<'a>(
    snapshot: &'a CockpitSnapshot,
    session: &SessionSnapshot,
    is_local: bool,
) -> Option<&'a AccountUsage> {
    let provider_accounts = || {
        snapshot
            .accounts
            .iter()
            .filter(|usage| usage.account.provider == session.provider)
    };

    if is_local {
        let mut by_config = provider_accounts().filter(|usage| match &session.config_dir {
            Some(config_dir) => usage.account.config_dir.as_path() == Path::new(config_dir),
            None => usage.account.is_default,
        });
        let match_ = by_config.next();
        if match_.is_some() && by_config.next().is_none() {
            return match_;
        }
    } else {
        return None;
    }

    let email = session.account_email.as_deref()?;
    let mut by_email =
        provider_accounts().filter(|usage| usage.account.email.as_deref() == Some(email));
    let match_ = by_email.next();
    (match_.is_some() && by_email.next().is_none())
        .then_some(match_)
        .flatten()
}

#[cfg(not(target_family = "wasm"))]
fn synthetic_account_id(
    session: &SessionSnapshot,
    host_id: &str,
    remote_host_identity: Option<&str>,
    accounts: &mut Vec<AccountDocument>,
) -> String {
    let provider = provider_name(session.provider);
    let (identity_kind, identity) = match (
        session.account_id.as_deref(),
        session.config_dir.as_deref(),
        session.account_email.as_deref(),
    ) {
        (Some(account_id), _, _) => ("account-id", account_id),
        (None, Some(config_dir), _) => ("config", config_dir),
        (None, None, Some(email)) => ("email", email),
        (None, None, None) => ("session", session.session_id.as_str()),
    };
    let id = match (remote_host_identity, session.account_id.as_deref()) {
        (Some(host_identity), Some(account_id)) => {
            remote_account_document_id(host_identity, session.provider, account_id)
        }
        (None, _) | (Some(_), None) => opaque_id(
            "remote-account",
            &[provider, host_id, identity_kind, identity],
        ),
    };
    if let Some(account) = accounts.iter_mut().find(|account| account.id == id) {
        if session.state == SessionState::Active {
            account.status = "working".to_string();
        }
        return id;
    }

    accounts.push(AccountDocument {
        id: id.clone(),
        host_id: host_id.to_string(),
        provider: provider.to_string(),
        label: session
            .account_email
            .clone()
            .unwrap_or_else(|| format!("Remote {}", provider_display_name(session.provider))),
        email: session.account_email.clone(),
        organization: None,
        role: None,
        plan: None,
        is_default: false,
        health: "degraded".to_string(),
        status: if session.state == SessionState::Active {
            "working"
        } else {
            "live"
        }
        .to_string(),
        usage_provenance: "unknown".to_string(),
        usage: None,
        sessions: Vec::new(),
    });
    id
}

#[cfg(not(target_family = "wasm"))]
fn stable_account_id(account: &Account) -> String {
    opaque_id_bytes(
        "account",
        &[
            provider_name(account.provider).as_bytes(),
            account.config_dir.as_os_str().as_encoded_bytes(),
        ],
    )
}

#[cfg(not(target_family = "wasm"))]
fn opaque_id(namespace: &str, parts: &[&str]) -> String {
    let parts = parts.iter().map(|part| part.as_bytes()).collect::<Vec<_>>();
    opaque_id_bytes(namespace, &parts)
}

#[cfg(not(target_family = "wasm"))]
fn opaque_id_bytes(namespace: &str, parts: &[&[u8]]) -> String {
    // Hash the full coordinate into the wire id without serializing private paths.
    const FNV_OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const FNV_PRIME: u128 = 0x0000000001000000000000000000013b;

    let mut hash = FNV_OFFSET;
    for byte in namespace
        .as_bytes()
        .iter()
        .copied()
        .chain(std::iter::once(0xff))
        .chain(
            parts
                .iter()
                .flat_map(|part| part.iter().copied().chain(std::iter::once(0xff))),
        )
    {
        hash ^= u128::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{namespace}:{hash:032x}")
}

#[cfg(not(target_family = "wasm"))]
fn host_document_id(is_local: bool, host_identity: &str) -> String {
    if is_local {
        "local".to_string()
    } else {
        opaque_id("host", &[host_identity])
    }
}

#[cfg(not(target_family = "wasm"))]
fn host_state(
    availability: HostAvailability,
    inventory_status: AgentInventoryStatus,
) -> &'static str {
    match (availability, inventory_status) {
        (HostAvailability::Removed, _) => "removed",
        (HostAvailability::Available, AgentInventoryStatus::Ready) => "connected",
        (HostAvailability::Available, AgentInventoryStatus::Unsupported) => "unsupported",
        (HostAvailability::Available, AgentInventoryStatus::Unavailable) => "unavailable",
    }
}

#[cfg(not(target_family = "wasm"))]
fn sort_accounts(accounts: &mut [AccountDocument]) {
    accounts.sort_by(|left, right| (&left.provider, &left.id).cmp(&(&right.provider, &right.id)));
}

#[cfg(not(target_family = "wasm"))]
fn document_account_status(sessions: &[SessionDocument]) -> &'static str {
    if sessions
        .iter()
        .any(|session| session.lifecycle == "live" && session.state == "active")
    {
        "working"
    } else if sessions
        .iter()
        .any(|session| session.lifecycle == "live" && session.state != "idle")
    {
        "live"
    } else {
        "offline"
    }
}

#[cfg(not(target_family = "wasm"))]
fn sort_attention(attention: &mut [AttentionDocument]) {
    attention.sort_by(|left, right| {
        (&left.host_id, &left.account_id, &left.session_id).cmp(&(
            &right.host_id,
            &right.account_id,
            &right.session_id,
        ))
    });
}

#[cfg(not(target_family = "wasm"))]
fn session_document(
    account_id: &str,
    host_id: &str,
    session: &SessionSnapshot,
    lifecycle: &'static str,
) -> SessionDocument {
    let provider = provider_name(session.provider);
    let id = opaque_id(
        "session",
        &[host_id, account_id, provider, &session.session_id],
    );
    SessionDocument {
        id,
        account_id: account_id.to_string(),
        host_id: host_id.to_string(),
        provider: provider.to_string(),
        lifecycle: lifecycle.to_string(),
        state: session_state_name(session.state).to_string(),
        name: nonempty(&session.name),
        project: nonempty(&session.project_name),
        branch: session.branch.clone(),
        worktree: session.worktree.clone(),
        model: nonempty(&session.model),
        effort: session.effort.clone().filter(|effort| !effort.is_empty()),
        context_tokens: (session.ctx_tokens > 0).then_some(session.ctx_tokens),
        last_activity: session.last_activity,
    }
}

#[cfg(not(target_family = "wasm"))]
fn window_document(window: WindowTotals) -> UsageWindowDocument {
    UsageWindowDocument {
        input_tokens: window.input,
        output_tokens: window.output,
        cache_create_tokens: window.cache_create,
        cache_read_tokens: window.cache_read,
        reasoning_tokens: window.reasoning,
        work_tokens: window.work,
        total_tokens: window.total,
        messages: window.messages,
        estimated_cost_usd: (!window.has_unpriced_usage).then_some(window.cost_usd),
        has_unpriced_usage: window.has_unpriced_usage,
    }
}

#[cfg(not(target_family = "wasm"))]
fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(not(target_family = "wasm"))]
fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "claude",
        Provider::Codex => "codex",
        Provider::Antigravity => "antigravity",
    }
}

#[cfg(not(target_family = "wasm"))]
fn provider_display_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
        Provider::Antigravity => "Antigravity",
    }
}

#[cfg(not(target_family = "wasm"))]
fn session_state_name(state: SessionState) -> &'static str {
    match state {
        SessionState::Active => "active",
        SessionState::Waiting => "waiting",
        SessionState::Monitor => "monitor",
        SessionState::Idle => "idle",
    }
}

#[cfg(not(target_family = "wasm"))]
fn account_status_name(status: AccountStatus) -> &'static str {
    match status {
        AccountStatus::Working => "working",
        AccountStatus::Live => "live",
        AccountStatus::Offline => "offline",
    }
}

#[cfg(not(target_family = "wasm"))]
fn provenance_name(provenance: UsageProvenance) -> &'static str {
    match provenance {
        UsageProvenance::Real => "real",
        UsageProvenance::Estimate => "estimate",
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "cockpit_tests.rs"]
mod tests;
