//! Compact display projection for managed-fleet memory and headroom.
//!
//! The sidebar intentionally receives only a marker state. Memory metrics stay
//! in the main Cockpit details so the tree retains its navigation density.

#[cfg(not(target_family = "wasm"))]
use crate::remote_server::fleet_memory::{
    HostMemorySnapshot, MemoryMeasurement, MemoryMeasurementStatus, MemoryProvenance,
    ProcessMemorySnapshot,
};
#[cfg(not(target_family = "wasm"))]
use crate::remote_server::managed_fleet::{evaluate_headroom, HeadroomDecision, HeadroomPolicy};
use remote_server::proto::{
    AgentAccountInventory, ManagedSessionExitInfo, ManagedSessionInfo,
    MemoryMeasurement as ProtoMemoryMeasurement,
    MemoryMeasurementStatus as ProtoMemoryMeasurementStatus, SessionInfo, SessionList,
};
use zaplex_cockpit::{Provider, SessionSnapshot};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const DEFAULT_MAX_MEASUREMENT_AGE_MILLIS: u64 = 5_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FleetSidebarMarker {
    Managed,
}

pub(crate) fn sidebar_marker(managed: bool) -> Option<FleetSidebarMarker> {
    managed.then_some(FleetSidebarMarker::Managed)
}

/// One daemon-owned managed PTY as retained by the Cockpit model. The routing
/// identity is intentionally the complete wire identity: a display label can
/// never select a lifecycle target.
#[derive(Clone, Debug, PartialEq)]
pub struct ManagedFleetSession {
    pub(crate) host_id: String,
    pub(crate) host_label: String,
    pub(crate) registry_node_id: Option<String>,
    pub(crate) session_id: String,
    pub(crate) generation: u64,
    pub(crate) provider: Provider,
    pub(crate) account_id: String,
    /// Human account identity joined from the same host/provider inventory.
    /// The opaque daemon id remains routing-only and is never rendered.
    pub(crate) account_label: Option<String>,
    pub(crate) project_root: String,
    pub(crate) launch_kind: String,
    pub(crate) launch_id: String,
    pub(crate) process_memory: Option<ProtoMemoryMeasurement>,
    pub(crate) host_available_memory: Option<ProtoMemoryMeasurement>,
    pub(crate) daemon_min_available_bytes: u64,
    pub(crate) collected_at_epoch_millis: u64,
    pub(crate) exit: Option<ManagedFleetExit>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedFleetExit {
    pub(crate) exit_code: Option<i32>,
    pub(crate) exited_at_epoch_millis: u64,
    pub(crate) diagnostic_code: String,
}

impl ManagedFleetExit {
    pub(crate) fn state_label(&self) -> &'static str {
        match self.diagnostic_code.as_str() {
            "stopped" => "Stopped",
            "process-ended" => "Process ended",
            _ => unreachable!("managed exit codes are validated at the protocol boundary"),
        }
    }
}

impl ManagedFleetSession {
    pub(crate) fn is_claude_remote_control(&self) -> bool {
        self.launch_kind == "claude-remote-control"
    }

    pub(crate) fn matches_agent_session(
        &self,
        host_id: Option<&str>,
        session: &SessionSnapshot,
    ) -> bool {
        self.exit.is_none()
            && host_id == Some(self.host_id.as_str())
            && session.pty_session_id.as_deref() == Some(self.session_id.as_str())
            && session.pty_session_generation == Some(self.generation)
            && session.provider == self.provider
            && session.account_id.as_deref() == Some(self.account_id.as_str())
            && session.project_root == self.project_root
    }

