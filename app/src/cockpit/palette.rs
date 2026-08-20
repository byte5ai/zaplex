//! Stable, secret-safe Command Palette projection of the Cockpit inventory.
//!
//! Rendering and fuzzy matching live in `search::command_palette::cockpit`; this
//! module owns identity and capability semantics so every palette consumer routes
//! the same Host × Provider × Account × Session target as the Cockpit itself.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use zaplex_cockpit::{
    host_ident, host_key, session_key, AgentInventoryStatus, CockpitSnapshot, FleetTree, Provider,
    ScanHealth, SessionSnapshot, SessionState,
};

use super::github_flows::{flow_keys, label_key_for_flow, RepositoryContext};

const MAX_DORMANT_SESSIONS_PER_ACCOUNT: usize = 50;

/// Stable execution target emitted by a Cockpit Command Palette result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CockpitPaletteTarget {
    Account {
        account_key: String,
    },
    Session {
        key: String,
        host: String,
        host_id: Option<String>,
        session_id: String,
        provider: Provider,
        config_dir: Option<String>,
        account_email: Option<String>,
        account_id: Option<String>,
        is_local: bool,
    },
    Host {
        key: String,
        registry_node_id: Option<String>,
        host_id: Option<String>,
        host: String,
        is_local: bool,
    },
    Project {
        key: String,
        registry_node_id: Option<String>,
        host_id: Option<String>,
        host: String,
        is_local: bool,
        project_root: PathBuf,
    },
    GitHubFlow {
        key: String,
        flow_key: String,
        repository: RepositoryContext,
    },
}

