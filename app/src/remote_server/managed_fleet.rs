//! Pure identity, launch, and headroom policy for managed remote agents.
//!
//! Wire decoding and process creation stay in `server_model`; this module keeps
//! the security-sensitive decisions deterministic and independently testable.

use super::fleet_memory::{
    HostMemorySnapshot, LinuxProcessIdentity, MemoryMeasurementStatus, MemoryProvenance,
};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use warp_util::path::ShellFamily;

pub(crate) const MANAGED_FLEET_SCHEMA_VERSION: u32 = 1;
pub(crate) const DEFAULT_MIN_AVAILABLE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub(crate) const DEFAULT_MAX_MEASUREMENT_AGE_MILLIS: u64 = 5_000;
const MAX_COMPONENT_BYTES: usize = 4096;
const MAX_DISPLAY_NAME_CHARS: usize = 80;
const MAX_REMOTE_CONTROL_CAPACITY: u16 = 256;
const MIB_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FleetValidationError {
    EmptyComponent,
    ComponentTooLong,
    ControlCharacter,
    ZeroGeneration,
    InvalidCapacity,
    InvalidHeadroom,
    ProviderMismatch,
    UnsupportedProvider,
}

impl FleetValidationError {
    pub(crate) fn protocol_code(self) -> &'static str {
        match self {
            Self::EmptyComponent => "empty-component",
            Self::ComponentTooLong => "component-too-long",
            Self::ControlCharacter => "control-character",
            Self::ZeroGeneration => "zero-generation",
            Self::InvalidCapacity => "invalid-capacity",
            Self::InvalidHeadroom => "invalid-headroom",
            Self::ProviderMismatch => "provider-mismatch",
            Self::UnsupportedProvider => "unsupported-provider",
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ManagedLaunchKey {
    host_id: String,
    account_id: String,
    project_root: String,
    provider: String,
}

impl ManagedLaunchKey {
    pub(crate) fn new(
        host_id: &str,
        account_id: &str,
        project_root: &str,
        provider: &str,
    ) -> Result<Self, FleetValidationError> {
        Ok(Self {
            host_id: validated_component(host_id)?,
            account_id: validated_component(account_id)?,
            project_root: validated_component(project_root)?,
            provider: validated_provider(provider)?,
        })
    }

    pub(crate) fn host_id(&self) -> &str {
        &self.host_id
    }

    pub(crate) fn account_id(&self) -> &str {
        &self.account_id
    }

    pub(crate) fn project_root(&self) -> &str {
        &self.project_root
    }

    pub(crate) fn provider(&self) -> &str {
        &self.provider
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedFleetIdentity {
    launch_key: ManagedLaunchKey,
    session_id: String,
    generation: u64,
}

impl ManagedFleetIdentity {
    pub(crate) fn new(
        launch_key: ManagedLaunchKey,
        session_id: &str,
        generation: u64,
    ) -> Result<Self, FleetValidationError> {
        if generation == 0 {
            return Err(FleetValidationError::ZeroGeneration);
        }
        Ok(Self {
            launch_key,
            session_id: validated_component(session_id)?,
            generation,
        })
    }

    pub(crate) fn launch_key(&self) -> &ManagedLaunchKey {
        &self.launch_key
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn matches_action(
        &self,
        launch_key: &ManagedLaunchKey,
        session_id: &str,
        generation: u64,
    ) -> bool {
        self.launch_key == *launch_key
            && self.session_id == session_id
            && self.generation == generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedLaunchKind {
    InteractiveAgent,
    ClaudeRemoteControl,
}

impl ManagedLaunchKind {
    pub(crate) fn protocol_name(self) -> &'static str {
        match self {
            Self::InteractiveAgent => "interactive-agent",
            Self::ClaudeRemoteControl => "claude-remote-control",
        }
    }

    pub(crate) fn validate_provider(self, provider: &str) -> Result<(), FleetValidationError> {
        match self {
            Self::InteractiveAgent
                if provider.eq_ignore_ascii_case("claude")
                    || provider.eq_ignore_ascii_case("codex") =>
            {
                Ok(())
            }
            Self::InteractiveAgent => Err(FleetValidationError::UnsupportedProvider),
            Self::ClaudeRemoteControl if provider.eq_ignore_ascii_case("claude") => Ok(()),
            Self::ClaudeRemoteControl => Err(FleetValidationError::ProviderMismatch),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ClaudeSpawnMode {
    SameDir,
    #[default]
    Worktree,
    Session,
}

impl ClaudeSpawnMode {
    pub(crate) fn cli_value(self) -> &'static str {
        match self {
            Self::SameDir => "same-dir",
            Self::Worktree => "worktree",
            Self::Session => "session",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClaudePermissionMode {
    AcceptEdits,
    Auto,
    BypassPermissions,
    Default,
    DontAsk,
    Plan,
}

impl ClaudePermissionMode {
    pub(crate) fn cli_value(self) -> &'static str {
        match self {
            Self::AcceptEdits => "acceptEdits",
            Self::Auto => "auto",
            Self::BypassPermissions => "bypassPermissions",
            Self::Default => "default",
            Self::DontAsk => "dontAsk",
            Self::Plan => "plan",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudeRemoteControlSpec {
    spawn_mode: ClaudeSpawnMode,
    capacity: u16,
    permission_mode: Option<ClaudePermissionMode>,
    display_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedLaunchPlan {
    launch_id: String,
    launch_key: ManagedLaunchKey,
    configuration: ManagedLaunchConfiguration,
    project_identity: Option<ManagedProjectIdentity>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ManagedProjectIdentity {
    device: u64,
    inode: u64,
}

impl ManagedProjectIdentity {
    #[cfg(unix)]
    pub(crate) fn capture(path: &Path) -> Option<Self> {
        let metadata = std::fs::symlink_metadata(path).ok()?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return None;
        }
        Some(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ManagedLaunchConfiguration {
    InteractiveAgent,
    ClaudeRemoteControl(ClaudeRemoteControlSpec),
}

impl ManagedLaunchPlan {
    pub(crate) fn interactive_agent(
        launch_id: &str,
        launch_key: ManagedLaunchKey,
    ) -> Result<Self, FleetValidationError> {
        ManagedLaunchKind::InteractiveAgent.validate_provider(launch_key.provider())?;
        Ok(Self {
            launch_id: validated_component(launch_id)?,
            launch_key,
            configuration: ManagedLaunchConfiguration::InteractiveAgent,
            project_identity: None,
        })
    }

    pub(crate) fn claude_remote_control(
        launch_id: &str,
        launch_key: ManagedLaunchKey,
        spec: ClaudeRemoteControlSpec,
    ) -> Result<Self, FleetValidationError> {
        ManagedLaunchKind::ClaudeRemoteControl.validate_provider(launch_key.provider())?;
        Ok(Self {
            launch_id: validated_component(launch_id)?,
            launch_key,
            configuration: ManagedLaunchConfiguration::ClaudeRemoteControl(spec),
            project_identity: None,
        })
    }

    pub(crate) fn launch_id(&self) -> &str {
        &self.launch_id
    }

    pub(crate) fn launch_key(&self) -> &ManagedLaunchKey {
        &self.launch_key
    }

    pub(crate) fn with_project_identity(mut self, identity: ManagedProjectIdentity) -> Self {
        self.project_identity = Some(identity);
        self
    }

    pub(crate) fn project_identity(&self) -> Option<ManagedProjectIdentity> {
        self.project_identity
    }

    #[cfg(unix)]
    pub(crate) fn project_identity_is_current(&self) -> bool {
        self.project_identity.is_some_and(|expected| {
            ManagedProjectIdentity::capture(Path::new(self.launch_key.project_root()))
                == Some(expected)
        })
    }

    pub(crate) fn kind(&self) -> ManagedLaunchKind {
        match &self.configuration {
            ManagedLaunchConfiguration::InteractiveAgent => ManagedLaunchKind::InteractiveAgent,
            ManagedLaunchConfiguration::ClaudeRemoteControl(_) => {
                ManagedLaunchKind::ClaudeRemoteControl
            }
        }
    }

    pub(crate) fn claude_spec(&self) -> Option<&ClaudeRemoteControlSpec> {
        match &self.configuration {
            ManagedLaunchConfiguration::InteractiveAgent => None,
            ManagedLaunchConfiguration::ClaudeRemoteControl(spec) => Some(spec),
        }
    }

    pub(crate) fn startup_command(&self, shell_family: ShellFamily) -> Vec<u8> {
        match &self.configuration {
            ManagedLaunchConfiguration::InteractiveAgent => {
                startup_command_from_argv(&[self.launch_key.provider().to_string()], shell_family)
            }
            ManagedLaunchConfiguration::ClaudeRemoteControl(spec) => {
                spec.startup_command(shell_family)
            }
        }
    }

    /// A transport retry is idempotent only when every immutable launch field
    /// matches. Reusing an id for another route/config is a conflict.
    pub(crate) fn is_retry_of(&self, other: &Self) -> bool {
        self == other
    }

    pub(crate) fn same_route_and_configuration(&self, other: &Self) -> bool {
        self.launch_key == other.launch_key && self.configuration == other.configuration
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedSessionMetadata {
    plan: ManagedLaunchPlan,
    process_root: Option<LinuxProcessIdentity>,
    account_route_identity: Option<super::agent_account::AccountRouteIdentity>,
}

impl ManagedSessionMetadata {
    pub(crate) fn new(plan: ManagedLaunchPlan, process_root: Option<LinuxProcessIdentity>) -> Self {
        Self {
            plan,
            process_root,
            account_route_identity: None,
        }
    }

    pub(crate) fn new_verified(
        plan: ManagedLaunchPlan,
        process_root: LinuxProcessIdentity,
        account_route_identity: super::agent_account::AccountRouteIdentity,
    ) -> Self {
        Self {
            plan,
            process_root: Some(process_root),
            account_route_identity: Some(account_route_identity),
        }
    }

    pub(crate) fn plan(&self) -> &ManagedLaunchPlan {
        &self.plan
    }

    pub(crate) fn process_root(&self) -> Option<LinuxProcessIdentity> {
        self.process_root
    }

    pub(crate) fn account_route_identity(
        &self,
    ) -> Option<&super::agent_account::AccountRouteIdentity> {
        self.account_route_identity.as_ref()
    }

    pub(crate) fn is_keepalive(&self) -> bool {
        true
    }
}

/// Detached-age GC is a lifecycle policy, not an implicit stop mechanism for
/// managed work. Explicit close/process-exit paths do not call this helper.
pub(crate) fn eligible_for_detached_age_gc(
    managed: Option<&ManagedSessionMetadata>,
    detached: bool,
    now_epoch_millis: u64,
    last_attached_epoch_millis: u64,
    max_detached_age_millis: u64,
) -> bool {
    managed.is_none()
        && detached
        && now_epoch_millis.saturating_sub(last_attached_epoch_millis) >= max_detached_age_millis
}

/// Ring-cap pressure may reclaim only unmanaged detached sessions. Managed
/// output rings remain bounded by their per-session ceiling; process memory is
/// handled by the start headroom gate rather than destructive surprise GC.
pub(crate) fn eligible_for_ring_pressure_gc(
    managed: Option<&ManagedSessionMetadata>,
    detached: bool,
) -> bool {
    managed.is_none() && detached
}

impl ClaudeRemoteControlSpec {
    pub(crate) fn new(
        spawn_mode: ClaudeSpawnMode,
        capacity: u16,
        permission_mode: Option<ClaudePermissionMode>,
        display_name: Option<&str>,
    ) -> Result<Self, FleetValidationError> {
        if capacity == 0 || capacity > MAX_REMOTE_CONTROL_CAPACITY {
            return Err(FleetValidationError::InvalidCapacity);
        }
        let display_name = display_name.map(validated_display_name).transpose()?;
        Ok(Self {
            spawn_mode,
            capacity,
            permission_mode,
            display_name,
        })
    }

    /// Returns argv data, never a shell command. The caller must preserve these
    /// argument boundaries or apply the active shell's existing escaping helper.
    pub(crate) fn argv(&self) -> Vec<String> {
        let mut argv = vec![
            "claude".to_string(),
            "remote-control".to_string(),
            "--spawn".to_string(),
            self.spawn_mode.cli_value().to_string(),
            "--capacity".to_string(),
            self.capacity.to_string(),
        ];
        if let Some(mode) = self.permission_mode {
            argv.push("--permission-mode".to_string());
            argv.push(mode.cli_value().to_string());
        }
        if let Some(name) = &self.display_name {
            argv.push("--name".to_string());
            argv.push(name.clone());
        }
        argv
    }

    pub(crate) fn spawn_mode(&self) -> ClaudeSpawnMode {
        self.spawn_mode
    }

    pub(crate) fn capacity(&self) -> u16 {
        self.capacity
    }

    pub(crate) fn permission_mode(&self) -> Option<ClaudePermissionMode> {
        self.permission_mode
    }

    pub(crate) fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    /// Builds the one-line startup delivery expected by the daemon PTY writer.
    /// Every argv item is escaped independently for the actual login shell;
    /// account selection remains daemon-local environment setup, not command
    /// text.
    pub(crate) fn startup_command(&self, shell_family: ShellFamily) -> Vec<u8> {
        startup_command_from_argv(&self.argv(), shell_family)
    }
}

impl Default for ClaudeRemoteControlSpec {
    fn default() -> Self {
        Self {
            spawn_mode: ClaudeSpawnMode::Worktree,
            capacity: 32,
            permission_mode: None,
            display_name: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HeadroomPolicy {
    daemon_floor_bytes: u64,
    requested_floor_bytes: Option<u64>,
    max_measurement_age_millis: u64,
}

impl HeadroomPolicy {
    pub(crate) fn new(
        daemon_floor_bytes: u64,
        requested_floor_bytes: Option<u64>,
        max_measurement_age_millis: u64,
    ) -> Result<Self, FleetValidationError> {
        if daemon_floor_bytes == 0
            || requested_floor_bytes == Some(0)
            || max_measurement_age_millis == 0
        {
            return Err(FleetValidationError::InvalidHeadroom);
        }
        Ok(Self {
            daemon_floor_bytes,
            requested_floor_bytes,
            max_measurement_age_millis,
        })
    }

    pub(crate) fn effective_floor_bytes(self) -> u64 {
        self.requested_floor_bytes
            .unwrap_or(0)
            .max(self.daemon_floor_bytes)
    }

    pub(crate) fn max_measurement_age_millis(self) -> u64 {
        self.max_measurement_age_millis
    }
}

impl Default for HeadroomPolicy {
    fn default() -> Self {
        Self {
            daemon_floor_bytes: DEFAULT_MIN_AVAILABLE_BYTES,
            requested_floor_bytes: None,
            max_measurement_age_millis: DEFAULT_MAX_MEASUREMENT_AGE_MILLIS,
        }
    }
}

/// Strictly parses the daemon's MiB setting. Missing configuration uses the
/// documented safe default; invalid or zero values fail instead of disabling
/// the gate accidentally.
pub(crate) fn daemon_headroom_floor_bytes(
    configured_mib: Option<&str>,
) -> Result<u64, FleetValidationError> {
    let Some(configured_mib) = configured_mib else {
        return Ok(DEFAULT_MIN_AVAILABLE_BYTES);
    };
    let mib = configured_mib
        .parse::<u64>()
        .map_err(|_| FleetValidationError::InvalidHeadroom)?;
    if mib == 0 {
        return Err(FleetValidationError::InvalidHeadroom);
    }
    mib.checked_mul(MIB_BYTES)
        .ok_or(FleetValidationError::InvalidHeadroom)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HeadroomDenialReason {
    BelowFloor,
    Unavailable,
    Unsupported,
    WrongProvenance,
    Stale,
    FutureDated,
}

impl HeadroomDenialReason {
    pub(crate) fn protocol_code(self) -> &'static str {
        match self {
            Self::BelowFloor => "below-floor",
            Self::Unavailable => "memory-unavailable",
            Self::Unsupported => "memory-unsupported",
            Self::WrongProvenance => "wrong-provenance",
            Self::Stale => "measurement-stale",
            Self::FutureDated => "measurement-future-dated",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HeadroomDecision {
    Allowed {
        available_bytes: u64,
        required_bytes: u64,
    },
    Denied {
        reason: HeadroomDenialReason,
        available_bytes: Option<u64>,
        required_bytes: u64,
    },
}

pub(crate) fn evaluate_headroom(
    policy: HeadroomPolicy,
    snapshot: &HostMemorySnapshot,
    now_epoch_millis: u64,
) -> HeadroomDecision {
    let required_bytes = policy.effective_floor_bytes();
    if snapshot.collected_at_epoch_millis > now_epoch_millis {
        return HeadroomDecision::Denied {
            reason: HeadroomDenialReason::FutureDated,
            available_bytes: snapshot.available.bytes(),
            required_bytes,
        };
    }
    if now_epoch_millis.saturating_sub(snapshot.collected_at_epoch_millis)
        > policy.max_measurement_age_millis()
    {
        return HeadroomDecision::Denied {
            reason: HeadroomDenialReason::Stale,
            available_bytes: snapshot.available.bytes(),
            required_bytes,
        };
    }
    if snapshot.available.provenance() != MemoryProvenance::LinuxProcMemAvailable {
        return HeadroomDecision::Denied {
            reason: match snapshot.available.status() {
                MemoryMeasurementStatus::Unsupported => HeadroomDenialReason::Unsupported,
                MemoryMeasurementStatus::Measured | MemoryMeasurementStatus::Unavailable => {
                    HeadroomDenialReason::WrongProvenance
                }
            },
            available_bytes: snapshot.available.bytes(),
            required_bytes,
        };
    }
    let available_bytes = match snapshot.available.status() {
        MemoryMeasurementStatus::Measured => snapshot.available.bytes(),
        MemoryMeasurementStatus::Unavailable => {
            return HeadroomDecision::Denied {
                reason: HeadroomDenialReason::Unavailable,
                available_bytes: None,
                required_bytes,
            };
        }
        MemoryMeasurementStatus::Unsupported => {
            return HeadroomDecision::Denied {
                reason: HeadroomDenialReason::Unsupported,
                available_bytes: None,
                required_bytes,
            };
        }
    };
    let Some(available_bytes) = available_bytes else {
        return HeadroomDecision::Denied {
            reason: HeadroomDenialReason::Unavailable,
            available_bytes: None,
            required_bytes,
        };
    };
    if available_bytes < required_bytes {
        return HeadroomDecision::Denied {
            reason: HeadroomDenialReason::BelowFloor,
            available_bytes: Some(available_bytes),
            required_bytes,
        };
    }
    HeadroomDecision::Allowed {
        available_bytes,
        required_bytes,
    }
}

fn validated_component(value: &str) -> Result<String, FleetValidationError> {
    if value.is_empty() {
        return Err(FleetValidationError::EmptyComponent);
    }
    if value.len() > MAX_COMPONENT_BYTES {
        return Err(FleetValidationError::ComponentTooLong);
    }
    if value.chars().any(char::is_control) {
        return Err(FleetValidationError::ControlCharacter);
    }
    Ok(value.to_string())
}

fn validated_provider(provider: &str) -> Result<String, FleetValidationError> {
    let provider = validated_component(provider)?;
    match provider.to_ascii_lowercase().as_str() {
        "claude" | "codex" => Ok(provider.to_ascii_lowercase()),
        _ => Err(FleetValidationError::UnsupportedProvider),
    }
}

fn validated_display_name(value: &str) -> Result<String, FleetValidationError> {
    let value = validated_component(value)?;
    if value.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(FleetValidationError::ComponentTooLong);
    }
    Ok(value)
}

fn startup_command_from_argv(argv: &[String], shell_family: ShellFamily) -> Vec<u8> {
    let mut command = argv
        .iter()
        .map(|argument| shell_family.escape(argument).into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    command.push('\n');
    command.into_bytes()
}

#[cfg(test)]
#[path = "managed_fleet_tests.rs"]
mod tests;