    pub(crate) fn is_running(&self) -> bool {
        self.exit.is_none()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ManagedFleetInventory {
    sessions: Vec<ManagedFleetSession>,
}

impl ManagedFleetInventory {
    pub(crate) fn sessions(&self) -> &[ManagedFleetSession] {
        &self.sessions
    }

    pub(crate) fn extend_session_list(
        &mut self,
        host_id: &str,
        host_label: &str,
        registry_node_id: Option<&str>,
        list: SessionList,
    ) {
        let SessionList {
            sessions,
            host_available_memory,
            daemon_min_available_bytes,
            collected_at_epoch_millis,
            recent_managed_exits,
            ..
        } = list;
        for session in sessions {
            if let Some(projected) = managed_session_from_proto(
                host_id,
                host_label,
                registry_node_id,
                host_available_memory.clone(),
                daemon_min_available_bytes,
                collected_at_epoch_millis,
                session,
            ) {
                self.sessions.push(projected);
            }
        }
        for exited in recent_managed_exits {
            if let Some(projected) = managed_exit_from_proto(
                host_id,
                host_label,
                registry_node_id,
                host_available_memory.clone(),
                daemon_min_available_bytes,
                collected_at_epoch_millis,
                exited,
            ) {
                let replaced_by_live = self.sessions.iter().any(|live| {
                    live.is_running()
                        && live.host_id == projected.host_id
                        && live.provider == projected.provider
                        && live.account_id == projected.account_id
                        && live.project_root == projected.project_root
                        && live.launch_id == projected.launch_id
                });
                if !replaced_by_live {
                    self.sessions.push(projected);
                }
            }
        }
        self.sessions.sort_by(|left, right| {
            left.host_id
                .cmp(&right.host_id)
                .then_with(|| left.project_root.cmp(&right.project_root))
                .then_with(|| provider_key(left.provider).cmp(provider_key(right.provider)))
                .then_with(|| left.account_id.cmp(&right.account_id))
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
    }

    pub(crate) fn matching_agent_session(
        &self,
        host_id: Option<&str>,
        session: &SessionSnapshot,
    ) -> Option<&ManagedFleetSession> {
        let mut matches = self
            .sessions
            .iter()
            .filter(|managed| managed.matches_agent_session(host_id, session));
        let matched = matches.next()?;
        matches.next().is_none().then_some(matched)
    }

    pub(crate) fn exact(&self, target: &ManagedFleetSession) -> Option<&ManagedFleetSession> {
        let mut matches = self.sessions.iter().filter(|candidate| {
            candidate.host_id == target.host_id
                && candidate.session_id == target.session_id
                && candidate.generation == target.generation
                && candidate.launch_kind == target.launch_kind
                && candidate.launch_id == target.launch_id
                && candidate.provider == target.provider
                && candidate.account_id == target.account_id
                && candidate.project_root == target.project_root
                && candidate.exit == target.exit
        });
        let matched = matches.next()?;
        matches.next().is_none().then_some(matched)
    }

    pub(crate) fn exact_live(&self, target: &ManagedFleetSession) -> Option<&ManagedFleetSession> {
        self.exact(target).filter(|session| session.is_running())
    }

    pub(crate) fn enrich_account_labels(&mut self, inventory: &zaplex_cockpit::FleetTree) {
        for managed in &mut self.sessions {
            if managed.account_label.is_some() {
                continue;
            }
            let mut labels = inventory
                .hosts
                .iter()
                .filter(|host| host.host_id.as_deref() == Some(managed.host_id.as_str()))
                .flat_map(|host| host.projects.iter())
                .flat_map(|project| project.sessions.iter())
                .filter(|session| {
                    session.provider == managed.provider
                        && session.account_id.as_deref() == Some(managed.account_id.as_str())
                })
                .filter_map(|session| session.account_email.as_deref())
                .filter(|label| !label.trim().is_empty());
            let first = labels.next().map(str::to_string);
            managed.account_label = if labels.all(|label| Some(label) == first.as_deref()) {
                first
            } else {
                None
            };
        }
    }

    /// Joins daemon-managed sessions to the same host's fresh, path-free
    /// account inventory. Opaque account ids remain routing-only; ambiguous or
    /// malformed display identities fail closed and retain the neutral label.
    pub(crate) fn enrich_remote_account_labels(
        &mut self,
        host_id: &str,
        inventory: &AgentAccountInventory,
    ) {
        if inventory.schema_version != 1
            || !matches!(inventory.health.as_str(), "loaded" | "degraded")
        {
            return;
        }
        for managed in self
            .sessions
            .iter_mut()
            .filter(|managed| managed.host_id == host_id)
        {
            let provider = provider_key(managed.provider);
            let mut labels = inventory
                .accounts
                .iter()
                .filter(|account| {
                    account.provider == provider && account.account_id == managed.account_id
                })
                .filter_map(|account| {
                    let label = if account.display_label.trim().is_empty() {
                        account.email.trim()
                    } else {
                        account.display_label.trim()
                    };
                    (!label.is_empty()
                        && label.len() <= 512
                        && !label.chars().any(char::is_control))
                    .then(|| label.to_string())
                });
            let first = labels.next();
            managed.account_label = if labels.next().is_none() { first } else { None };
        }
    }

    pub(crate) fn remove_host(&mut self, host_id: &str) -> bool {
        let before = self.sessions.len();
        self.sessions.retain(|session| session.host_id != host_id);
        self.sessions.len() != before
    }
}

fn managed_session_from_proto(
    host_id: &str,
    host_label: &str,
    registry_node_id: Option<&str>,
    host_available_memory: Option<ProtoMemoryMeasurement>,
    daemon_min_available_bytes: u64,
    collected_at_epoch_millis: u64,
    session: SessionInfo,
) -> Option<ManagedFleetSession> {
    let ManagedSessionInfo {
        schema_version,
        provider,
        account_id,
        project_root,
        launch_kind,
        launch_id,
        generation,
    } = session.managed?;
    let provider = match provider.as_str() {
        "claude" => Provider::Claude,
        "codex" => Provider::Codex,
        _ => return None,
    };
    if schema_version != 1
        || generation == 0
        || generation != session.generation
        || host_id.is_empty()
        || session.session_id.is_empty()
        || account_id.is_empty()
        || project_root.is_empty()
        || launch_id.is_empty()
        || !matches!(
            launch_kind.as_str(),
            "interactive-agent" | "claude-remote-control"
        )
    {
        return None;
    }
    Some(ManagedFleetSession {
        host_id: host_id.to_string(),
        host_label: host_label.to_string(),
        registry_node_id: registry_node_id.map(str::to_string),
        session_id: session.session_id,
        generation,
        provider,
        account_id,
        account_label: None,
        project_root,
        launch_kind,
        launch_id,
        process_memory: session.process_memory,
        host_available_memory,
        daemon_min_available_bytes,
        collected_at_epoch_millis,
        exit: None,
    })
}

fn managed_exit_from_proto(
    host_id: &str,
    host_label: &str,
    registry_node_id: Option<&str>,
    host_available_memory: Option<ProtoMemoryMeasurement>,
    daemon_min_available_bytes: u64,
    collected_at_epoch_millis: u64,
    exited: ManagedSessionExitInfo,
) -> Option<ManagedFleetSession> {
    let ManagedSessionInfo {
        schema_version,
        provider,
        account_id,
        project_root,
        launch_kind,
        launch_id,
        generation,
    } = exited.managed?;
    let provider = match provider.as_str() {
        "claude" => Provider::Claude,
        "codex" => Provider::Codex,
        _ => return None,
    };
    if schema_version != 1
        || generation == 0
        || host_id.is_empty()
        || exited.session_id.is_empty()
        || account_id.is_empty()
        || project_root.is_empty()
        || launch_id.is_empty()
        || exited.exited_at_epoch_millis == 0
        || !matches!(exited.diagnostic_code.as_str(), "process-ended" | "stopped")
        || !matches!(
            launch_kind.as_str(),
            "interactive-agent" | "claude-remote-control"
        )
    {
        return None;
    }
    Some(ManagedFleetSession {
        host_id: host_id.to_string(),
        host_label: host_label.to_string(),
        registry_node_id: registry_node_id.map(str::to_string),
        session_id: exited.session_id,
        generation,
        provider,
        account_id,
        account_label: None,
        project_root,
        launch_kind,
        launch_id,
        process_memory: None,
        host_available_memory,
        daemon_min_available_bytes,
        collected_at_epoch_millis,
        exit: Some(ManagedFleetExit {
            exit_code: exited.exit_code,
            exited_at_epoch_millis: exited.exited_at_epoch_millis,
            diagnostic_code: exited.diagnostic_code,
        }),
    })
}

fn provider_key(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "claude",
        Provider::Codex => "codex",
        Provider::Antigravity => "antigravity",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FleetDetailHealth {
    Normal,
    Blocked,
    Degraded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FleetDetailRow {
    pub(crate) label: &'static str,
    pub(crate) value: String,
    pub(crate) hint: String,
    pub(crate) health: FleetDetailHealth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedFleetDetails {
    pub(crate) process_memory: FleetDetailRow,
    pub(crate) host_headroom: FleetDetailRow,
    pub(crate) launch_blocked: bool,
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn managed_fleet_details(
    process: Option<&ProcessMemorySnapshot>,
    host: &HostMemorySnapshot,
    policy: HeadroomPolicy,
    now_epoch_millis: u64,
) -> ManagedFleetDetails {
    let process_memory = process
        .map(|snapshot| {
            let timestamp_valid = snapshot.collected_at_epoch_millis <= now_epoch_millis
                && now_epoch_millis.saturating_sub(snapshot.collected_at_epoch_millis)
                    <= policy.max_measurement_age_millis();
            if timestamp_valid {
                memory_row("RAM", &snapshot.pss, "PSS · Linux")
            } else {
                unavailable_row("RAM", "Messung veraltet")
            }
        })
        .unwrap_or_else(|| unavailable_row("RAM", "Noch nicht gemessen"));
    let decision = evaluate_headroom(policy, host, now_epoch_millis);
    let (host_headroom, launch_blocked) = match decision {
        HeadroomDecision::Allowed {
            available_bytes,
            required_bytes,
        } => (
            FleetDetailRow {
                label: "Frei",
                value: format_bytes(available_bytes),
                hint: floor_hint(available_bytes, required_bytes),
                health: FleetDetailHealth::Normal,
            },
            false,
        ),
        HeadroomDecision::Denied {
            available_bytes,
            required_bytes,
            ..
        } => {
            let row = match available_bytes {
                Some(available_bytes) => FleetDetailRow {
                    label: "Frei",
                    value: format_bytes(available_bytes),
                    hint: floor_hint(available_bytes, required_bytes),
                    health: FleetDetailHealth::Blocked,
                },
                None => unavailable_row("Frei", "Nicht verfügbar"),
            };
            (row, true)
        }
    };
    ManagedFleetDetails {
        process_memory,
        host_headroom,
        launch_blocked,
    }
}

/// Render the additive wire projection without inventing a numeric value for a
/// missing/unsupported measurement. The daemon remains the authority for start
/// admission; `launch_blocked` here is explanatory UI state only.
pub(crate) fn managed_fleet_details_from_proto(
    session: &ManagedFleetSession,
) -> ManagedFleetDetails {
    let now_epoch_millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    managed_fleet_details_from_proto_at(session, now_epoch_millis)
}

pub(crate) fn managed_fleet_details_from_proto_at(
    session: &ManagedFleetSession,
    now_epoch_millis: u64,
) -> ManagedFleetDetails {
    let measurement_is_fresh = session.collected_at_epoch_millis != 0
        && session.collected_at_epoch_millis <= now_epoch_millis
        && now_epoch_millis.saturating_sub(session.collected_at_epoch_millis)
            <= DEFAULT_MAX_MEASUREMENT_AGE_MILLIS;
    if !measurement_is_fresh {
        return ManagedFleetDetails {
            process_memory: unavailable_row("RAM", "Messung veraltet"),
            host_headroom: unavailable_row("Frei", "Messung veraltet"),
            launch_blocked: true,
        };
    }
    let process_memory = proto_memory_row(
        "RAM",
        session.process_memory.as_ref(),
        "linux-proc-smaps-rollup",
        "PSS · Linux",
    );
    let host_headroom = proto_memory_row(
        "Frei",
        session.host_available_memory.as_ref(),
        "linux-proc-memavailable",
        "MemAvailable · Linux",
    );
    let launch_blocked = match (
        session
            .host_available_memory
            .as_ref()
            .and_then(|measurement| measurement.bytes),
        session.daemon_min_available_bytes,
    ) {
        (Some(available), floor) => floor > 0 && available < floor,
        (None, _) => true,
    };
    let host_headroom = if host_headroom.health == FleetDetailHealth::Normal
        && session.daemon_min_available_bytes > 0
    {
        let available = session
            .host_available_memory
            .as_ref()
            .and_then(|measurement| measurement.bytes)
            .expect("normal measured row carries bytes");
        FleetDetailRow {
            hint: floor_hint(available, session.daemon_min_available_bytes),
            health: if available < session.daemon_min_available_bytes {
                FleetDetailHealth::Blocked
            } else {
                FleetDetailHealth::Normal
            },
            ..host_headroom
        }
    } else {
        host_headroom
    };
    ManagedFleetDetails {
        process_memory,
        host_headroom,
        launch_blocked,
    }
}

fn proto_memory_row(
    label: &'static str,
    measurement: Option<&ProtoMemoryMeasurement>,
    expected_provenance: &str,
    measured_hint: &'static str,
) -> FleetDetailRow {
    let Some(measurement) = measurement else {
        return unavailable_row(label, "Nicht verfügbar");
    };
    let measured = ProtoMemoryMeasurementStatus::try_from(measurement.status)
        .is_ok_and(|status| status == ProtoMemoryMeasurementStatus::Measured);
    match (measured, measurement.bytes) {
        (true, Some(bytes)) if measurement.provenance == expected_provenance => FleetDetailRow {
            label,
            value: format_bytes(bytes),
            hint: measured_hint.to_string(),
            health: FleetDetailHealth::Normal,
        },
        (true, Some(_)) | (true, None) | (false, Some(_)) | (false, None) => {
            unavailable_row(label, "Nicht verfügbar")
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn memory_row(
    label: &'static str,
    measurement: &MemoryMeasurement,
    measured_hint: &'static str,
) -> FleetDetailRow {
    match (measurement.status(), measurement.bytes()) {
        (MemoryMeasurementStatus::Measured, Some(bytes)) => FleetDetailRow {
            label,
            value: format_bytes(bytes),
            hint: match measurement.provenance() {
                MemoryProvenance::LinuxProcSmapsRollup => measured_hint,
                MemoryProvenance::LinuxProcMemAvailable => "MemAvailable · Linux",
                MemoryProvenance::UnsupportedPlatform => "Nicht verfügbar",
            }
            .to_string(),
            health: FleetDetailHealth::Normal,
        },
        (MemoryMeasurementStatus::Measured, None)
        | (MemoryMeasurementStatus::Unavailable, None)
        | (MemoryMeasurementStatus::Unsupported, None)
        | (MemoryMeasurementStatus::Unavailable, Some(_))
        | (MemoryMeasurementStatus::Unsupported, Some(_)) => {
            unavailable_row(label, "Nicht verfügbar")
        }
    }
}

fn unavailable_row(label: &'static str, hint: &'static str) -> FleetDetailRow {
    FleetDetailRow {
        label,
        value: "—".to_string(),
        hint: hint.to_string(),
        health: FleetDetailHealth::Degraded,
    }
}

fn floor_hint(available_bytes: u64, required_bytes: u64) -> String {
    if available_bytes < required_bytes {
        format!("Minimum {}", format_bytes(required_bytes))
    } else {
        format!("MemAvailable · min. {}", format_bytes(required_bytes))
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= GIB {
        let tenths = bytes.saturating_mul(10) / GIB;
        format!("{}.{:01} GB", tenths / 10, tenths % 10)
    } else {
        format!("{} MB", bytes / MIB)
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "fleet_details_tests.rs"]
mod tests;
