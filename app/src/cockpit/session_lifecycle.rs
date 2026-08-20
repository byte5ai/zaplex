//! Fail-closed lifecycle plans for exact provider sessions.
//!
//! Restart, rename, and stale cleanup share one route type so none of them can
//! silently fall back from a remote account id to a local config directory or
//! from a stable host id to a display label. This module only decides whether
//! an operation is safe and preserves its intent; the workspace and daemon own
//! the actual terminal, process, and provider-registry mutations.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use zaplex_cockpit::{Provider, SessionSnapshot};

use crate::cockpit::{agent_of, launch_registry::LaunchRecord};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SessionHostRoute {
    Local,
    Remote { host_id: String, node_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SessionAccountRoute {
    Local {
        config_dir: Option<PathBuf>,
        account_email: Option<String>,
    },
    Remote {
        account_id: String,
        account_email: Option<String>,
    },
}

/// The complete provider/account/host identity of one conversation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionRoute {
    pub(crate) provider: Provider,
    pub(crate) session_id: String,
    pub(crate) host: SessionHostRoute,
    pub(crate) account: SessionAccountRoute,
    pub(crate) cwd: PathBuf,
    pub(crate) pid: u32,
    pub(crate) process_fingerprint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SessionRouteError {
    MissingSessionId,
    InvalidWorkingDirectory,
    MissingRemoteHostIdentity,
    MissingRemoteAccountIdentity,
    LeakedRemoteConfigDirectory,
    LocalRouteContainsRemoteAccount,
}

impl std::fmt::Display for SessionRouteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::MissingSessionId => "the provider session id is missing",
            Self::InvalidWorkingDirectory => "the session working directory is not absolute",
            Self::MissingRemoteHostIdentity => "the remote host identity is incomplete",
            Self::MissingRemoteAccountIdentity => "the remote account identity is missing",
            Self::LeakedRemoteConfigDirectory => {
                "a remote route contains a host-local config directory"
            }
            Self::LocalRouteContainsRemoteAccount => {
                "a local route contains a daemon account identity"
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for SessionRouteError {}

impl SessionRoute {
    pub(crate) fn from_snapshot(
        session: &SessionSnapshot,
        is_local: bool,
        host_id: Option<&str>,
        node_id: Option<&str>,
    ) -> Result<Self, SessionRouteError> {
        if session.session_id.trim().is_empty() {
            return Err(SessionRouteError::MissingSessionId);
        }
        let cwd = PathBuf::from(&session.cwd);
        let cwd_is_absolute = if is_local {
            cwd.is_absolute()
        } else {
            session.cwd.starts_with('/')
        };
        if !cwd_is_absolute {
            return Err(SessionRouteError::InvalidWorkingDirectory);
        }

        let (host, account) = if is_local {
            if session.account_id.is_some() {
                return Err(SessionRouteError::LocalRouteContainsRemoteAccount);
            }
            (
                SessionHostRoute::Local,
                SessionAccountRoute::Local {
                    config_dir: session.config_dir.as_deref().map(PathBuf::from),
                    account_email: session.account_email.clone(),
                },
            )
        } else {
            let host_id = host_id
                .filter(|value| !value.trim().is_empty())
                .ok_or(SessionRouteError::MissingRemoteHostIdentity)?;
            let node_id = node_id
                .filter(|value| !value.trim().is_empty())
                .ok_or(SessionRouteError::MissingRemoteHostIdentity)?;
            if session.config_dir.is_some() {
                return Err(SessionRouteError::LeakedRemoteConfigDirectory);
            }
            let account_id = session
                .account_id
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or(SessionRouteError::MissingRemoteAccountIdentity)?;
            (
                SessionHostRoute::Remote {
                    host_id: host_id.to_string(),
                    node_id: node_id.to_string(),
                },
                SessionAccountRoute::Remote {
                    account_id: account_id.to_string(),
                    account_email: session.account_email.clone(),
                },
            )
        };

        Ok(Self {
            provider: session.provider,
            session_id: session.session_id.clone(),
            host,
            account,
            cwd,
            pid: session.pid,
            process_fingerprint: session.process_fingerprint.clone(),
        })
    }

    fn launch_record_matches(&self, record: &LaunchRecord) -> bool {
        let expected_host = match &self.host {
            SessionHostRoute::Local => None,
            SessionHostRoute::Remote { host_id, .. } => Some(host_id.as_str()),
        };
        let (config_dir, account_email, account_id) = match &self.account {
            SessionAccountRoute::Local {
                config_dir,
                account_email,
            } => (
                config_dir.as_deref().map(Path::to_string_lossy),
                account_email.as_deref(),
                None,
            ),
            SessionAccountRoute::Remote {
                account_id,
                account_email,
            } => (None, account_email.as_deref(), Some(account_id.as_str())),
        };
        record.agent == agent_of(self.provider)
            && record.host.as_deref() == expected_host
            && record.cwd.as_deref() == Some(self.cwd.as_path())
            && record.config_dir.as_deref() == config_dir.as_deref()
            && record.account_email.as_deref() == account_email
            && record.account_id.as_deref() == account_id
    }
}

/// Immediate evidence about the process or terminal that must be replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RestartPresence {
    Dormant,
    ExactTerminal { terminal_key: String },
    VerifiedProcess,
    ProcessReused,
    Unverifiable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SessionLifecycleCapabilities {
    pub(crate) can_restart: bool,
    pub(crate) can_rename: bool,
    pub(crate) can_cleanup_stale: bool,
}

pub(crate) fn lifecycle_capabilities(
    route: &SessionRoute,
    presence: &RestartPresence,
    exact_launch_bound: bool,
    cleanup_candidate: bool,
) -> SessionLifecycleCapabilities {
    let provider_resumes = agent_of(route.provider)
        .resume_command(&route.session_id)
        .is_some();
    let exact_running_target = matches!(
        presence,
        RestartPresence::ExactTerminal { .. } | RestartPresence::VerifiedProcess
    );
    SessionLifecycleCapabilities {
        can_restart: exact_launch_bound && provider_resumes && exact_running_target,
        can_rename: matches!(&route.host, SessionHostRoute::Local)
            && matches!(route.provider, Provider::Claude | Provider::Codex),
        can_cleanup_stale: route.provider == Provider::Claude && cleanup_candidate,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RestartTermination {
    None,
    ExactTerminal { terminal_key: String },
    VerifiedProcess { pid: u32, fingerprint: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResumeInvocation {
    LocalShell {
        command: String,
    },
    RemoteDaemon {
        host_id: String,
        node_id: String,
        account_id: String,
        provider: Provider,
        session_id: String,
        cwd: PathBuf,
        model: Option<String>,
        effort: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RestartPlan {
    pub(crate) route: SessionRoute,
    pub(crate) termination: RestartTermination,
    pub(crate) resume: ResumeInvocation,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LifecycleOperationId(u64);

impl LifecycleOperationId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleOperationState {
    Pending,
    Applied,
    Failed { retryable: bool, message: String },
}

/// One stable mutation identity. Retrying a partial failure retains the same
/// id and target, while an applied operation can never return to Pending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LifecycleOperation<T> {
    pub(crate) id: LifecycleOperationId,
    pub(crate) target: T,
    pub(crate) state: LifecycleOperationState,
}

impl<T> LifecycleOperation<T> {
    pub(crate) fn new(target: T) -> Self {
        Self {
            id: LifecycleOperationId::next(),
            target,
            state: LifecycleOperationState::Pending,
        }
    }

    pub(crate) fn mark_applied(&mut self) {
        if !matches!(self.state, LifecycleOperationState::Applied) {
            self.state = LifecycleOperationState::Applied;
        }
    }

    pub(crate) fn mark_failed(&mut self, retryable: bool, message: impl Into<String>) {
        if !matches!(self.state, LifecycleOperationState::Applied) {
            self.state = LifecycleOperationState::Failed {
                retryable,
                message: message.into(),
            };
        }
    }

    pub(crate) fn retry(&mut self) -> bool {
        match self.state {
            LifecycleOperationState::Failed {
                retryable: true, ..
            } => {
                self.state = LifecycleOperationState::Pending;
                true
            }
            LifecycleOperationState::Pending
            | LifecycleOperationState::Applied
            | LifecycleOperationState::Failed {
                retryable: false, ..
            } => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RestartPlanError {
    LaunchIntentUnbound,
    ProcessIdentityUnavailable,
    ProcessIdentityChanged,
    ProviderCannotResume,
}

/// Build an immutable terminate-then-resume plan from one exact launch record.
/// Any account/host mismatch or uncertain process identity rejects the whole
/// operation before it can terminate or launch anything.
pub(crate) fn plan_restart(
    route: SessionRoute,
    presence: RestartPresence,
    record: &LaunchRecord,
) -> Result<RestartPlan, RestartPlanError> {
    if !route.launch_record_matches(record) {
        return Err(RestartPlanError::LaunchIntentUnbound);
    }
    let termination = match presence {
        RestartPresence::Dormant => RestartTermination::None,
        RestartPresence::ExactTerminal { terminal_key } => {
            RestartTermination::ExactTerminal { terminal_key }
        }
        RestartPresence::VerifiedProcess => {
            let fingerprint = route
                .process_fingerprint
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or(RestartPlanError::ProcessIdentityUnavailable)?;
            if route.pid == 0 {
                return Err(RestartPlanError::ProcessIdentityUnavailable);
            }
            RestartTermination::VerifiedProcess {
                pid: route.pid,
                fingerprint,
            }
        }
        RestartPresence::ProcessReused => return Err(RestartPlanError::ProcessIdentityChanged),
        RestartPresence::Unverifiable => return Err(RestartPlanError::ProcessIdentityUnavailable),
    };

    let agent = agent_of(route.provider);
    let resume = match (&route.host, &route.account) {
        (SessionHostRoute::Local, SessionAccountRoute::Local { config_dir, .. }) => {
            ResumeInvocation::LocalShell {
                command: agent
                    .resume_command_routed_with(
                        &route.session_id,
                        config_dir.as_deref(),
                        record.model.as_deref(),
                        record.effort.as_deref(),
                    )
                    .ok_or(RestartPlanError::ProviderCannotResume)?,
            }
        }
        (
            SessionHostRoute::Remote { host_id, node_id },
            SessionAccountRoute::Remote { account_id, .. },
        ) => ResumeInvocation::RemoteDaemon {
            host_id: host_id.clone(),
            node_id: node_id.clone(),
            account_id: account_id.clone(),
            provider: route.provider,
            session_id: route.session_id.clone(),
            cwd: route.cwd.clone(),
            model: record.model.clone(),
            effort: record.effort.clone(),
        },
        (SessionHostRoute::Local, SessionAccountRoute::Remote { .. })
        | (SessionHostRoute::Remote { .. }, SessionAccountRoute::Local { .. }) => {
            return Err(RestartPlanError::LaunchIntentUnbound)
        }
    };

    Ok(RestartPlan {
        route,
        termination,
        resume,
        model: record.model.clone(),
        effort: record.effort.clone(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CleanupProcessEvidence {
    Dead,
    MatchingLive,
    ProcessReused,
    Unverifiable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CleanupRejection {
    InventoryChanged,
    SessionStillVisible,
    ProcessStillLive,
    ProcessIdentityChanged,
    ProcessIdentityUnavailable,
}

/// Cleanup is allowed only against the same inventory revision after both the
/// current inventory and an immediate process proof say the entry is gone.
pub(crate) fn authorize_stale_cleanup(
    candidate_revision: u64,
    current_revision: u64,
    still_visible: bool,
    process: CleanupProcessEvidence,
) -> Result<(), CleanupRejection> {
    if candidate_revision != current_revision {
        return Err(CleanupRejection::InventoryChanged);
    }
    if still_visible {
        return Err(CleanupRejection::SessionStillVisible);
    }
    match process {
        CleanupProcessEvidence::Dead => Ok(()),
        CleanupProcessEvidence::MatchingLive => Err(CleanupRejection::ProcessStillLive),
        CleanupProcessEvidence::ProcessReused => Err(CleanupRejection::ProcessIdentityChanged),
        CleanupProcessEvidence::Unverifiable => Err(CleanupRejection::ProcessIdentityUnavailable),
    }
}

pub(crate) const MAX_SESSION_NAME_BYTES: usize = 80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SessionNameError {
    Empty,
    TooLong,
    ControlCharacter,
    Conflict,
}

pub(crate) fn validate_session_name(name: &str) -> Result<String, SessionNameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(SessionNameError::Empty);
    }
    if name.len() > MAX_SESSION_NAME_BYTES {
        return Err(SessionNameError::TooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(SessionNameError::ControlCharacter);
    }
    Ok(name.to_string())
}

/// Names conflict only within the exact provider/host/account scope. The
/// current route may retain its own name, while a copied session id or account
/// on another host never aliases it.
pub(crate) fn validate_rename_conflict<'a>(
    route: &SessionRoute,
    requested: &str,
    existing: impl IntoIterator<Item = (&'a SessionRoute, &'a str)>,
) -> Result<String, SessionNameError> {
    let requested = validate_session_name(requested)?;
    let conflict = existing.into_iter().any(|(candidate, name)| {
        candidate != route
            && candidate.provider == route.provider
            && candidate.host == route.host
            && candidate.account == route.account
            && name.eq_ignore_ascii_case(&requested)
    });
    if conflict {
        Err(SessionNameError::Conflict)
    } else {
        Ok(requested)
    }
}

#[cfg(test)]
#[path = "session_lifecycle_tests.rs"]
mod tests;