impl CockpitPaletteTarget {
    pub fn stable_key(&self) -> &str {
        match self {
            Self::Account { account_key } => account_key,
            Self::Session { key, .. }
            | Self::Host { key, .. }
            | Self::Project { key, .. }
            | Self::GitHubFlow { key, .. } => key,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CockpitPaletteKind {
    Account,
    Session,
    Host,
    Project,
    GitHubFlow,
}

/// One safe-to-index palette row. Routing-only fields remain in `target` and
/// never enter `search_text`, labels, accessibility text, or telemetry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CockpitPaletteRecord {
    pub kind: CockpitPaletteKind,
    pub primary: String,
    pub secondary: String,
    pub search_text: String,
    pub waiting: bool,
    pub target: CockpitPaletteTarget,
}

impl CockpitPaletteRecord {
    pub fn stable_key(&self) -> &str {
        self.target.stable_key()
    }

    pub fn accessibility_label(&self) -> String {
        let kind = match self.kind {
            CockpitPaletteKind::Account => "Account",
            CockpitPaletteKind::Session => "Agent session",
            CockpitPaletteKind::Host => "Host",
            CockpitPaletteKind::Project => "Project",
            CockpitPaletteKind::GitHubFlow => "GitHub workflow",
        };
        if self.secondary.is_empty() {
            format!("{kind}: {}", self.primary)
        } else {
            format!("{kind}: {}. {}", self.primary, self.secondary)
        }
    }
}

fn title_provider(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
        Provider::Antigravity => "Antigravity",
    }
}

fn leaf_label(path: &str) -> String {
    path.rsplit(|character| character == '/' || character == '\\')
        .next()
        .map(str::to_string)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Project".to_string())
}

fn session_primary(session: &SessionSnapshot) -> String {
    if !session.name.trim().is_empty() {
        return session.name.clone();
    }
    if let Some(branch) = session
        .branch
        .as_deref()
        .filter(|branch| !branch.is_empty())
    {
        return branch.to_string();
    }
    session
        .worktree
        .as_deref()
        .filter(|worktree| !worktree.is_empty())
        .map(leaf_label)
        .unwrap_or_else(|| session.session_id.clone())
}

fn account_display_by_route(snapshot: &CockpitSnapshot) -> HashMap<(Provider, String), String> {
    let mut labels = HashMap::new();
    for usage in &snapshot.accounts {
        if let Some(email) = usage.account.email.as_deref() {
            labels.insert(
                (usage.account.provider, email.to_string()),
                usage.account.label.clone(),
            );
        }
        if let Some(config_dir) = usage.account.config_dir_pin() {
            labels.insert(
                (usage.account.provider, config_dir),
                usage.account.label.clone(),
            );
        }
    }
    labels
}

fn session_record(
    session: &SessionSnapshot,
    host: &str,
    host_id: Option<&str>,
    is_local: bool,
    account_label: Option<&str>,
) -> CockpitPaletteRecord {
    let provider = title_provider(session.provider);
    let project = if session.project_name.trim().is_empty() {
        leaf_label(&session.project_root)
    } else {
        session.project_name.clone()
    };
    let primary = session_primary(session);
    let mut searchable = vec![
        provider.to_string(),
        host.to_string(),
        project.clone(),
        primary.clone(),
        session.session_id.clone(),
        session.model.clone(),
    ];
    if let Some(label) = account_label.filter(|label| !label.is_empty()) {
        searchable.push(label.to_string());
    }
    if let Some(email) = session
        .account_email
        .as_deref()
        .filter(|email| !email.is_empty())
    {
        searchable.push(email.to_string());
    }
    if let Some(branch) = session
        .branch
        .as_deref()
        .filter(|branch| !branch.is_empty())
    {
        searchable.push(branch.to_string());
    }
    if let Some(worktree) = session
        .worktree
        .as_deref()
        .filter(|worktree| !worktree.is_empty())
    {
        searchable.push(leaf_label(worktree));
    }
    let secondary = [provider, host, project.as_str(), session.model.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let key = session_key(is_local, host_id, session);
    CockpitPaletteRecord {
        kind: CockpitPaletteKind::Session,
        primary,
        secondary,
        search_text: searchable.join(" "),
        waiting: session.state == SessionState::Waiting,
        target: CockpitPaletteTarget::Session {
            key,
            host: host.to_string(),
            host_id: host_id.map(str::to_owned),
            session_id: session.session_id.clone(),
            provider: session.provider,
            config_dir: session.config_dir.clone(),
            account_email: session.account_email.clone(),
            account_id: session.account_id.clone(),
            is_local,
        },
    }
}

/// Build the complete dynamic index from one authoritative model generation.
/// The caller obtains `snapshot` and `fleet` in one model read, so this function
/// never observes a half-updated host/account projection.
pub fn build_palette_index(
    snapshot: &CockpitSnapshot,
    fleet: &FleetTree,
    repository: Option<&RepositoryContext>,
) -> Vec<CockpitPaletteRecord> {
    if matches!(&snapshot.health, ScanHealth::Pending) {
        return Vec::new();
    }

    let account_labels = account_display_by_route(snapshot);
    let mut records = Vec::new();
    for usage in &snapshot.accounts {
        let provider = title_provider(usage.account.provider);
        let email = usage.account.email.as_deref().unwrap_or_default();
        let secondary = [
            provider,
            email,
            usage.account.plan_tier.as_deref().unwrap_or_default(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty() && *part != usage.account.label)
        .collect::<Vec<_>>()
        .join(" · ");
        let search_text = [
            provider,
            usage.account.label.as_str(),
            email,
            usage.account.org.as_deref().unwrap_or_default(),
            usage.account.plan_tier.as_deref().unwrap_or_default(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
        records.push(CockpitPaletteRecord {
            kind: CockpitPaletteKind::Account,
            primary: usage.account.label.clone(),
            secondary,
            search_text,
            waiting: usage
                .sessions
                .iter()
                .any(|session| session.state == SessionState::Waiting),
            target: CockpitPaletteTarget::Account {
                account_key: usage.account.key.clone(),
            },
        });
    }

    let mut live_session_keys = HashSet::new();
    for host in fleet.hosts.iter().filter(|host| host.is_available()) {
        let host_identity = host_ident(host.is_local, host.host_id.as_deref());
        records.push(CockpitPaletteRecord {
            kind: CockpitPaletteKind::Host,
            primary: host.host.clone(),
            secondary: if host.is_local {
                "Local host".to_string()
            } else {
                "Connected host".to_string()
            },
            search_text: format!("host {}", host.host),
            waiting: host.needs_me > 0,
            target: CockpitPaletteTarget::Host {
                key: format!("host:{host_identity}"),
                registry_node_id: host.registry_node_id.clone(),
                host_id: host.host_id.clone(),
                host: host.host.clone(),
                is_local: host.is_local,
            },
        });

        if host.inventory_status != AgentInventoryStatus::Ready && !host.is_local {
            continue;
        }
        for project in &host.projects {
            let project_key = host_key(host.is_local, host.host_id.as_deref(), &project.root);
            records.push(CockpitPaletteRecord {
                kind: CockpitPaletteKind::Project,
                primary: project.name.clone(),
                secondary: host.host.clone(),
                search_text: format!("project {} {}", project.name, host.host),
                waiting: project.needs_me > 0,
                target: CockpitPaletteTarget::Project {
                    key: format!("project:{project_key}"),
                    registry_node_id: host.registry_node_id.clone(),
                    host_id: host.host_id.clone(),
                    host: host.host.clone(),
                    is_local: host.is_local,
                    project_root: PathBuf::from(&project.root),
                },
            });
            for session in &project.sessions {
                let key = session_key(host.is_local, host.host_id.as_deref(), session);
                let account_label = session
                    .account_email
                    .as_ref()
                    .and_then(|route| account_labels.get(&(session.provider, route.clone())))
                    .or_else(|| {
                        session.config_dir.as_ref().and_then(|route| {
                            account_labels.get(&(session.provider, route.clone()))
                        })
                    })
                    .map(String::as_str);
                records.push(session_record(
                    session,
                    &host.host,
                    host.host_id.as_deref(),
                    host.is_local,
                    account_label,
                ));
                live_session_keys.insert(key);
            }
        }
    }

    // Dormant local histories are deliberately outside FleetTree. Add them once,
    // retaining the account route while keeping config paths out of the index text.
    for usage in &snapshot.accounts {
        for session in usage
            .idle_sessions
            .iter()
            .take(MAX_DORMANT_SESSIONS_PER_ACCOUNT)
        {
            let key = session_key(true, None, session);
            if live_session_keys.insert(key) {
                records.push(session_record(
                    session,
                    "Local",
                    None,
                    true,
                    Some(&usage.account.label),
                ));
            }
        }
    }

    if let Some(repository) = repository {
        for flow_key in flow_keys() {
            let label = label_key_for_flow(flow_key).unwrap_or(flow_key);
            let key = format!(
                "github:{}:{}:{}",
                repository.slug,
                repository.worktree.display(),
                flow_key
            );
            records.push(CockpitPaletteRecord {
                kind: CockpitPaletteKind::GitHubFlow,
                primary: flow_display_label(flow_key).to_string(),
                secondary: repository.slug.clone(),
                search_text: format!(
                    "github {} {} {} {}",
                    flow_key, label, repository.slug, repository.display_label
                ),
                waiting: false,
                target: CockpitPaletteTarget::GitHubFlow {
                    key,
                    flow_key: flow_key.to_string(),
                    repository: repository.clone(),
                },
            });
        }
    }

    records
}

pub fn flow_display_label(flow_key: &str) -> &'static str {
    match flow_key {
        super::github_flows::FLOW_QUICK_ISSUE => "Draft GitHub issue",
        super::github_flows::FLOW_PR_REVIEW => "Review GitHub pull request",
        super::github_flows::FLOW_TRIAGE => "Triage GitHub issue",
        _ => "GitHub workflow",
    }
}

#[cfg(test)]
#[path = "palette_tests.rs"]
mod tests;
