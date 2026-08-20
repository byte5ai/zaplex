use crate::terminal::bootstrap::{daemon_bootstrap_delivery, DaemonBootstrapDelivery};
use crate::terminal::shell::ShellType;
use repo_metadata::repositories::{DetectedRepositories, RepoDetectionSource};
use repo_metadata::{RepoMetadataEvent, RepoMetadataModel, RepositoryIdentifier};
#[cfg(unix)]
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use warp_core::channel::ChannelState;
use warp_core::SessionId;
#[cfg(unix)]
use warp_util::path::ShellFamily;
use warp_util::standardized_path::StandardizedPath;
use warpui::platform::TerminationMode;
use warpui::r#async::{Spawnable, SpawnableOutput, SpawnedFutureHandle};
use warpui::{Entity, ModelContext, SingletonEntity};

use warp_files::{FileModel, FileModelEvent};
use warp_util::content_version::ContentVersion;
use warp_util::file::FileId;

use super::proto::{
    client_message, delete_file_response, run_command_response, server_message,
    write_file_response, Abort, AgentProcessSignal, AgentProcessSignalRequest,
    AgentProcessSignalResponse, AgentProcessSignalStatus, AgentPtyBindingResponse,
    AgentPtyBindingStatus, AgentSessionIdentity, AgentSessionInfo, AgentSessionList, Authenticate,
    ClientMessage, DeleteFile, DeleteFileResponse, DeleteFileSuccess, ErrorCode, ErrorResponse,
    FailedFileRead, FileContextProto, FileOperationError, HostExec, HostExecResult, Initialize,
    InitializeResponse, ManagedSessionLifecycleResponse, ManagedSessionLifecycleStatus,
    NavigatedToDirectory, NavigatedToDirectoryResponse, ReadAgentTranscript,
    ReadFileContextResponse, RunCommandError, RunCommandErrorCode, RunCommandRequest,
    RunCommandResponse, RunCommandSuccess, ServerMessage, SessionBootstrapped, StartupCommandAck,
    WriteFile, WriteFileResponse, WriteFileSuccess,
};
#[cfg(unix)]
use super::proto::{
    AttachSession, BindAgentPty, CloseSession, DetachSession, ManagedLaunch,
    ManagedSessionExitInfo, ManagedSessionInfo, ManagedSessionLifecycleAction,
    ManagedSessionLifecycleRequest, MemoryMeasurement, MemoryMeasurementStatus, OpenSession,
    ResizeSession, SessionAttached, SessionExited, SessionInfo, SessionInput, SessionList,
    SessionNotice, SessionOpened, SessionOutput, SessionSize, SetBootstrapPreamble, UnbindAgentPty,
};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use zaplex_cockpit::{GuardrailSignal, ProcessSignalError};
#[cfg(unix)]
use zaplex_remote_session::agent_binding::{
    AgentIdentity, AgentPtyBindings, BindingError, BindingRequest,
};
#[cfg(unix)]
use zaplex_remote_session::types::FEATURE_MULTIPLEXER_INVENTORY_V1;
use zaplex_remote_session::types::{
    supported_features, FEATURE_AGENT_ACCOUNT_ROUTING_V1, FEATURE_AGENT_PROCESS_SIGNAL_V1,
    FEATURE_AGENT_PTY_BINDING_V2, FEATURE_AGENT_TRANSCRIPT_READ_V1, FEATURE_MANAGED_AGENT_FLEET_V1,
    FEATURE_SAFE_FILE_TRANSACTIONS_V1,
};

// Buffer-sync related: depends on GlobalBufferModel, which server-local operations are only
// available under `local_fs`, so the entire server-side buffer handling is gated by `local_fs`.
#[cfg(feature = "local_fs")]
use super::proto::{
    create_directory_response, list_directory_response, read_file_chunk_response,
    resolve_conflict_response, resolve_path_response, save_buffer_response,
    write_file_chunk_response, BufferEdit, BufferUpdatedPush, CloseBuffer, CreateDirectory,
    CreateDirectoryResponse, CreateDirectorySuccess, DirEntry, FileSystemEntryKind, ListDirectory,
    ListDirectoryResponse, ListDirectorySuccess, OpenBuffer, OpenBufferResponse, ReadFileChunk,
    ReadFileChunkResponse, ReadFileChunkSuccess, ResolveConflict, ResolveConflictResponse,
    ResolveConflictSuccess, ResolvePath, ResolvePathResponse, ResolvePathSuccess, SaveBuffer,
    SaveBufferResponse, SaveBufferSuccess, TextEdit, WriteFileChunk, WriteFileChunkResponse,
    WriteFileChunkSuccess,
};
#[cfg(feature = "local_fs")]
use super::server_buffer_tracker::{PendingBufferRequestKind, ServerBufferTracker};
#[cfg(feature = "local_fs")]
use crate::code::global_buffer_model::{GlobalBufferModel, GlobalBufferModelEvent};

/// How long the daemon waits with no connections before exiting.
pub const GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Reap a session that has had no attached connection for this long. Detached
/// sessions otherwise live indefinitely so a client can reconnect (laptop
/// closed, etc.); this bounds memory against truly abandoned ones (Stage 4).
#[cfg(unix)]
const MAX_DETACHED_SESSION_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
/// Soft cap on total output-ring bytes across all of this host's sessions. When
/// exceeded, the oldest *detached* sessions are reaped until back under it
/// (live, attached sessions are never reaped).
#[cfg(unix)]
const HOST_RING_CAP_BYTES: usize = 256 * 1024 * 1024;
/// How often the detached-session GC sweep runs.
#[cfg(unix)]
const GC_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const MAX_CONCURRENT_AGENT_TRANSCRIPT_READS: usize = 2;
#[cfg(unix)]
const MAX_CONCURRENT_MANAGED_MEMORY_READS: usize = 1;
#[cfg(unix)]
const MAX_RECENT_MANAGED_EXITS: usize = 32;
#[cfg(unix)]
const RECENT_MANAGED_EXIT_TTL_MILLIS: u64 = 15 * 60 * 1000;

/// Unique identifier for a connected proxy session in daemon mode.
pub type ConnectionId = uuid::Uuid;
use super::protocol::RequestId;
use crate::ai::agent::FileLocations;
use crate::ai::blocklist::{read_local_file_context, ReadFileContextResult};
use crate::terminal::model::session::command_executor::{
    ExecuteCommandOptions, LocalCommandExecutor,
};

/// Outcome of dispatching a request-style `ClientMessage`.
///
/// Notifications (fire-and-forget messages like `SessionBootstrapped` and
/// `Abort`) do not produce a `HandlerOutcome`; they are dispatched inline in
/// `handle_message` and return early.
enum HandlerOutcome {
    /// The response is ready synchronously — the caller sends it immediately.
    Sync(server_message::Message),
    /// The handler initiated async work whose response will be sent later.
    ///
    /// When the handle is `Some`, the caller inserts it into `in_progress`
    /// so the request can be cancelled via `Abort`. Removal on
    /// completion/abort is arranged by [`ServerModel::spawn_request_handler`].
    ///
    /// `None` is used for async work whose completion is delivered through
    /// a separate event subscription and is not currently cancellable via
    /// `Abort` (e.g. `FileModel` events for file writes and deletes, which
    /// are tracked by `FileId` in `pending_file_ops` rather than by
    /// `RequestId` in `in_progress`).
    Async(Option<SpawnedFutureHandle>),
}

struct AgentTranscriptReadPermit {
    in_flight: Arc<AtomicUsize>,
}

impl AgentTranscriptReadPermit {
    fn try_acquire(in_flight: Arc<AtomicUsize>) -> Option<Self> {
        in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_CONCURRENT_AGENT_TRANSCRIPT_READS).then_some(current + 1)
            })
            .ok()
            .map(|_| Self { in_flight })
    }
}

impl Drop for AgentTranscriptReadPermit {
    fn drop(&mut self) {
        let previous = self.in_flight.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
    }
}

#[cfg(unix)]
struct ManagedMemoryReadPermit {
    in_flight: Arc<AtomicUsize>,
}

#[cfg(unix)]
impl ManagedMemoryReadPermit {
    fn try_acquire(in_flight: Arc<AtomicUsize>) -> Option<Self> {
        in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_CONCURRENT_MANAGED_MEMORY_READS).then_some(current + 1)
            })
            .ok()
            .map(|_| Self { in_flight })
    }
}

#[cfg(unix)]
impl Drop for ManagedMemoryReadPermit {
    fn drop(&mut self) {
        let previous = self.in_flight.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
    }
}

#[cfg(unix)]
#[derive(Clone, Debug)]
struct ManagedExitRecord {
    plan: super::managed_fleet::ManagedLaunchPlan,
    account_route_identity: super::agent_account::AccountRouteIdentity,
    session_id: String,
    generation: u64,
    exit_code: Option<i32>,
    exited_at_epoch_millis: u64,
    shell: String,
    rows: usize,
    cols: usize,
    ring_ceiling_bytes: u64,
    diagnostic: ManagedExitDiagnostic,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManagedExitDiagnostic {
    ProcessEnded,
    Stopped,
}

#[cfg(unix)]
impl ManagedExitDiagnostic {
    fn protocol_code(self) -> &'static str {
        match self {
            Self::ProcessEnded => "process-ended",
            Self::Stopped => "stopped",
        }
    }
}

#[cfg(unix)]
impl ManagedExitRecord {
    fn matches(&self, request: &ManagedSessionLifecycleRequest) -> bool {
        self.session_id == request.session_id
            && self.generation == request.expected_generation
            && self.plan.launch_id() == request.launch_id
            && self.plan.launch_key().provider() == request.provider
            && self.plan.launch_key().account_id() == request.account_id
            && self.plan.launch_key().project_root() == request.project_root
    }

    fn to_proto(&self) -> ManagedSessionExitInfo {
        ManagedSessionExitInfo {
            managed: Some(managed_session_plan_info(&self.plan, self.generation)),
            session_id: self.session_id.clone(),
            exit_code: self.exit_code,
            exited_at_epoch_millis: self.exited_at_epoch_millis,
            diagnostic_code: self.diagnostic.protocol_code().to_string(),
        }
    }
}

#[cfg(unix)]
fn push_recent_managed_exit(records: &mut VecDeque<ManagedExitRecord>, record: ManagedExitRecord) {
    records.retain(|existing| {
        record
            .exited_at_epoch_millis
            .saturating_sub(existing.exited_at_epoch_millis)
            <= RECENT_MANAGED_EXIT_TTL_MILLIS
            && (existing.session_id != record.session_id
                || existing.generation != record.generation)
    });
    while records.len() >= MAX_RECENT_MANAGED_EXITS {
        records.pop_front();
    }
    records.push_back(record);
}

#[cfg(test)]
impl HandlerOutcome {
    fn into_message(self) -> server_message::Message {
        match self {
            HandlerOutcome::Sync(message) => message,
            HandlerOutcome::Async(_) => panic!("expected synchronous handler outcome"),
        }
    }
}

fn execute_agent_process_signal_with<F>(
    req: AgentProcessSignalRequest,
    current_sessions: &[AgentSessionInfo],
    capability_negotiated: bool,
    send: F,
) -> AgentProcessSignalResponse
where
    F: FnOnce(u32, &str, GuardrailSignal) -> Result<(), ProcessSignalError>,
{
    let AgentProcessSignalRequest {
        session_id,
        pid,
        expected_process_fingerprint,
        signal,
    } = req;

    let failure =
        |status: AgentProcessSignalStatus, error_message: String| AgentProcessSignalResponse {
            session_id: session_id.clone(),
            pid,
            status: status.into(),
            error_message,
        };

    if !capability_negotiated {
        return failure(
            AgentProcessSignalStatus::InvalidRequest,
            "agent-process-signal-v1 capability was not negotiated".to_string(),
        );
    }
    if session_id.trim().is_empty() {
        return failure(
            AgentProcessSignalStatus::InvalidRequest,
            "agent session id is empty".to_string(),
        );
    }
    if expected_process_fingerprint.is_empty() {
        return failure(
            AgentProcessSignalStatus::IdentityUnverifiable,
            "expected process fingerprint is empty".to_string(),
        );
    }

    let signal = match AgentProcessSignal::try_from(signal) {
        Ok(AgentProcessSignal::Interrupt) => GuardrailSignal::Interrupt,
        Ok(AgentProcessSignal::Kill) => GuardrailSignal::Kill,
        Ok(AgentProcessSignal::Unspecified) | Err(_) => {
            return failure(
                AgentProcessSignalStatus::InvalidRequest,
                "unsupported agent process signal".to_string(),
            );
        }
    };

    let mut matching_sessions = current_sessions
        .iter()
        .filter(|session| session.session_id == session_id);
    let Some(registered) = matching_sessions.next() else {
        return failure(
            AgentProcessSignalStatus::InvalidRequest,
            "agent session is not present in the current inventory".to_string(),
        );
    };
    if matching_sessions.next().is_some() {
        return failure(
            AgentProcessSignalStatus::InvalidRequest,
            "agent session id is ambiguous in the current inventory".to_string(),
        );
    }
    if registered.pid != pid {
        return failure(
            AgentProcessSignalStatus::StaleIdentity,
            "process id no longer matches the current agent inventory".to_string(),
        );
    }
    if registered.process_fingerprint.trim().is_empty() {
        return failure(
            AgentProcessSignalStatus::IdentityUnverifiable,
            "current agent inventory has no verified process fingerprint".to_string(),
        );
    }
    if registered.process_fingerprint != expected_process_fingerprint {
        return failure(
            AgentProcessSignalStatus::StaleIdentity,
            "process fingerprint no longer matches the current agent inventory".to_string(),
        );
    }

    match send(registered.pid, &registered.process_fingerprint, signal) {
        Ok(()) => AgentProcessSignalResponse {
            session_id: session_id.clone(),
            pid,
            status: AgentProcessSignalStatus::Sent.into(),
            error_message: String::new(),
        },
        Err(error) => {
            let status = match &error {
                ProcessSignalError::InvalidPid => AgentProcessSignalStatus::InvalidRequest,
                ProcessSignalError::IdentityUnavailable(_) => {
                    AgentProcessSignalStatus::IdentityUnverifiable
                }
                ProcessSignalError::IdentityChanged => AgentProcessSignalStatus::StaleIdentity,
                ProcessSignalError::SignalFailed(_) => AgentProcessSignalStatus::SignalFailed,
                ProcessSignalError::UnsupportedPlatform => {
                    AgentProcessSignalStatus::IdentityUnverifiable
                }
            };
            failure(status, error.to_string())
        }
    }
}

fn server_features_with_runtime_support(
    process_signalling_supported: bool,
    safe_file_transactions_supported: bool,
) -> Vec<String> {
    let mut features = supported_features();
    if !process_signalling_supported {
        features.retain(|feature| {
            feature != FEATURE_AGENT_PROCESS_SIGNAL_V1 && feature != FEATURE_MANAGED_AGENT_FLEET_V1
        });
    }
    if !safe_file_transactions_supported {
        features.retain(|feature| feature != FEATURE_SAFE_FILE_TRANSACTIONS_V1);
    }
    features
}

/// Tracks an in-flight file write or delete so the async completion
/// event can be correlated back to the originating client request.
enum FileOpKind {
    Write,
    Delete,
}

struct PendingFileOp {
    request_id: RequestId,
    conn_id: ConnectionId,
    kind: FileOpKind,
}

/// Manages pending file operations and ensures that the corresponding
/// `FileModel` entry is always cleaned up when an operation completes
/// or fails, preventing `FileState` leaks.
struct PendingFileOps {
    ops: HashMap<FileId, PendingFileOp>,
}

impl PendingFileOps {
    fn new() -> Self {
        Self {
            ops: HashMap::new(),
        }
    }

    /// Registers a file path with `FileModel`, sets the initial version,
    /// and tracks the pending operation. Returns the `FileId` and
    /// `ContentVersion` for the caller to initiate the actual I/O.
    fn insert(
        &mut self,
        path: &Path,
        request_id: RequestId,
        conn_id: ConnectionId,
        kind: FileOpKind,
        ctx: &mut ModelContext<ServerModel>,
    ) -> (FileId, ContentVersion) {
        let file_model = FileModel::handle(ctx);
        let file_id = file_model.update(ctx, |m, ctx| m.register_file_path(path, false, ctx));
        let version = ContentVersion::new();
        file_model.update(ctx, |m, _| m.set_version(file_id, version));
        self.ops.insert(
            file_id,
            PendingFileOp {
                request_id,
                conn_id,
                kind,
            },
        );
        (file_id, version)
    }

    fn get(&self, file_id: &FileId) -> Option<&PendingFileOp> {
        self.ops.get(file_id)
    }

    /// Removes a pending operation and unsubscribes the file from `FileModel`,
    /// preventing the `FileState` entry from leaking.
    fn remove(
        &mut self,
        file_id: FileId,
        ctx: &mut ModelContext<ServerModel>,
    ) -> Option<PendingFileOp> {
        let op = self.ops.remove(&file_id)?;
        FileModel::handle(ctx).update(ctx, |m, ctx| m.unsubscribe(file_id, ctx));
        Some(op)
    }
}

/// The top-level server-side orchestrator model.
///
/// Receives `ClientMessage`s from connected proxy sessions and routes
/// `ServerMessage` responses and push notifications back through each
/// connection's dedicated sender channel.
pub struct ServerModel {
    /// Per-connection outbound channels, keyed by `ConnectionId`.
    ///
    /// The daemon can serve multiple proxy connections simultaneously — one
    /// per SSH session / Zaplex tab connecting to this host.  Each entry maps
    /// a connection's `Uuid` to the channel the connection task drains to
    /// write `ServerMessage`s back to its proxy.
    connection_senders: HashMap<ConnectionId, async_channel::Sender<ServerMessage>>,
    /// Capabilities advertised by each connected client during Initialize.
    connection_features: HashMap<ConnectionId, HashSet<String>>,
    /// Per-connection set of repo roots for which we've already sent a
    /// snapshot in this connection's lifetime.
    ///
    /// Used to avoid sending duplicate snapshots on repeated
    /// `NavigatedToDirectory` calls while the user `cd`s within the same repo.
    snapshot_sent_roots_by_connection: HashMap<ConnectionId, HashSet<StandardizedPath>>,
    /// Abort handle for the active grace timer, if any.
    /// Calling `.abort()` cancels the timer before it fires.
    grace_timer_cancel: Option<SpawnedFutureHandle>,
    /// Tracks in-progress requests that can be cancelled via `Abort`.
    /// Calling `.abort()` on the handle cancels the background future and
    /// triggers its `on_abort` callback.
    in_progress: HashMap<RequestId, SpawnedFutureHandle>,
    /// Stable host identifier generated once at process startup.
    /// Returned in every `InitializeResponse` so clients can deduplicate
    /// host-scoped models.
    host_id: String,
    /// Per-session command executors created from `SessionBootstrapped` notifications.
    executors: HashMap<SessionId, Arc<LocalCommandExecutor>>,
    /// Tracks in-flight file write/delete operations and handles cleanup.
    pending_file_ops: PendingFileOps,
    /// Tracks open server-local buffers, their connections, and pending
    /// buffer requests (OpenBuffer, SaveBuffer, ResolveConflict).
    #[cfg(feature = "local_fs")]
    buffers: ServerBufferTracker,
    /// Daemon-wide bearer credential for the identity-scoped daemon.
    ///
    /// The token is written by Initialize when the client supplies a
    /// non-empty credential, or by Authenticate during token rotation. It is
    /// intentionally retained across proxy connection teardown and cleared
    /// only by daemon process exit.
    auth_token: Option<String>,
    /// Live daemon-hosted terminal sessions, keyed by their daemon-assigned id.
    #[cfg(unix)]
    sessions: HashMap<String, super::session_host::Session>,
    /// Generation-checked agent associations for live daemon PTYs.
    #[cfg(unix)]
    agent_pty_bindings: AgentPtyBindings,
    /// Monotonic generation assigned to the next PTY opened by this daemon.
    #[cfg(unix)]
    next_pty_generation: u64,
    /// Descriptor-bound file handles and durable mutation journal used by the
    /// safe file-manager transfer protocol.
    #[cfg(unix)]
    safe_files: super::safe_file::SafeFileServer,
    /// Bounded transcript parse cache shared by all agent-inventory requests.
    /// The scan itself stays off the model thread; the mutex only serializes
    /// concurrent readers of the process-local cache.
    agent_transcript_cache: Arc<Mutex<zaplex_cockpit::TranscriptScanCache>>,
    /// Opaque account ids from the latest daemon-local inventory mapped to
    /// provider config roots that never cross the wire.
    agent_account_routes: super::agent_account::AccountRouteCache,
    #[cfg(test)]
    fresh_agent_account_routes_for_test: Option<super::agent_account::AccountRoutes>,
    /// Daemon-wide cap for transcript filesystem scans and parsing. The permit
    /// is owned by the abortable future, so cancellation releases it as well.
    agent_transcript_reads_in_flight: Arc<AtomicUsize>,
    /// Global daemon cap for procfs-backed managed memory inventory work.
    /// Busy callers get typed unavailable measurements instead of starting a
    /// requests-times-sessions fan-out.
    #[cfg(unix)]
    managed_memory_reads_in_flight: Arc<AtomicUsize>,
    /// Bounded, time-limited explanations and exact restart identities for
    /// managed sessions that ended or were explicitly stopped.
    #[cfg(unix)]
    recent_managed_exits: VecDeque<ManagedExitRecord>,
    /// Parsed once at daemon startup. Invalid configuration keeps the managed
    /// start gate closed without weakening ordinary terminal sessions.
    #[cfg(unix)]
    managed_min_available_bytes: Result<u64, super::managed_fleet::FleetValidationError>,
}

impl Entity for ServerModel {
    type Event = ();
}

impl SingletonEntity for ServerModel {}

impl ServerModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let host_id = uuid::Uuid::new_v4().to_string();
        #[cfg(unix)]
        let configured_managed_headroom = std::env::var("ZAPLEX_MANAGED_MIN_AVAILABLE_MB").ok();
        log::info!(
            "Daemon started: PID={}, host_id={}",
            std::process::id(),
            host_id
        );
        let mut model = Self {
            connection_senders: HashMap::new(),
            connection_features: HashMap::new(),
            snapshot_sent_roots_by_connection: HashMap::new(),
            grace_timer_cancel: None,
            in_progress: HashMap::new(),
            host_id,
            executors: HashMap::new(),
            pending_file_ops: PendingFileOps::new(),
            #[cfg(feature = "local_fs")]
            buffers: ServerBufferTracker::new(),
            auth_token: None,
            #[cfg(unix)]
            sessions: HashMap::new(),
            #[cfg(unix)]
            agent_pty_bindings: AgentPtyBindings::default(),
            #[cfg(unix)]
            next_pty_generation: 1,
            #[cfg(unix)]
            safe_files: super::safe_file::SafeFileServer::new(),
            agent_transcript_cache: Arc::new(Mutex::new(
                zaplex_cockpit::TranscriptScanCache::default(),
            )),
            agent_account_routes: Default::default(),
            #[cfg(test)]
            fresh_agent_account_routes_for_test: None,
            agent_transcript_reads_in_flight: Arc::new(AtomicUsize::new(0)),
            #[cfg(unix)]
            managed_memory_reads_in_flight: Arc::new(AtomicUsize::new(0)),
            #[cfg(unix)]
            recent_managed_exits: VecDeque::new(),
            #[cfg(unix)]
            managed_min_available_bytes: super::managed_fleet::daemon_headroom_floor_bytes(
                configured_managed_headroom.as_deref(),
            ),
        };
        // Subscribe to FileModel and RepoMetadataModel events
        // file operation results and repo metadata pushes are forwarded to all
        // connected proxy sessions.
        {
            let file_model = FileModel::handle(ctx);
            ctx.subscribe_to_model(&file_model, |me, event, ctx| {
                let file_id = event.file_id();
                let Some(pending_kind) = me.pending_file_ops.get(&file_id).map(|op| &op.kind)
                else {
                    return; // Not a file op we're tracking.
                };
                let response_message = match (event, pending_kind) {
                    (FileModelEvent::FileSaved { .. }, FileOpKind::Write) => {
                        server_message::Message::WriteFileResponse(WriteFileResponse {
                            result: Some(write_file_response::Result::Success(WriteFileSuccess {})),
                        })
                    }
                    (FileModelEvent::FileSaved { .. }, FileOpKind::Delete) => {
                        server_message::Message::DeleteFileResponse(DeleteFileResponse {
                            result: Some(delete_file_response::Result::Success(
                                DeleteFileSuccess {},
                            )),
                        })
                    }
                    (FileModelEvent::FailedToSave { error, .. }, FileOpKind::Write) => {
                        server_message::Message::WriteFileResponse(WriteFileResponse {
                            result: Some(write_file_response::Result::Error(FileOperationError {
                                message: format!("{error}"),
                            })),
                        })
                    }
                    (FileModelEvent::FailedToSave { error, .. }, FileOpKind::Delete) => {
                        server_message::Message::DeleteFileResponse(DeleteFileResponse {
                            result: Some(delete_file_response::Result::Error(FileOperationError {
                                message: format!("{error}"),
                            })),
                        })
                    }
                    (FileModelEvent::FileLoaded { .. }, _)
                    | (FileModelEvent::FailedToLoad { .. }, _)
                    | (FileModelEvent::FileUpdated { .. }, _) => return,
                };
                // Remove the pending op and unsubscribe from FileModel.
                let pending = me
                    .pending_file_ops
                    .remove(file_id, ctx)
                    .expect("pending op was confirmed present");
                me.send_server_message(
                    Some(pending.conn_id),
                    Some(&pending.request_id),
                    response_message,
                );
            });
        }
        {
            let repo_model = RepoMetadataModel::handle(ctx);
            ctx.subscribe_to_model(&repo_model, |me, event, ctx| match event {
                RepoMetadataEvent::IncrementalUpdateReady { update } => {
                    me.send_server_message(
                        None,
                        None,
                        server_message::Message::RepoMetadataUpdate(update.into()),
                    );
                }
                RepoMetadataEvent::RepositoryUpdated {
                    id: RepositoryIdentifier::Local(path),
                } => {
                    // A repo finished indexing — push the full tree as a snapshot.
                    let id = RepositoryIdentifier::local(path.clone());
                    let repo_model = RepoMetadataModel::handle(ctx);
                    if let Some(state) = repo_model.as_ref(ctx).get_repository(&id, ctx) {
                        let entries = super::repo_metadata_proto::file_tree_entry_to_snapshot_proto(
                            &state.entry,
                        );
                        me.send_server_message(
                            None,
                            None,
                            server_message::Message::RepoMetadataSnapshot(
                                super::proto::RepoMetadataSnapshot {
                                    repo_path: path.to_string(),
                                    entries,
                                    sync_complete: true,
                                },
                            ),
                        );
                        // Mark this root as snapshot-sent for all active connections
                        // so subsequent NavigatedToDirectory calls skip re-sending.
                        for sent_roots in me.snapshot_sent_roots_by_connection.values_mut() {
                            sent_roots.insert(path.clone());
                        }
                    }
                }
                RepoMetadataEvent::RepositoryRemoved { .. }
                | RepoMetadataEvent::FileTreeUpdated { .. }
                | RepoMetadataEvent::FileTreeEntryUpdated { .. }
                | RepoMetadataEvent::UpdatingRepositoryFailed { .. }
                | RepoMetadataEvent::RepositoryUpdated {
                    id: RepositoryIdentifier::Remote(_),
                } => {}
            });
        }
        // Subscribe to GlobalBufferModel events for server-local buffers.
        #[cfg(feature = "local_fs")]
        {
            let gbm = GlobalBufferModel::handle(ctx);
            ctx.subscribe_to_model(&gbm, |me, event, ctx| match event {
                GlobalBufferModelEvent::BufferLoaded { file_id, .. } => {
                    // Complete all pending OpenBuffer requests for this file.
                    let pending = me
                        .buffers
                        .take_pending_by_kind(file_id, PendingBufferRequestKind::OpenBuffer);
                    if !pending.is_empty() {
                        let gbm = GlobalBufferModel::handle(ctx);
                        let content = gbm.as_ref(ctx).content_for_file(*file_id, ctx);
                        let server_version = gbm
                            .as_ref(ctx)
                            .sync_clock_for_server_local(*file_id)
                            .map(|c| c.server_version.as_u64());

                        for (request_id, conn_id) in pending {
                            let message = match (&content, server_version) {
                                (Some(content), Some(sv)) => {
                                    server_message::Message::OpenBufferResponse(
                                        OpenBufferResponse {
                                            content: content.clone(),
                                            server_version: sv,
                                        },
                                    )
                                }
                                _ => server_message::Message::Error(ErrorResponse {
                                    code: ErrorCode::Internal.into(),
                                    message: format!(
                                        "Buffer loaded but content or sync clock unavailable for file {file_id:?}"
                                    ),
                                }),
                            };
                            me.send_server_message(Some(conn_id), Some(&request_id), message);
                        }
                    }
                }
                GlobalBufferModelEvent::ServerLocalBufferUpdated {
                    file_id,
                    edits,
                    new_server_version,
                    expected_client_version,
                } => {
                    // Push incremental edits to all connections that have this buffer open.
                    let Some(conns) = me.buffers.connections_for_buffer(file_id) else {
                        return;
                    };
                    // Find the path for this file_id; abort the push if tracker
                    // state is inconsistent (empty path would violate path↔buffer contract).
                    let Some(path) = me.buffers.path_for_file_id(*file_id) else {
                        log::error!(
                            "Missing path mapping for server-local buffer file_id={file_id:?}"
                        );
                        return;
                    };

                    let proto_edits: Vec<TextEdit> = edits
                        .iter()
                        .map(|edit| TextEdit {
                            start_offset: edit.start.as_usize() as u64,
                            end_offset: edit.end.as_usize() as u64,
                            text: edit.text.clone(),
                        })
                        .collect();

                    let conns: Vec<_> = conns.iter().copied().collect();
                    for conn_id in conns {
                        me.send_server_message(
                            Some(conn_id),
                            None,
                            server_message::Message::BufferUpdated(BufferUpdatedPush {
                                path: path.clone(),
                                new_server_version: new_server_version.as_u64(),
                                expected_client_version: expected_client_version.as_u64(),
                                edits: proto_edits.clone(),
                            }),
                        );
                    }
                }
                GlobalBufferModelEvent::FileSaved { file_id } => {
                    for (request_id, conn_id) in me
                        .buffers
                        .take_pending_by_kind(file_id, PendingBufferRequestKind::SaveBuffer)
                    {
                        me.send_server_message(
                            Some(conn_id),
                            Some(&request_id),
                            server_message::Message::SaveBufferResponse(SaveBufferResponse {
                                result: Some(save_buffer_response::Result::Success(
                                    SaveBufferSuccess {},
                                )),
                            }),
                        );
                    }
                    for (request_id, conn_id) in me
                        .buffers
                        .take_pending_by_kind(file_id, PendingBufferRequestKind::ResolveConflict)
                    {
                        me.send_server_message(
                            Some(conn_id),
                            Some(&request_id),
                            server_message::Message::ResolveConflictResponse(
                                ResolveConflictResponse {
                                    result: Some(resolve_conflict_response::Result::Success(
                                        ResolveConflictSuccess {},
                                    )),
                                },
                            ),
                        );
                    }
                }
                GlobalBufferModelEvent::FailedToSave { file_id, error } => {
                    for (request_id, conn_id) in me
                        .buffers
                        .take_pending_by_kind(file_id, PendingBufferRequestKind::SaveBuffer)
                    {
                        me.send_server_message(
                            Some(conn_id),
                            Some(&request_id),
                            server_message::Message::SaveBufferResponse(SaveBufferResponse {
                                result: Some(save_buffer_response::Result::Error(
                                    FileOperationError {
                                        message: format!("{error}"),
                                    },
                                )),
                            }),
                        );
                    }
                    for (request_id, conn_id) in me
                        .buffers
                        .take_pending_by_kind(file_id, PendingBufferRequestKind::ResolveConflict)
                    {
                        me.send_server_message(
                            Some(conn_id),
                            Some(&request_id),
                            server_message::Message::ResolveConflictResponse(
                                ResolveConflictResponse {
                                    result: Some(resolve_conflict_response::Result::Error(
                                        FileOperationError {
                                            message: format!("{error}"),
                                        },
                                    )),
                                },
                            ),
                        );
                    }
                }
                GlobalBufferModelEvent::FailedToLoad { file_id, error } => {
                    for (request_id, conn_id) in me
                        .buffers
                        .take_pending_by_kind(file_id, PendingBufferRequestKind::OpenBuffer)
                    {
                        me.send_server_message(
                            Some(conn_id),
                            Some(&request_id),
                            server_message::Message::Error(ErrorResponse {
                                code: ErrorCode::Internal.into(),
                                message: format!("Failed to load buffer: {error}"),
                            }),
                        );
                    }
                }
                GlobalBufferModelEvent::BufferUpdatedFromFileEvent { .. }
                | GlobalBufferModelEvent::RemoteBufferConflict { .. } => {
                    // Not relevant for server-local buffers.
                }
            });
        }
        // Start the grace timer immediately so the daemon exits if no proxy
        // connects within GRACE_PERIOD. In practice the spawning proxy connects
        // within milliseconds, so the risk of premature shutdown is negligible;
        // register_connection will cancel the timer the moment the first proxy
        // arrives.
        model.start_grace_timer(ctx);
        // Periodic memory governor for detached sessions (Stage 4).
        #[cfg(unix)]
        model.start_gc_timer(ctx);
        model
    }

    /// Called when a proxy connects.  Inserts `conn_tx` into the connection
    /// map so `send_server_message` can route responses to this proxy, and
    /// cancels the grace timer if it was running.
    pub fn register_connection(
        &mut self,
        conn_id: ConnectionId,
        conn_tx: async_channel::Sender<ServerMessage>,
        ctx: &mut ModelContext<Self>,
    ) {
        log::info!(
            "Daemon: connection {conn_id} registered — {} active, host_id={}",
            self.connection_senders.len() + 1,
            self.host_id
        );
        if let Some(handle) = self.grace_timer_cancel.take() {
            handle.abort();
        }
        self.connection_senders.insert(conn_id, conn_tx);
        self.connection_features.insert(conn_id, HashSet::new());
        self.snapshot_sent_roots_by_connection
            .insert(conn_id, HashSet::new());
        ctx.notify();
    }

    /// Called when a proxy disconnects.  Removes it from the connection map
    /// and starts the grace timer if no connections remain.
    pub fn deregister_connection(&mut self, conn_id: ConnectionId, ctx: &mut ModelContext<Self>) {
        self.snapshot_sent_roots_by_connection.remove(&conn_id);
        self.connection_features.remove(&conn_id);
        // Guard against double-deregister (reader and writer tasks both call
        // this on connection close; the second call must be a safe no-op).
        if self.connection_senders.remove(&conn_id).is_none() {
            return;
        }
        #[cfg(unix)]
        self.safe_files.close_connection(conn_id);
        // Drop this connection from all open server-local buffers; orphaned
        // buffers (no remaining connections) are deallocated by the tracker.
        #[cfg(feature = "local_fs")]
        self.buffers.remove_connection(conn_id, ctx);
        let remaining = self.connection_senders.len();
        log::info!("Daemon: connection {conn_id} deregistered — {remaining} active remaining");
        if remaining == 0 {
            // Persistent sessions keep the daemon alive across client
            // disconnects (the whole point of the native session layer): only
            // arm the shutdown grace timer when nothing is still running.
            // (A detached-idle GC for long-abandoned sessions is Stage 4.)
            if self.has_live_sessions() {
                log::info!("Daemon: no connections, but live session(s) remain — staying up");
            } else {
                log::info!("Daemon: grace timer started ({GRACE_PERIOD:?})");
                self.start_grace_timer(ctx);
            }
        }
        ctx.notify();
    }

    /// Whether any daemon-hosted session is still alive. Keeps the daemon up
    /// across client disconnects so the session survives until reattach.
    fn has_live_sessions(&self) -> bool {
        #[cfg(unix)]
        {
            !self.sessions.is_empty()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    /// Starts (or restarts) a timer that shuts the daemon down after
    /// [`GRACE_PERIOD`] with no connected proxies.  If a timer is already
    /// running its abort handle is cancelled before the new one is stored.
    /// When a proxy connects, `register_connection` aborts the handle,
    /// preventing the shutdown.
    fn start_grace_timer(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(handle) = self.grace_timer_cancel.take() {
            handle.abort();
        }
        let handle = ctx.spawn_abortable(
            async_io::Timer::after(GRACE_PERIOD),
            |_, _, ctx| {
                log::info!("Daemon: grace period expired, shutting down");
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            },
            |_, _| {
                log::debug!("Daemon: grace timer cancelled");
            },
        );
        self.grace_timer_cancel = Some(handle);
    }

    /// Called by the background stdin reader task via `ModelSpawner`.
    ///
    /// Dispatches on the `oneof message` variant. Notifications are handled
    /// inline; request-style messages return a `HandlerOutcome` that is
    /// centrally acted on here: `Sync` responses are sent immediately and
    /// `Async` handles are tracked in `in_progress` so they can be aborted.
    pub fn handle_message(
        &mut self,
        conn_id: ConnectionId,
        msg: ClientMessage,
        ctx: &mut ModelContext<Self>,
    ) {
        let request_id = RequestId::from(msg.request_id);

        let outcome = match msg.message {
            Some(client_message::Message::Initialize(msg)) => {
                self.handle_initialize(conn_id, msg, &request_id)
            }
            Some(client_message::Message::Authenticate(msg)) => {
                self.handle_authenticate(msg);
                return;
            }
            Some(client_message::Message::SessionBootstrapped(msg)) => {
                self.handle_session_bootstrapped(msg);
                return;
            }
            Some(client_message::Message::Abort(abort)) => {
                self.handle_abort(abort, &request_id);
                return;
            }
            Some(client_message::Message::RunCommand(req)) => {
                self.handle_run_command(req, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::AgentProcessSignal(req)) => {
                self.handle_agent_process_signal(req, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::HostExec(req)) => {
                self.handle_host_exec(req, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::NavigatedToDirectory(msg)) => {
                self.handle_navigated_to_directory(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::LoadRepoMetadataDirectory(msg)) => {
                self.handle_load_repo_metadata_directory(msg, &request_id, ctx)
            }
            Some(client_message::Message::WriteFile(msg)) => {
                self.handle_write_file(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::DeleteFile(msg)) => {
                self.handle_delete_file(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::ReadFileContext(msg)) => {
                self.handle_read_file_context(msg, &request_id, conn_id, ctx)
            }
            #[cfg(feature = "local_fs")]
            Some(client_message::Message::OpenBuffer(msg)) => {
                self.handle_open_buffer(msg, &request_id, conn_id, ctx)
            }
            #[cfg(feature = "local_fs")]
            Some(client_message::Message::BufferEdit(msg)) => {
                self.handle_buffer_edit(msg, ctx);
                return; // fire-and-forget notification
            }
            #[cfg(feature = "local_fs")]
            Some(client_message::Message::CloseBuffer(msg)) => {
                self.handle_close_buffer(msg, conn_id, ctx);
                return; // fire-and-forget notification
            }
            #[cfg(feature = "local_fs")]
            Some(client_message::Message::SaveBuffer(msg)) => {
                self.handle_save_buffer(msg, &request_id, conn_id, ctx)
            }
            #[cfg(feature = "local_fs")]
            Some(client_message::Message::ResolveConflict(msg)) => {
                self.handle_resolve_conflict(msg, &request_id, conn_id, ctx)
            }
            // Zaplex: Remote terminal file link directory listing (for path form validation).
            #[cfg(feature = "local_fs")]
            Some(client_message::Message::ListDirectory(msg)) => self.handle_list_directory(msg),
            #[cfg(feature = "local_fs")]
            Some(client_message::Message::ResolvePath(msg)) => self.handle_resolve_path(msg),
            #[cfg(feature = "local_fs")]
            Some(client_message::Message::CreateDirectory(msg)) => {
                self.handle_create_directory(msg)
            }
            #[cfg(feature = "local_fs")]
            Some(client_message::Message::ReadFileChunk(msg)) => self.handle_read_file_chunk(msg),
            #[cfg(feature = "local_fs")]
            Some(client_message::Message::WriteFileChunk(msg)) => self.handle_write_file_chunk(msg),
            // zaplex native session host (see remote_server.proto, "Native
            // Remote Session Layer"). Stage 1 implements the per-session PTY
            // host on unix; attach/detach/list land in Stages 3-4.
            #[cfg(unix)]
            Some(client_message::Message::OpenSession(msg)) => {
                self.handle_open_session(&request_id, conn_id, msg, ctx)
            }
            #[cfg(unix)]
            Some(client_message::Message::SessionInput(msg)) => {
                match self.handle_session_input(conn_id, msg) {
                    Some(ack) => {
                        HandlerOutcome::Sync(server_message::Message::StartupCommandAck(ack))
                    }
                    None => return,
                }
            }
            #[cfg(unix)]
            Some(client_message::Message::ResizeSession(msg)) => {
                self.handle_resize_session(conn_id, msg);
                return;
            }
            #[cfg(unix)]
            Some(client_message::Message::CloseSession(msg)) => {
                self.handle_close_session(msg, ctx);
                return;
            }
            // Re-attach a reconnecting client to a still-running session and
            // replay the output it missed (Stage 3).
            #[cfg(unix)]
            Some(client_message::Message::AttachSession(msg)) => {
                self.handle_attach_session_request(&request_id, conn_id, msg, ctx)
            }
            #[cfg(unix)]
            Some(client_message::Message::DetachSession(msg)) => {
                self.handle_detach_session(conn_id, msg);
                return;
            }
            // The opening client reports where bootstrap completed so we can
            // freeze an eviction-proof preamble for later adopts (T1.3).
            #[cfg(unix)]
            Some(client_message::Message::SetBootstrapPreamble(msg)) => {
                self.handle_set_bootstrap_preamble(msg);
                return;
            }
            // No session host off unix → nothing to capture; drop the notification.
            #[cfg(not(unix))]
            Some(client_message::Message::SetBootstrapPreamble(_)) => return,
            #[cfg(unix)]
            Some(client_message::Message::BindAgentPty(msg)) => {
                self.handle_bind_agent_pty(&request_id, conn_id, msg, ctx)
            }
            #[cfg(unix)]
            Some(client_message::Message::UnbindAgentPty(msg)) => {
                self.handle_unbind_agent_pty(conn_id, msg)
            }
            #[cfg(unix)]
            Some(client_message::Message::SafeFile(request)) => {
                let is_close_notification = request_id.is_empty()
                    && matches!(
                        request.operation.as_ref(),
                        Some(super::proto::safe_file_request::Operation::CloseHandle(_))
                    );
                if self.client_supports_safe_file_transactions(conn_id)
                    && self.safe_files.is_available()
                {
                    let response = self.safe_files.handle(conn_id, request);
                    if is_close_notification {
                        if let Some(super::proto::safe_file_response::Result::Error(error)) =
                            response.result
                        {
                            log::warn!("Safe-file close notification failed: {}", error.message);
                        }
                        return;
                    }
                    HandlerOutcome::Sync(server_message::Message::SafeFileResponse(response))
                } else {
                    if is_close_notification {
                        log::debug!(
                            "Ignoring safe-file close notification without negotiated capability"
                        );
                        return;
                    }
                    HandlerOutcome::Sync(server_message::Message::SafeFileResponse(
                        super::proto::SafeFileResponse {
                            result: Some(super::proto::safe_file_response::Result::Error(
                                FileOperationError {
                                    message:
                                        "safe-file-transactions-v1 capability was not negotiated"
                                            .to_string(),
                                },
                            )),
                        },
                    ))
                }
            }
            #[cfg(not(unix))]
            Some(
                client_message::Message::BindAgentPty(_)
                | client_message::Message::UnbindAgentPty(_),
            ) => HandlerOutcome::Sync(server_message::Message::AgentPtyBindingResponse(
                AgentPtyBindingResponse {
                    status: AgentPtyBindingStatus::CapabilityRequired.into(),
                    message: "agent-pty-binding-v2 requires a unix daemon".to_string(),
                },
            )),
            #[cfg(not(unix))]
            Some(client_message::Message::SafeFile(_)) => HandlerOutcome::Sync(
                server_message::Message::SafeFileResponse(super::proto::SafeFileResponse {
                    result: Some(super::proto::safe_file_response::Result::Error(
                        FileOperationError {
                            message: "safe-file transactions require a supported unix daemon"
                                .to_string(),
                        },
                    )),
                }),
            ),
            // Agent-session inventory (Agent-Cockpit): report this host's
            // Claude/Codex agent-sessions discovered on the daemon's filesystem.
            // Not PTY-bound, so available on every platform.
            Some(client_message::Message::ListAgentSessions(_)) => {
                self.handle_list_agent_sessions(&request_id, conn_id, ctx)
            }
            // Secret-free provider-account inventory and opaque launch routes.
            Some(client_message::Message::ListAgentAccounts(_)) => {
                self.handle_list_agent_accounts(&request_id, conn_id, ctx)
            }
            Some(client_message::Message::ReadAgentTranscript(request)) => {
                self.handle_read_agent_transcript(&request_id, conn_id, request, ctx)
            }
            // Multi-session listing for the sidebar / adopt-by-id (Stage 4).
            #[cfg(unix)]
            Some(client_message::Message::ListSessions(_)) => {
                self.handle_list_sessions(&request_id, conn_id, ctx)
            }
            #[cfg(target_os = "linux")]
            Some(client_message::Message::ManagedSessionLifecycle(request)) => {
                self.handle_managed_session_lifecycle(&request_id, conn_id, request, ctx)
            }
            #[cfg(not(target_os = "linux"))]
            Some(client_message::Message::ManagedSessionLifecycle(request)) => {
                HandlerOutcome::Sync(server_message::Message::ManagedSessionLifecycleResponse(
                    ManagedSessionLifecycleResponse {
                        schema_version: 1,
                        action: request.action,
                        status: ManagedSessionLifecycleStatus::CapabilityRequired.into(),
                        session_id: request.session_id,
                        generation: request.expected_generation,
                        replacement_session_id: String::new(),
                        replacement_generation: 0,
                        diagnostic_code: "unsupported-platform".to_string(),
                    },
                ))
            }
            #[cfg(not(unix))]
            Some(client_message::Message::ListSessions(_)) => {
                HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: "zaplex session host requires a unix daemon".to_string(),
                }))
            }
            #[cfg(unix)]
            Some(client_message::Message::ListMultiplexerSessions(_)) => {
                if self.client_supports_multiplexer_inventory(conn_id) {
                    self.handle_list_multiplexer_sessions(&request_id, conn_id, ctx)
                } else {
                    HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                        code: ErrorCode::InvalidRequest.into(),
                        message: "multiplexer-inventory-v1 capability was not negotiated"
                            .to_string(),
                    }))
                }
            }
            #[cfg(not(unix))]
            Some(client_message::Message::ListMultiplexerSessions(_)) => {
                HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: "multiplexer inventory requires a unix daemon".to_string(),
                }))
            }
            // Non-unix daemons have no session host (PTY ownership is unix-only).
            #[cfg(not(unix))]
            Some(
                client_message::Message::AttachSession(_)
                | client_message::Message::DetachSession(_),
            ) => HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "zaplex session host requires a unix daemon".to_string(),
            })),
            // Non-unix daemons have no session host (PTY ownership is unix-only).
            #[cfg(not(unix))]
            Some(
                client_message::Message::OpenSession(_)
                | client_message::Message::SessionInput(_)
                | client_message::Message::ResizeSession(_)
                | client_message::Message::CloseSession(_),
            ) => HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "zaplex session host requires a unix daemon".to_string(),
            })),
            #[cfg(not(feature = "local_fs"))]
            Some(
                client_message::Message::OpenBuffer(_)
                | client_message::Message::BufferEdit(_)
                | client_message::Message::CloseBuffer(_)
                | client_message::Message::SaveBuffer(_)
                | client_message::Message::ResolveConflict(_)
                | client_message::Message::ListDirectory(_)
                | client_message::Message::ResolvePath(_)
                | client_message::Message::CreateDirectory(_)
                | client_message::Message::ReadFileChunk(_)
                | client_message::Message::WriteFileChunk(_),
            ) => HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "Buffer syncing requires the local_fs feature".to_string(),
            })),
            None => {
                log::warn!(
                    "Received ClientMessage with no message variant (request_id={request_id})"
                );
                HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: "ClientMessage had no message variant set".to_string(),
                }))
            }
        };

        match outcome {
            HandlerOutcome::Sync(message) => {
                self.send_server_message(Some(conn_id), Some(&request_id), message);
            }
            HandlerOutcome::Async(Some(handle)) => {
                self.in_progress.insert(request_id, handle);
            }
            HandlerOutcome::Async(None) => {
                // Async work tracked elsewhere (e.g. `pending_file_ops`);
                // the response will be sent via an event subscription.
            }
        }
    }

    /// Routes a server message to its destination.
    ///
    /// - `conn_id = Some(id)` — sends only to the connection that originated
    ///   the request (used for all request/response pairs).
    /// - `conn_id = None` — broadcasts to every connected proxy (used for
    ///   server-initiated push notifications such as repo metadata updates).
    fn send_server_message(
        &self,
        conn_id: Option<ConnectionId>,
        request_id: Option<&RequestId>,
        message: server_message::Message,
    ) {
        let msg = ServerMessage {
            request_id: request_id.map(|id| id.clone().into()).unwrap_or_default(),
            message: Some(message),
        };
        if let Some(target) = conn_id {
            if let Some(conn_tx) = self.connection_senders.get(&target) {
                if let Err(e) = conn_tx.try_send(msg) {
                    log::warn!("Daemon: failed to send to conn {target}: {e}");
                }
            } else {
                log::debug!("Daemon: no sender for conn {target} (already disconnected)");
            }
        } else {
            // Push notification — broadcast to all connections.
            for (id, conn_tx) in &self.connection_senders {
                if let Err(e) = conn_tx.try_send(msg.clone()) {
                    log::warn!("Daemon: failed to send to conn {id}: {e}");
                }
            }
        }
    }

    /// Spawns an abortable future tied to `request_id` and wires up automatic
    /// removal from `in_progress` on completion or abort.
    ///
    /// The returned handle is intended to be returned from a handler as
    /// `HandlerOutcome::Async(Some(handle))`; the caller (`handle_message`)
    /// inserts it into `in_progress`.
    fn spawn_request_handler<S, F>(
        &mut self,
        request_id: RequestId,
        future: S,
        on_resolve: F,
        ctx: &mut ModelContext<Self>,
    ) -> SpawnedFutureHandle
    where
        S: Spawnable,
        <S as Future>::Output: SpawnableOutput,
        F: 'static + FnOnce(&mut Self, <S as Future>::Output, &mut ModelContext<Self>),
    {
        let resolve_id = request_id.clone();
        let abort_id = request_id;
        ctx.spawn_abortable(
            future,
            move |me, output, ctx| {
                me.in_progress.remove(&resolve_id);
                on_resolve(me, output, ctx);
            },
            move |me, _ctx| {
                log::info!("Request cancelled (request_id={abort_id})");
                me.in_progress.remove(&abort_id);
            },
        )
    }

    /// Handles `Initialize` by returning the server version and host id.
    ///
    /// `server_version` is the release tag the daemon was built from
    /// (`GIT_RELEASE_TAG`) or the empty string for `cargo run` / locally
    /// deployed builds. The client treats an empty version as "unknown" and
    /// skips strict version enforcement, which keeps the
    /// `script/deploy_remote_server` developer workflow functional.
    fn handle_initialize(
        &mut self,
        conn_id: ConnectionId,
        msg: Initialize,
        request_id: &RequestId,
    ) -> HandlerOutcome {
        log::info!("Handling Initialize (request_id={request_id})");
        self.connection_features
            .insert(conn_id, msg.features.into_iter().collect());
        if !msg.auth_token.is_empty() {
            self.auth_token = Some(msg.auth_token);
        }
        let server_version = ChannelState::app_version().unwrap_or("").to_string();
        #[cfg(unix)]
        let safe_file_transactions_supported = self.safe_files.is_available();
        #[cfg(not(unix))]
        let safe_file_transactions_supported = false;
        let features = server_features_with_runtime_support(
            zaplex_cockpit::local_process_signalling_supported(),
            safe_file_transactions_supported,
        );
        HandlerOutcome::Sync(server_message::Message::InitializeResponse(
            InitializeResponse {
                server_version,
                host_id: self.host_id.clone(),
                // Capabilities this daemon advertises. Stage 0 is empty (the
                // native session host is not implemented yet); Stage 1 adds
                // FEATURE_SESSION_HOST via supported_features().
                features,
            },
        ))
    }

    fn client_supports_agent_pty_binding(&self, conn_id: ConnectionId) -> bool {
        self.connection_features
            .get(&conn_id)
            .is_some_and(|features| features.contains(FEATURE_AGENT_PTY_BINDING_V2))
    }

    fn client_supports_agent_account_routing(&self, conn_id: ConnectionId) -> bool {
        self.connection_features
            .get(&conn_id)
            .is_some_and(|features| features.contains(FEATURE_AGENT_ACCOUNT_ROUTING_V1))
    }

    fn client_supports_agent_transcript_read(&self, conn_id: ConnectionId) -> bool {
        self.connection_features
            .get(&conn_id)
            .is_some_and(|features| features.contains(FEATURE_AGENT_TRANSCRIPT_READ_V1))
    }

    fn client_supports_agent_process_signal(&self, conn_id: ConnectionId) -> bool {
        zaplex_cockpit::local_process_signalling_supported()
            && self
                .connection_features
                .get(&conn_id)
                .is_some_and(|features| features.contains(FEATURE_AGENT_PROCESS_SIGNAL_V1))
    }

    #[cfg(unix)]
    fn client_supports_managed_fleet(&self, conn_id: ConnectionId) -> bool {
        self.client_supports_managed_fleet_with_runtime(
            conn_id,
            zaplex_cockpit::local_process_signalling_supported(),
        )
    }

    #[cfg(unix)]
    fn client_supports_managed_fleet_with_runtime(
        &self,
        conn_id: ConnectionId,
        process_signalling_supported: bool,
    ) -> bool {
        cfg!(target_os = "linux")
            && process_signalling_supported
            && self
                .connection_features
                .get(&conn_id)
                .is_some_and(|features| features.contains(FEATURE_MANAGED_AGENT_FLEET_V1))
    }

    #[cfg(unix)]
    fn client_supports_multiplexer_inventory(&self, conn_id: ConnectionId) -> bool {
        self.connection_features
            .get(&conn_id)
            .is_some_and(|features| features.contains(FEATURE_MULTIPLEXER_INVENTORY_V1))
    }

    #[cfg(unix)]
    fn client_supports_safe_file_transactions(&self, conn_id: ConnectionId) -> bool {
        self.connection_features
            .get(&conn_id)
            .is_some_and(|features| features.contains(FEATURE_SAFE_FILE_TRANSACTIONS_V1))
    }

    /// Handles `Authenticate` by replacing the daemon-wide credential.
    /// This is a notification — no response is sent.
    fn handle_authenticate(&mut self, msg: Authenticate) {
        if msg.auth_token.is_empty() {
            log::warn!("Received Authenticate notification with empty auth token; ignoring");
            return;
        }
        self.auth_token = Some(msg.auth_token);
    }

    pub fn auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    /// Handles `Abort` by cancelling the in-progress request it targets.
    /// This is a notification — no response is sent.
    fn handle_abort(&mut self, abort: Abort, request_id: &RequestId) {
        let target_id = RequestId::from(abort.request_id_to_abort);
        if let Some(handle) = self.in_progress.remove(&target_id) {
            log::info!(
                "Aborting in-progress request (request_id={target_id}, \
                 abort_request_id={request_id})"
            );
            handle.abort();
        } else {
            log::info!(
                "Abort for unknown/completed request (request_id={target_id}, \
                 abort_request_id={request_id})"
            );
        }
    }

    /// Handles `SessionBootstrapped` by creating a `LocalCommandExecutor` for
    /// the session. This is a notification — no response is sent.
    fn handle_session_bootstrapped(&mut self, msg: SessionBootstrapped) {
        let session_id = SessionId::from(msg.session_id);
        log::info!(
            "Handling SessionBootstrapped: session_id={session_id:?}, \
             shell_type={:?}, shell_path={:?}",
            msg.shell_type,
            msg.shell_path,
        );

        let Some(shell_type) = ShellType::from_name(&msg.shell_type) else {
            log::error!(
                "Unknown shell_type {:?} in SessionBootstrapped for session {session_id:?}",
                msg.shell_type,
            );
            return;
        };

        let shell_path = msg.shell_path.map(PathBuf::from);
        if shell_path.is_none() {
            log::warn!(
                "SessionBootstrapped for session {session_id:?} had no shell_path; \
                 LocalCommandExecutor will fall back to bare shell name",
            );
        }
        let executor = Arc::new(LocalCommandExecutor::new(shell_path, shell_type));
        if self.executors.insert(session_id, executor).is_some() {
            log::warn!(
                "Overwriting existing executor for session {session_id:?} \
                 (re-SessionBootstrapped with shell_type={:?})",
                msg.shell_type,
            );
        }
    }

    /// Handles `RunCommand` by delegating to the session's `LocalCommandExecutor`.
    ///
    /// On success, returns a `HandlerOutcome::Async` whose task resolves the
    /// request with a `RunCommandResponse`. On validation failure (missing
    /// executor), returns a `HandlerOutcome::Sync` error response.
    fn handle_run_command(
        &mut self,
        req: RunCommandRequest,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        let session_id = SessionId::from(req.session_id);
        log::info!(
            "Handling RunCommand (request_id={request_id}, session_id={session_id:?}): \
             command={:?}, cwd={:?}",
            req.command,
            req.working_directory,
        );

        let command = req.command;
        let cwd = req.working_directory;
        let env_vars = if req.environment_variables.is_empty() {
            None
        } else {
            Some(req.environment_variables)
        };

        let Some(executor) = self.executors.get(&session_id).cloned() else {
            log::error!("No executor for session {session_id:?}, session was never initialized");
            return HandlerOutcome::Sync(server_message::Message::RunCommandResponse(
                RunCommandResponse {
                    result: Some(run_command_response::Result::Error(RunCommandError {
                        code: RunCommandErrorCode::SessionNotFound.into(),
                        message: format!("No executor for session {session_id:?}"),
                    })),
                },
            ));
        };

        // Call `execute_local_command` directly because the
        // `CommandExecutor::execute_command` trait method requires
        // a `&Shell` (version, options, plugins from bootstrap).
        let request_id_for_response = request_id.clone();
        let conn_id_for_response = conn_id;
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                executor
                    .execute_local_command(
                        &command,
                        cwd.as_deref(),
                        env_vars,
                        ExecuteCommandOptions::default(),
                    )
                    .await
            },
            move |me, result, _ctx| {
                let result_oneof = match result {
                    Ok(output) => {
                        log::info!(
                            "RunCommand completed (request_id={request_id_for_response}): \
                             exit_code={:?}, stdout_len={}, stderr_len={}",
                            output.exit_code,
                            output.stdout.len(),
                            output.stderr.len(),
                        );
                        run_command_response::Result::Success(RunCommandSuccess {
                            stdout: output.stdout.clone(),
                            stderr: output.stderr.clone(),
                            exit_code: output.exit_code.map(|c| c.value()),
                        })
                    }
                    Err(e) => {
                        log::warn!("RunCommand failed (request_id={request_id_for_response}): {e}");
                        run_command_response::Result::Error(RunCommandError {
                            code: RunCommandErrorCode::ExecutionFailed.into(),
                            message: format!("Failed to execute command: {e}"),
                        })
                    }
                };
                me.send_server_message(
                    Some(conn_id_for_response),
                    Some(&request_id_for_response),
                    server_message::Message::RunCommandResponse(RunCommandResponse {
                        result: Some(result_oneof),
                    }),
                );
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    /// Handles the narrow Agent-Cockpit process-signal RPC.
    ///
    /// The negotiated capability and a fresh daemon-local agent inventory must
    /// bind the requested session id, pid, and fingerprint before dispatch.
    /// No command text reaches a shell or `HostExec`: the daemon then delegates
    /// only to [`zaplex_cockpit::send_verified_process_signal`], which re-verifies
    /// the process identity immediately before signalling.
    fn handle_agent_process_signal(
        &mut self,
        req: AgentProcessSignalRequest,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        let capability_negotiated = self.client_supports_agent_process_signal(conn_id);
        if !capability_negotiated {
            return HandlerOutcome::Sync(server_message::Message::AgentProcessSignalResponse(
                execute_agent_process_signal_with(req, &[], false, |_, _, _| {
                    unreachable!("capability rejection cannot dispatch a process signal")
                }),
            ));
        }

        let request_id_for_response = request_id.clone();
        let transcript_cache = Arc::clone(&self.agent_transcript_cache);
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                let mut cache = transcript_cache.lock().unwrap_or_else(|poisoned| {
                    log::warn!(
                        "Daemon: agent transcript cache mutex was poisoned; recovering its state"
                    );
                    poisoned.into_inner()
                });
                let current_sessions = collect_agent_sessions(&mut cache);
                drop(cache);
                execute_agent_process_signal_with(
                    req,
                    &current_sessions,
                    true,
                    zaplex_cockpit::send_verified_process_signal,
                )
            },
            move |me, response, _ctx| {
                me.send_server_message(
                    Some(conn_id),
                    Some(&request_id_for_response),
                    server_message::Message::AgentProcessSignalResponse(response),
                );
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    /// Handles `HostExec` — a **session-less** one-shot host command. Unlike
    /// [`Self::handle_run_command`], it does not look up a per-session
    /// `LocalCommandExecutor`; it builds an ad-hoc executor over the daemon's
    /// default user shell (`$SHELL`, falling back to `/bin/bash`) rooted at the
    /// daemon's home directory and runs the command there. Agent process
    /// guardrails must use [`Self::handle_agent_process_signal`] instead.
    ///
    /// On success returns a `HandlerOutcome::Async` resolving a `HostExecResult`;
    /// a failure to even spawn the command surfaces as a top-level
    /// `ErrorResponse` (mirrors the other async handlers) — never a silent
    /// no-op.
    fn handle_host_exec(
        &mut self,
        req: HostExec,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling HostExec (request_id={request_id}): command={:?}",
            req.command,
        );

        // Resolve the daemon's default user shell — no bootstrapped session
        // required. `LocalCommandExecutor::from_name` / bare fallback mirror how
        // `handle_open_session` picks the host shell.
        let shell_path = std::env::var("SHELL").ok().filter(|s| !s.is_empty());
        let shell_type = shell_path
            .as_deref()
            .and_then(ShellType::from_name)
            .unwrap_or(ShellType::Bash);
        // Root the command at the daemon's home so relative paths resolve
        // sensibly; a guardrail `kill` is cwd-independent, but this keeps the
        // path honest for any other session-less use.
        let cwd = dirs::home_dir().map(|p| p.to_string_lossy().into_owned());
        let executor = LocalCommandExecutor::new(shell_path.map(PathBuf::from), shell_type);

        let command = req.command;
        let request_id_for_response = request_id.clone();
        let conn_id_for_response = conn_id;
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                executor
                    .execute_local_command(
                        &command,
                        cwd.as_deref(),
                        None,
                        ExecuteCommandOptions::default(),
                    )
                    .await
            },
            move |me, result, _ctx| {
                let message = match result {
                    Ok(output) => {
                        log::info!(
                            "HostExec completed (request_id={request_id_for_response}): \
                             exit_code={:?}, stdout_len={}, stderr_len={}",
                            output.exit_code,
                            output.stdout.len(),
                            output.stderr.len(),
                        );
                        server_message::Message::HostExecResult(HostExecResult {
                            stdout: output.stdout.clone(),
                            stderr: output.stderr.clone(),
                            exit_code: output.exit_code.map(|c| c.value()),
                        })
                    }
                    Err(e) => {
                        log::warn!("HostExec failed (request_id={request_id_for_response}): {e}");
                        server_message::Message::Error(ErrorResponse {
                            code: ErrorCode::Internal.into(),
                            message: format!("Failed to execute host command: {e}"),
                        })
                    }
                };
                me.send_server_message(
                    Some(conn_id_for_response),
                    Some(&request_id_for_response),
                    message,
                );
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    /// Handles `NavigatedToDirectory` by running git detection first, then
    /// responding. On validation failure returns a `HandlerOutcome::Sync` error;
    /// otherwise spawns a task and returns a `HandlerOutcome::Async(Some(_))`
    /// handle.
    fn handle_navigated_to_directory(
        &mut self,
        msg: NavigatedToDirectory,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling NavigatedToDirectory path={} (request_id={request_id})",
            msg.path
        );

        let std_path = match StandardizedPath::from_local_canonicalized(Path::new(&msg.path)) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("Invalid path for NavigatedToDirectory: {e}");
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: format!("Invalid path: {e}"),
                }));
            }
        };

        // Kick off git detection. The returned future resolves with the git
        // root path (Some) or None if no git repo was found.
        let path_str = msg.path.clone();
        let git_future = DetectedRepositories::handle(ctx).update(ctx, |repos, ctx| {
            repos.detect_possible_git_repo(&path_str, RepoDetectionSource::TerminalNavigation, ctx)
        });

        let request_id_for_response = request_id.clone();
        let conn_id_for_response = conn_id;
        let handle = self.spawn_request_handler(
            request_id.clone(),
            git_future,
            move |me, git_root, ctx| {
                let (indexed_path, is_git) = if let Some(root) = git_root {
                    // Git repo found. Full indexing was already triggered by
                    // DetectedGitRepo → LocalRepoMetadataModel. The client
                    // waits for RepositoryIndexedPush before FetchFileTree.
                    let root_str = root.to_string_lossy().to_string();
                    log::info!("Git repo detected at {root_str} for path {}", std_path);
                    (root_str, true)
                } else {
                    // No git repo. Lazy-load the directory for first-level data,
                    // then push the snapshot immediately.
                    RepoMetadataModel::handle(ctx).update(ctx, |repo_model, ctx| {
                        if let Err(e) = repo_model.index_lazy_loaded_path(&std_path, ctx) {
                            log::warn!("Failed to lazy-load directory {std_path}: {e}");
                        }
                    });
                    (std_path.to_string(), false)
                };

                me.send_server_message(
                    Some(conn_id_for_response),
                    Some(&request_id_for_response),
                    server_message::Message::NavigatedToDirectoryResponse(
                        NavigatedToDirectoryResponse {
                            indexed_path: indexed_path.clone(),
                            is_git,
                        },
                    ),
                );

                // After responding, push a snapshot if metadata is available.
                //
                // For git repos this is an opportunistic push for the case
                // where the repo was already indexed and RepositoryUpdated
                // won't fire again (which would otherwise leave the client
                // with only a placeholder root). We skip if a snapshot was
                // already sent for this connection+root.
                //
                // For non-git directories the lazy-loaded tree is always
                // broadcast to all connections.
                if let Ok(root_path) =
                    StandardizedPath::from_local_canonicalized(Path::new(&indexed_path))
                {
                    if is_git {
                        let already_sent = me
                            .snapshot_sent_roots_by_connection
                            .get(&conn_id_for_response)
                            .is_some_and(|roots| roots.contains(&root_path));
                        if already_sent {
                            log::debug!(
                                "Snapshot already sent for repo {indexed_path} \
                                 to conn {conn_id_for_response}, skipping"
                            );
                            return;
                        }
                    }

                    let id = RepositoryIdentifier::local(root_path.clone());
                    let repo_model = RepoMetadataModel::handle(ctx);
                    if let Some(state) = repo_model.as_ref(ctx).get_repository(&id, ctx) {
                        let entries = super::repo_metadata_proto::file_tree_entry_to_snapshot_proto(
                            &state.entry,
                        );
                        // Git snapshots target the requesting connection;
                        // non-git snapshots broadcast to all.
                        let target = if is_git {
                            Some(conn_id_for_response)
                        } else {
                            None
                        };
                        me.send_server_message(
                            target,
                            None,
                            server_message::Message::RepoMetadataSnapshot(
                                super::proto::RepoMetadataSnapshot {
                                    repo_path: indexed_path,
                                    entries,
                                    sync_complete: true,
                                },
                            ),
                        );
                        if is_git {
                            if let Some(sent_roots) = me
                                .snapshot_sent_roots_by_connection
                                .get_mut(&conn_id_for_response)
                            {
                                sent_roots.insert(root_path);
                            }
                        }
                    }
                }
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    /// Handles `LoadRepoMetadataDirectory` by loading a subdirectory on the
    /// server's local model and returning the children synchronously.
    fn handle_load_repo_metadata_directory(
        &mut self,
        msg: super::proto::LoadRepoMetadataDirectory,
        request_id: &RequestId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling LoadRepoMetadataDirectory repo_path={} dir_path={} (request_id={request_id})",
            msg.repo_path,
            msg.dir_path
        );

        let repo_path = match StandardizedPath::from_local_canonicalized(Path::new(&msg.repo_path))
        {
            Ok(p) => p,
            Err(e) => {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: format!("Invalid repo_path: {e}"),
                }));
            }
        };

        let dir_path = match StandardizedPath::from_local_canonicalized(Path::new(&msg.dir_path)) {
            Ok(p) => p,
            Err(e) => {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: format!("Invalid dir_path: {e}"),
                }));
            }
        };

        // Validate that the directory is a descendant of the repo.
        if !dir_path.starts_with(&repo_path) {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: format!(
                    "dir_path {dir_path} is not a descendant of repo_path {repo_path}"
                ),
            }));
        }

        // Load the directory on the server's local model.
        let load_result = RepoMetadataModel::handle(ctx).update(ctx, |model, ctx| {
            model.load_directory(&repo_path, &dir_path, ctx)
        });

        if let Err(e) = load_result {
            log::warn!("LoadRepoMetadataDirectory failed: {e}");
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::Internal.into(),
                message: format!("Failed to load directory: {e}"),
            }));
        }

        // Read back the loaded children and serialize them.
        let id = RepositoryIdentifier::local(repo_path.clone());
        let entries = RepoMetadataModel::handle(ctx)
            .as_ref(ctx)
            .get_repository(&id, ctx)
            .map(|state| {
                super::repo_metadata_proto::file_tree_children_to_proto_entries(
                    &state.entry,
                    &dir_path,
                )
            })
            .unwrap_or_default();

        HandlerOutcome::Sync(server_message::Message::LoadRepoMetadataDirectoryResponse(
            super::proto::LoadRepoMetadataDirectoryResponse {
                repo_path: msg.repo_path,
                dir_path: msg.dir_path,
                entries,
            },
        ))
    }

    /// Handles `WriteFile` by registering the path and triggering an async
    /// write via `FileModel`. On a successful dispatch, returns
    /// `HandlerOutcome::Async(None)` — the response is sent later by the
    /// `FileModel` event subscription, and the op is not cancellable via
    /// `Abort`. On failure to dispatch, returns a `HandlerOutcome::Sync`
    /// error response.
    fn handle_write_file(
        &mut self,
        msg: WriteFile,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling WriteFile path={} (request_id={request_id})",
            msg.path
        );
        let path = Path::new(&msg.path);

        let (file_id, version) =
            self.pending_file_ops
                .insert(path, request_id.clone(), conn_id, FileOpKind::Write, ctx);

        let file_model = FileModel::handle(ctx);
        if let Err(err) =
            file_model.update(ctx, |m, ctx| m.save(file_id, msg.content, version, ctx))
        {
            self.pending_file_ops.remove(file_id, ctx);
            return HandlerOutcome::Sync(server_message::Message::WriteFileResponse(
                WriteFileResponse {
                    result: Some(write_file_response::Result::Error(FileOperationError {
                        message: format!("Failed to initiate write: {err}"),
                    })),
                },
            ));
        }

        // Response sent asynchronously via the event subscription.
        HandlerOutcome::Async(None)
    }

    /// Handles `DeleteFile` by registering the path and triggering an async
    /// delete via `FileModel`. On a successful dispatch, returns
    /// `HandlerOutcome::Async(None)` — the response is sent later by the
    /// `FileModel` event subscription, and the op is not cancellable via
    /// `Abort`. On failure to dispatch, returns a `HandlerOutcome::Sync`
    /// error response.
    fn handle_delete_file(
        &mut self,
        msg: DeleteFile,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling DeleteFile path={} (request_id={request_id})",
            msg.path
        );
        let path = Path::new(&msg.path);

        let (file_id, version) = self.pending_file_ops.insert(
            path,
            request_id.clone(),
            conn_id,
            FileOpKind::Delete,
            ctx,
        );

        let file_model = FileModel::handle(ctx);
        if let Err(err) = file_model.update(ctx, |m, ctx| m.delete(file_id, version, ctx)) {
            self.pending_file_ops.remove(file_id, ctx);
            return HandlerOutcome::Sync(server_message::Message::DeleteFileResponse(
                DeleteFileResponse {
                    result: Some(delete_file_response::Result::Error(FileOperationError {
                        message: format!("Failed to initiate delete: {err}"),
                    })),
                },
            ));
        }

        // Response sent asynchronously via the event subscription.
        HandlerOutcome::Async(None)
    }

    /// Handles `ReadFileContext` by spawning an async batch file read on the
    /// background executor. Returns `HandlerOutcome::Async` with the spawned
    /// handle so the request can be cancelled via `Abort`.
    fn handle_read_file_context(
        &mut self,
        msg: super::proto::ReadFileContextRequest,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling ReadFileContext ({} files, request_id={request_id})",
            msg.files.len()
        );

        let max_file_bytes = msg.max_file_bytes.map(|b| b as usize);
        let max_batch_bytes = msg.max_batch_bytes.map(|b| b as usize);
        let file_locations: Vec<FileLocations> = msg
            .files
            .into_iter()
            .map(|f| FileLocations {
                name: f.path,
                lines: f
                    .line_ranges
                    .into_iter()
                    .map(|r| r.start as usize..r.end as usize)
                    .collect(),
            })
            .collect();
        let request_id_for_response = request_id.clone();

        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                read_local_file_context(
                    &file_locations,
                    None,
                    None,
                    max_file_bytes,
                    max_batch_bytes,
                )
                .await
            },
            move |me, result: anyhow::Result<ReadFileContextResult>, _ctx| {
                let response = match result {
                    Ok(result) => file_context_result_to_proto(result),
                    Err(err) => ReadFileContextResponse {
                        file_contexts: vec![],
                        failed_files: vec![FailedFileRead {
                            path: String::new(),
                            error: Some(FileOperationError {
                                message: format!("{err:#}"),
                            }),
                        }],
                    },
                };
                me.send_server_message(
                    Some(conn_id),
                    Some(&request_id_for_response),
                    server_message::Message::ReadFileContextResponse(response),
                );
            },
            ctx,
        );

        HandlerOutcome::Async(Some(handle))
    }

    /// Handles `OpenBuffer` by opening the file via `GlobalBufferModel`.
    /// The response is sent asynchronously when `BufferLoaded` fires.
    #[cfg(feature = "local_fs")]
    fn handle_open_buffer(
        &mut self,
        msg: OpenBuffer,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling OpenBuffer path={path} (request_id={request_id})",
            path = msg.path
        );

        let path = PathBuf::from(&msg.path);
        let gbm = GlobalBufferModel::handle(ctx);
        let buffer_state = gbm.update(ctx, |gbm, ctx| gbm.open_server_local(path, ctx));
        let file_id = buffer_state.file_id;

        // Track path → FileId mapping and connection. track_open_buffer holds a strong reference
        // to the buffer — daemon has no editor view, so without holding it the buffer would be
        // reclaimed before FileModel async load completes (see ServerBufferTracker::buffer_handles).
        self.buffers
            .track_open_buffer(msg.path.clone(), file_id, buffer_state.buffer);
        self.buffers.add_connection(file_id, conn_id);

        // If already loaded, respond immediately.
        if gbm.as_ref(ctx).buffer_loaded(file_id) {
            let content = gbm
                .as_ref(ctx)
                .content_for_file(file_id, ctx)
                .unwrap_or_default();
            let server_version = gbm
                .as_ref(ctx)
                .sync_clock_for_server_local(file_id)
                .map(|c| c.server_version.as_u64())
                .unwrap_or(1);
            return HandlerOutcome::Sync(server_message::Message::OpenBufferResponse(
                OpenBufferResponse {
                    content,
                    server_version,
                },
            ));
        }

        // Not yet loaded — stash request info so the GlobalBufferModelEvent
        // subscription can send the response when content arrives.
        self.buffers.insert_pending(
            file_id,
            request_id.clone(),
            conn_id,
            PendingBufferRequestKind::OpenBuffer,
        );
        HandlerOutcome::Async(None)
    }

    /// Handles `BufferEdit` notification (fire-and-forget).
    /// Delegates to `GlobalBufferModel::apply_client_edit`. On rejection
    /// (stale server version), the edit is silently dropped.
    #[cfg(feature = "local_fs")]
    fn handle_buffer_edit(&mut self, msg: BufferEdit, ctx: &mut ModelContext<Self>) {
        let Some(file_id) = self.buffers.file_id_for_path(&msg.path) else {
            log::warn!("BufferEdit for unknown buffer: {path}", path = msg.path);
            return;
        };

        let expected_sv = ContentVersion::from_wire_u64(msg.expected_server_version);
        let new_cv = ContentVersion::from_wire_u64(msg.new_client_version);

        // Per spec: if the edit is rejected (stale server version),
        // the server silently drops it.
        GlobalBufferModel::handle(ctx).update(ctx, |gbm, ctx| {
            gbm.apply_client_edit(file_id, &msg.edits, expected_sv, new_cv, ctx);
        });
    }

    /// Handles `SaveBuffer` by persisting the buffer to disk.
    #[cfg(feature = "local_fs")]
    fn handle_save_buffer(
        &mut self,
        msg: SaveBuffer,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling SaveBuffer path={path} (request_id={request_id})",
            path = msg.path
        );

        let Some(file_id) = self.buffers.file_id_for_path(&msg.path) else {
            return HandlerOutcome::Sync(server_message::Message::SaveBufferResponse(
                SaveBufferResponse {
                    result: Some(save_buffer_response::Result::Error(FileOperationError {
                        message: format!("Buffer not open: {path}", path = msg.path),
                    })),
                },
            ));
        };

        let result = GlobalBufferModel::handle(ctx)
            .update(ctx, |gbm, ctx| gbm.save_server_local(file_id, ctx));

        match result {
            Ok(()) => {
                // Response will come via the FileSaved event subscription.
                // Track the file_id → (request_id, conn_id) so the event
                // handler can correlate.
                self.buffers.insert_pending(
                    file_id,
                    request_id.clone(),
                    conn_id,
                    PendingBufferRequestKind::SaveBuffer,
                );
                HandlerOutcome::Async(None)
            }
            Err(err) => HandlerOutcome::Sync(server_message::Message::SaveBufferResponse(
                SaveBufferResponse {
                    result: Some(save_buffer_response::Result::Error(FileOperationError {
                        message: format!("Failed to save: {err}"),
                    })),
                },
            )),
        }
    }

    /// Handles `ResolveConflict` by replacing the server buffer with the
    /// client's content and persisting to disk. Returns an async
    /// `HandlerOutcome` — the response is sent when `FileSaved` or
    /// `FailedToSave` fires.
    #[cfg(feature = "local_fs")]
    fn handle_resolve_conflict(
        &mut self,
        msg: ResolveConflict,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling ResolveConflict path={path} (request_id={request_id})",
            path = msg.path
        );

        let Some(file_id) = self.buffers.file_id_for_path(&msg.path) else {
            return HandlerOutcome::Sync(server_message::Message::ResolveConflictResponse(
                ResolveConflictResponse {
                    result: Some(resolve_conflict_response::Result::Error(
                        FileOperationError {
                            message: format!("Buffer not open: {path}", path = msg.path),
                        },
                    )),
                },
            ));
        };

        let ack_sv = ContentVersion::from_wire_u64(msg.acknowledged_server_version);
        let current_cv = ContentVersion::from_wire_u64(msg.current_client_version);
        let result = GlobalBufferModel::handle(ctx).update(ctx, |gbm, ctx| {
            gbm.resolve_conflict(file_id, ack_sv, current_cv, &msg.client_content, ctx)
        });

        match result {
            Ok(()) => {
                self.buffers.insert_pending(
                    file_id,
                    request_id.clone(),
                    conn_id,
                    PendingBufferRequestKind::ResolveConflict,
                );
                HandlerOutcome::Async(None)
            }
            Err(err) => HandlerOutcome::Sync(server_message::Message::ResolveConflictResponse(
                ResolveConflictResponse {
                    result: Some(resolve_conflict_response::Result::Error(
                        FileOperationError {
                            message: format!("Failed to resolve conflict: {err}"),
                        },
                    )),
                },
            )),
        }
    }

    /// Zaplex: Handle `ListDirectory` — sync listing of direct children in a directory.
    ///
    /// For precise validation by remote terminal file link detection: client caches real directory
    /// entries under a cwd, link detector uses this to extract correct filenames from `ls -l` lines.
    /// `std::fs::read_dir` is a cheap sync call on daemon, so directly returns
    /// `HandlerOutcome::Sync`, not spawning async.
    #[cfg(feature = "local_fs")]
    fn handle_list_directory(&self, msg: ListDirectory) -> HandlerOutcome {
        log::info!("Handling ListDirectory path={}", msg.path);

        let path = expand_user_path(&msg.path);
        let result = match std::fs::read_dir(&path) {
            Ok(read_dir) => {
                let mut entries = Vec::new();
                for entry in read_dir.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    // Prefer `file_type()` (doesn't follow symlinks, no extra stat needed);
                    // fall back to `metadata()` on failure (follows symlinks).
                    let file_type = entry.file_type().ok();
                    let metadata = entry.metadata().ok();
                    let kind = entry_kind(file_type.as_ref(), metadata.as_ref());
                    let is_dir = kind == FileSystemEntryKind::Directory as i32;
                    let size_bytes = metadata.as_ref().filter(|m| m.is_file()).map(|m| m.len());
                    let modified_epoch_millis = metadata
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(system_time_to_epoch_millis);
                    entries.push(DirEntry {
                        name,
                        is_dir,
                        kind,
                        size_bytes,
                        modified_epoch_millis,
                    });
                }
                entries.sort_by(|a, b| a.name.cmp(&b.name));
                let canonical_path = path
                    .canonicalize()
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();
                list_directory_response::Result::Success(ListDirectorySuccess {
                    entries,
                    canonical_path,
                })
            }
            Err(err) => list_directory_response::Result::Error(FileOperationError {
                message: format!("Failed to list directory {}: {err}", msg.path),
            }),
        };

        HandlerOutcome::Sync(server_message::Message::ListDirectoryResponse(
            ListDirectoryResponse {
                result: Some(result),
            },
        ))
    }

    #[cfg(feature = "local_fs")]
    fn handle_resolve_path(&self, msg: ResolvePath) -> HandlerOutcome {
        let path = expand_user_path(&msg.path);
        let result = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                let file_type = metadata.file_type();
                let kind = entry_kind(Some(&file_type), Some(&metadata));
                let canonical_path = path
                    .canonicalize()
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();
                resolve_path_response::Result::Success(ResolvePathSuccess {
                    canonical_path,
                    kind,
                    size_bytes: metadata.is_file().then_some(metadata.len()),
                })
            }
            Err(err) => resolve_path_response::Result::Error(FileOperationError {
                message: format!("Failed to resolve path {}: {err}", msg.path),
            }),
        };

        HandlerOutcome::Sync(server_message::Message::ResolvePathResponse(
            ResolvePathResponse {
                result: Some(result),
            },
        ))
    }

    #[cfg(feature = "local_fs")]
    fn handle_create_directory(&self, msg: CreateDirectory) -> HandlerOutcome {
        let path = expand_user_path(&msg.path);
        let result = match std::fs::create_dir_all(&path) {
            Ok(()) => {
                let canonical_path = path
                    .canonicalize()
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();
                create_directory_response::Result::Success(CreateDirectorySuccess {
                    canonical_path,
                })
            }
            Err(err) => create_directory_response::Result::Error(FileOperationError {
                message: format!("Failed to create directory {}: {err}", msg.path),
            }),
        };

        HandlerOutcome::Sync(server_message::Message::CreateDirectoryResponse(
            CreateDirectoryResponse {
                result: Some(result),
            },
        ))
    }

    #[cfg(feature = "local_fs")]
    fn handle_read_file_chunk(&self, msg: ReadFileChunk) -> HandlerOutcome {
        use std::io::{Read, Seek, SeekFrom};

        let path = expand_user_path(&msg.path);
        let result = (|| -> std::io::Result<ReadFileChunkSuccess> {
            let mut file = std::fs::File::open(&path)?;
            let total_size = file.metadata().ok().map(|m| m.len());
            file.seek(SeekFrom::Start(msg.offset))?;
            let max_bytes = msg.max_bytes.min(8 * 1024 * 1024) as usize;
            let mut bytes = vec![0; max_bytes];
            let read = file.read(&mut bytes)?;
            bytes.truncate(read);
            let next_offset = msg.offset + read as u64;
            let eof = total_size.is_some_and(|size| next_offset >= size) || read == 0;
            Ok(ReadFileChunkSuccess {
                bytes,
                next_offset,
                total_size,
                eof,
            })
        })();

        let result = match result {
            Ok(success) => read_file_chunk_response::Result::Success(success),
            Err(err) => read_file_chunk_response::Result::Error(FileOperationError {
                message: format!("Failed to read file chunk {}: {err}", msg.path),
            }),
        };

        HandlerOutcome::Sync(server_message::Message::ReadFileChunkResponse(
            ReadFileChunkResponse {
                result: Some(result),
            },
        ))
    }

    #[cfg(feature = "local_fs")]
    fn handle_write_file_chunk(&self, msg: WriteFileChunk) -> HandlerOutcome {
        use std::io::{Seek, SeekFrom, Write};

        let path = expand_user_path(&msg.path);
        let result = (|| -> std::io::Result<WriteFileChunkSuccess> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut options = std::fs::OpenOptions::new();
            options.create(true).write(true);
            if msg.truncate {
                options.truncate(true);
            }
            let mut file = options.open(&path)?;
            file.seek(SeekFrom::Start(msg.offset))?;
            file.write_all(&msg.bytes)?;
            #[cfg(unix)]
            if let Some(executable) = msg.executable {
                use std::os::unix::fs::PermissionsExt;

                let mode = if executable { 0o755 } else { 0o644 };
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
            }
            Ok(WriteFileChunkSuccess {
                next_offset: msg.offset + msg.bytes.len() as u64,
            })
        })();

        let result = match result {
            Ok(success) => write_file_chunk_response::Result::Success(success),
            Err(err) => write_file_chunk_response::Result::Error(FileOperationError {
                message: format!("Failed to write file chunk {}: {err}", msg.path),
            }),
        };

        HandlerOutcome::Sync(server_message::Message::WriteFileChunkResponse(
            WriteFileChunkResponse {
                result: Some(result),
            },
        ))
    }

    /// Handles `CloseBuffer` notification (fire-and-forget).
    /// Removes the connection from the buffer's connection set.
    /// Deallocates the buffer if no connections remain.
    #[cfg(feature = "local_fs")]
    fn handle_close_buffer(
        &mut self,
        msg: CloseBuffer,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) {
        log::info!(
            "Handling CloseBuffer path={path} conn={conn_id}",
            path = msg.path
        );
        self.buffers.close_buffer(&msg.path, conn_id, ctx);
    }
}

#[cfg(feature = "local_fs")]
fn expand_user_path(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(path)
}

#[cfg(feature = "local_fs")]
fn entry_kind(file_type: Option<&std::fs::FileType>, metadata: Option<&std::fs::Metadata>) -> i32 {
    if file_type.is_some_and(|ft| ft.is_symlink()) {
        return FileSystemEntryKind::Symlink as i32;
    }
    if metadata.is_some_and(|metadata| metadata.is_dir()) {
        return FileSystemEntryKind::Directory as i32;
    }
    if metadata.is_some_and(|metadata| metadata.is_file()) {
        return FileSystemEntryKind::File as i32;
    }
    FileSystemEntryKind::Other as i32
}

#[cfg(feature = "local_fs")]
fn system_time_to_epoch_millis(time: std::time::SystemTime) -> Option<u64> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

/// Converts a [`ReadFileContextResult`] into its protobuf equivalent.
fn file_context_result_to_proto(result: ReadFileContextResult) -> ReadFileContextResponse {
    use crate::ai::agent::AnyFileContent;

    let file_contexts = result
        .file_contexts
        .into_iter()
        .map(|fc| {
            let content = match fc.content {
                AnyFileContent::StringContent(text) => {
                    super::proto::file_context_proto::Content::TextContent(text)
                }
                AnyFileContent::BinaryContent(bytes) => {
                    super::proto::file_context_proto::Content::BinaryContent(bytes)
                }
            };
            let last_modified_epoch_millis = fc
                .last_modified
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64);
            FileContextProto {
                file_name: fc.file_name,
                content: Some(content),
                line_range_start: fc.line_range.as_ref().map(|r| r.start as u32),
                line_range_end: fc.line_range.as_ref().map(|r| r.end as u32),
                last_modified_epoch_millis,
                line_count: fc.line_count as u32,
            }
        })
        .collect();

    let failed_files = result
        .missing_files
        .into_iter()
        .map(|path| FailedFileRead {
            path,
            error: Some(FileOperationError {
                message: "File not found or could not be read".to_string(),
            }),
        })
        .collect();

    ReadFileContextResponse {
        file_contexts,
        failed_files,
    }
}

/// Current Unix time in epoch milliseconds (`0` if the clock is before the
/// epoch). Used to stamp session attach times for `ListSessions` / the GC.
#[cfg(unix)]
fn now_epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Discovers the agent-sessions (Claude/Codex/Antigravity CLI conversations) present on
/// this daemon's filesystem and maps them to the wire shape. Pure filesystem +
/// transcript reads — no PTY, so it works on every platform. Runs the same
/// `zaplex_cockpit` discovery the local app uses, over the daemon's own home:
/// discover accounts through the same roots as the route inventory, tail each
/// account's transcripts, flatten.
///
/// Returns an empty list (never an error) when the home directory can't be
/// resolved — an inventory the client can safely fold as "zero sessions here".
struct CollectedAgentSessions {
    sessions: Vec<super::proto::AgentSessionInfo>,
    account_routes: Option<super::agent_account::AccountRoutes>,
}

fn collect_agent_sessions_for_peer(
    transcript_cache: &mut zaplex_cockpit::TranscriptScanCache,
    supports_account_routing: bool,
) -> CollectedAgentSessions {
    let now = chrono::Utc::now();
    // Build account inventory and session inventory in one discovery pass. This
    // keeps CLAUDE_CONFIG_DIR/CODEX_HOME and process-discovered plexed roots
    // identical to the opaque route cache that assigns account ids.
    let scan = super::agent_account::scan_agent_accounts_with_cache(transcript_cache);
    let mut snapshots = scan.sessions;
    if let Some(home) = dirs::home_dir() {
        snapshots.extend(zaplex_cockpit::antigravity_idle_sessions(
            &home,
            now,
            zaplex_cockpit::IDLE_MAX_AGE,
            zaplex_cockpit::IDLE_SESSION_LIMIT,
        ));
    } else {
        log::warn!("Daemon: ListAgentSessions: no home dir; reporting empty inventory");
    }
    let account_routes = supports_account_routing.then_some(scan.routes);
    let sessions = snapshots
        .into_iter()
        .map(|mut snapshot| {
            if let Some(routes) = account_routes.as_ref() {
                snapshot.account_id = super::agent_account::session_account_id(
                    routes,
                    snapshot.provider.as_str(),
                    snapshot.config_dir.as_deref(),
                );
                // A routing-capable peer must never receive a path from this
                // host. An unresolvable opaque identity stays visible but is
                // deliberately non-routable rather than falling back to a path.
                snapshot.config_dir = None;
            }
            super::agent_session::snapshot_to_proto(&snapshot)
        })
        .collect();
    CollectedAgentSessions {
        sessions,
        account_routes,
    }
}

fn collect_agent_sessions(
    transcript_cache: &mut zaplex_cockpit::TranscriptScanCache,
) -> Vec<super::proto::AgentSessionInfo> {
    collect_agent_sessions_for_peer(transcript_cache, false).sessions
}

/// Daemon-side agent-session inventory handler (Agent-Cockpit). Cross-platform:
/// filesystem/transcript discovery, no PTY ownership required.
impl ServerModel {
    /// Reports this host's secret-free AI-account inventory and refreshes the
    /// daemon-local opaque-id route cache. Discovery runs off the model thread;
    /// no provider config path is included in the response.
    fn handle_list_agent_accounts(
        &mut self,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        if !self.client_supports_agent_account_routing(conn_id) {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "agent-account-routing-v1 capability was not negotiated".to_string(),
            }));
        }
        let request_id_for_response = request_id.clone();
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async { super::agent_account::scan_agent_accounts() },
            move |me, scan, _ctx| {
                me.agent_account_routes.replace(scan.routes);
                me.send_server_message(
                    Some(conn_id),
                    Some(&request_id_for_response),
                    server_message::Message::AgentAccountInventory(scan.inventory),
                );
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    /// Resolves and reads one provider transcript entirely on this daemon.
    /// Account discovery, exact opaque-route resolution, and bounded parsing
    /// share one global permit and run off the model thread. No cached route is
    /// trusted to select a transcript root.
    fn handle_read_agent_transcript(
        &mut self,
        request_id: &RequestId,
        conn_id: ConnectionId,
        request: ReadAgentTranscript,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        if !self.client_supports_agent_transcript_read(conn_id) {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "agent-transcript-read-v1 capability was not negotiated".to_string(),
            }));
        }
        let Some(permit) = AgentTranscriptReadPermit::try_acquire(Arc::clone(
            &self.agent_transcript_reads_in_flight,
        )) else {
            return HandlerOutcome::Sync(server_message::Message::AgentTranscriptResponse(
                super::transcript_rpc::busy_response(&request),
            ));
        };
        let request_id_for_response = request_id.clone();
        #[cfg(test)]
        let fresh_routes_for_test = self.fresh_agent_account_routes_for_test.clone();
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                let _permit = permit;
                #[cfg(test)]
                let routes = match fresh_routes_for_test {
                    Some(routes) => routes,
                    None => super::agent_account::scan_agent_accounts().routes,
                };
                #[cfg(not(test))]
                let routes = super::agent_account::scan_agent_accounts().routes;
                let response = match super::transcript_rpc::resolve_request(&routes, request) {
                    Ok(resolved) => super::transcript_rpc::read_transcript(resolved),
                    Err(response) => response,
                };
                (routes, response)
            },
            move |me, (routes, response), _ctx| {
                me.agent_account_routes.replace(routes);
                me.send_server_message(
                    Some(conn_id),
                    Some(&request_id_for_response),
                    server_message::Message::AgentTranscriptResponse(response),
                );
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    /// Reports this host's agent-session inventory for the unified cross-host
    /// Agent-Inventory tree. Discovery failures degrade to an empty list rather
    /// than erroring the client's whole tree.
    ///
    /// The scan itself — account discovery plus a transcript-session filesystem
    /// walk with JSON parsing — can be slow on hosts with many Claude/Codex
    /// transcripts or a slow home dir. Antigravity adds only one small bounded
    /// registry read. Running it inline on the model thread
    /// would stall PTY/session servicing (`SessionInput`/`SessionOutput`/attach)
    /// for the duration of every cockpit inventory poll. So we offload the work
    /// off the model thread: [`Self::spawn_request_handler`] runs the future on
    /// the background executor (see `ModelContext::spawn_abortable`) and invokes
    /// `on_resolve` back on the model thread, where we send the response
    /// correlated to the originating `request_id`/`conn_id`.
    fn handle_list_agent_sessions(
        &mut self,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        let request_id_for_response = request_id.clone();
        let conn_id_for_response = conn_id;
        let supports_account_routing = self.client_supports_agent_account_routing(conn_id);
        let transcript_cache = Arc::clone(&self.agent_transcript_cache);
        // `collect_agent_sessions` performs blocking filesystem/JSON work with no
        // await points; because `spawn_request_handler` schedules this future on
        // the background executor, that blocking work never touches the model
        // thread. The "no home dir → empty list, never error" behavior is
        // preserved inside `collect_agent_sessions`.
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                let mut cache = transcript_cache.lock().unwrap_or_else(|poisoned| {
                    log::warn!(
                        "Daemon: agent transcript cache mutex was poisoned; recovering its state"
                    );
                    poisoned.into_inner()
                });
                collect_agent_sessions_for_peer(&mut cache, supports_account_routing)
            },
            move |me, collected, _ctx| {
                let CollectedAgentSessions {
                    mut sessions,
                    account_routes,
                } = collected;
                if let Some(routes) = account_routes {
                    me.agent_account_routes.replace(routes);
                }
                #[cfg(unix)]
                me.reconcile_and_overlay_agent_bindings(conn_id_for_response, &mut sessions);
                me.send_server_message(
                    Some(conn_id_for_response),
                    Some(&request_id_for_response),
                    server_message::Message::AgentSessionList(AgentSessionList { sessions }),
                );
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    #[cfg(unix)]
    fn handle_list_multiplexer_sessions(
        &mut self,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        let request_id_for_response = request_id.clone();
        let handle = self.spawn_request_handler(
            request_id.clone(),
            super::multiplexer::discover_multiplexer_sessions(),
            move |me, inventory, _ctx| {
                me.send_server_message(
                    Some(conn_id),
                    Some(&request_id_for_response),
                    server_message::Message::MultiplexerSessionList(inventory),
                );
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }
}

/// Daemon-side session-host handlers (Stage 1). Unix-only: the daemon owns the
/// PTYs. The per-session state and async tasks live in `super::session_host`.
#[cfg(unix)]
fn agent_identity_from_proto(
    identity: Option<AgentSessionIdentity>,
) -> Result<AgentIdentity, AgentPtyBindingResponse> {
    let Some(identity) = identity else {
        return Err(agent_pty_binding_response(
            AgentPtyBindingStatus::InvalidRequest,
            "agent identity is required",
        ));
    };
    if identity.provider.is_empty() || identity.session_id.is_empty() {
        return Err(agent_pty_binding_response(
            AgentPtyBindingStatus::InvalidRequest,
            "agent provider and session id are required",
        ));
    }
    if !identity.account_id.is_empty() && !identity.config_dir.is_empty() {
        return Err(agent_pty_binding_response(
            AgentPtyBindingStatus::InvalidRequest,
            "opaque account id and host config path are mutually exclusive",
        ));
    }
    Ok(AgentIdentity {
        provider: identity.provider,
        session_id: identity.session_id,
        account_email: (!identity.account_email.is_empty()).then_some(identity.account_email),
        config_dir: identity
            .account_id
            .is_empty()
            .then_some(identity.config_dir)
            .filter(|config_dir| !config_dir.is_empty()),
        account_id: (!identity.account_id.is_empty()).then_some(identity.account_id),
    })
}

#[cfg(unix)]
fn agent_identity_to_proto(identity: &AgentIdentity) -> AgentSessionIdentity {
    AgentSessionIdentity {
        session_id: identity.session_id.clone(),
        provider: identity.provider.clone(),
        account_email: identity.account_email.clone().unwrap_or_default(),
        config_dir: identity
            .account_id
            .is_none()
            .then(|| identity.config_dir.clone())
            .flatten()
            .unwrap_or_default(),
        account_id: identity.account_id.clone().unwrap_or_default(),
    }
}

#[cfg(unix)]
fn live_agent_identities(sessions: &[AgentSessionInfo]) -> HashSet<AgentIdentity> {
    sessions
        .iter()
        .filter(|session| session.state != "idle")
        .filter(|session| !session.provider.is_empty() && !session.session_id.is_empty())
        .map(|session| AgentIdentity {
            provider: session.provider.clone(),
            session_id: session.session_id.clone(),
            account_email: (!session.account_email.is_empty())
                .then(|| session.account_email.clone()),
            config_dir: session
                .account_id
                .is_empty()
                .then(|| session.config_dir.clone())
                .filter(|config_dir| !config_dir.is_empty()),
            account_id: (!session.account_id.is_empty()).then(|| session.account_id.clone()),
        })
        .collect()
}

#[cfg(unix)]
fn agent_pty_binding_response(
    status: AgentPtyBindingStatus,
    message: impl Into<String>,
) -> AgentPtyBindingResponse {
    AgentPtyBindingResponse {
        status: status.into(),
        message: message.into(),
    }
}

#[cfg(unix)]
fn binding_error_response(error: BindingError) -> AgentPtyBindingResponse {
    match error {
        BindingError::PtyNotFound => agent_pty_binding_response(
            AgentPtyBindingStatus::PtyNotFound,
            "PTY session was not found",
        ),
        BindingError::StaleGeneration => agent_pty_binding_response(
            AgentPtyBindingStatus::StaleGeneration,
            "PTY generation is stale",
        ),
        BindingError::ForeignDaemon => agent_pty_binding_response(
            AgentPtyBindingStatus::ForeignDaemon,
            "request targets another daemon instance",
        ),
        BindingError::ForeignConnection => agent_pty_binding_response(
            AgentPtyBindingStatus::ForeignConnection,
            "PTY is attached to another connection",
        ),
        BindingError::ForegroundConflict => agent_pty_binding_response(
            AgentPtyBindingStatus::ForegroundConflict,
            "PTY already has a live foreground agent",
        ),
        BindingError::HandoffMismatch => agent_pty_binding_response(
            AgentPtyBindingStatus::HandoffMismatch,
            "explicit handoff does not match the foreground agent",
        ),
        BindingError::IdentityNotBound => agent_pty_binding_response(
            AgentPtyBindingStatus::IdentityNotBound,
            "agent identity is not bound to this PTY",
        ),
        BindingError::IdentityAlreadyBound => agent_pty_binding_response(
            AgentPtyBindingStatus::IdentityAlreadyBound,
            "agent identity is already bound to another PTY",
        ),
    }
}

#[cfg(unix)]
fn managed_launch_plan(
    host_id: &str,
    msg: &mut OpenSession,
) -> Result<super::managed_fleet::ManagedLaunchPlan, &'static str> {
    use super::managed_fleet::{
        ClaudePermissionMode, ClaudeRemoteControlSpec, ClaudeSpawnMode, ManagedLaunchKey,
        ManagedLaunchPlan, MANAGED_FLEET_SCHEMA_VERSION,
    };

    let launch = msg
        .managed_launch
        .as_ref()
        .ok_or("missing-managed-launch")?;
    let route = msg
        .agent_launch_route
        .as_ref()
        .ok_or("missing-account-route")?;
    if launch.schema_version != MANAGED_FLEET_SCHEMA_VERSION
        || route.schema_version != super::agent_account::ACCOUNT_ROUTING_SCHEMA_VERSION
        || launch.provider != route.provider
        || launch.project_root != msg.cwd.as_deref().unwrap_or_default()
        || msg.requested_min_available_bytes == Some(0)
    {
        return Err("invalid-managed-envelope");
    }
    let canonical =
        std::fs::canonicalize(&launch.project_root).map_err(|_| "project-unavailable")?;
    if !canonical.is_dir() {
        return Err("project-unavailable");
    }
    let project_identity = super::managed_fleet::ManagedProjectIdentity::capture(&canonical)
        .ok_or("project-identity-unavailable")?;
    let canonical = canonical.to_str().ok_or("project-not-utf8")?.to_string();
    msg.cwd = Some(canonical.clone());
    let key = ManagedLaunchKey::new(host_id, &route.account_id, &canonical, &launch.provider)
        .map_err(|error| error.protocol_code())?;
    let plan = match launch.kind.as_str() {
        "interactive-agent"
            if launch.spawn_mode.is_empty()
                && launch.capacity == 0
                && launch.permission_mode.is_empty()
                && launch.display_name.is_empty() =>
        {
            ManagedLaunchPlan::interactive_agent(&launch.launch_id, key)
                .map_err(|error| error.protocol_code())
        }
        "claude-remote-control" => {
            let spawn_mode = match launch.spawn_mode.as_str() {
                "same-dir" => ClaudeSpawnMode::SameDir,
                "worktree" => ClaudeSpawnMode::Worktree,
                "session" => ClaudeSpawnMode::Session,
                _ => return Err("invalid-spawn-mode"),
            };
            let permission_mode = match launch.permission_mode.as_str() {
                "" => None,
                "acceptEdits" => Some(ClaudePermissionMode::AcceptEdits),
                "auto" => Some(ClaudePermissionMode::Auto),
                "bypassPermissions" => Some(ClaudePermissionMode::BypassPermissions),
                "default" => Some(ClaudePermissionMode::Default),
                "dontAsk" => Some(ClaudePermissionMode::DontAsk),
                "plan" => Some(ClaudePermissionMode::Plan),
                _ => return Err("invalid-permission-mode"),
            };
            let capacity = u16::try_from(launch.capacity).map_err(|_| "invalid-capacity")?;
            let spec = ClaudeRemoteControlSpec::new(
                spawn_mode,
                capacity,
                permission_mode,
                (!launch.display_name.is_empty()).then_some(launch.display_name.as_str()),
            )
            .map_err(|error| error.protocol_code())?;
            ManagedLaunchPlan::claude_remote_control(&launch.launch_id, key, spec)
                .map_err(|error| error.protocol_code())
        }
        "interactive-agent" | "claude-remote-control" => Err("invalid-managed-options"),
        _ => Err("unsupported-managed-kind"),
    }?;
    Ok(plan.with_project_identity(project_identity))
}

#[cfg(unix)]
fn fresh_managed_launch_identity(
    routes: &super::agent_account::AccountRoutes,
    plan: &super::managed_fleet::ManagedLaunchPlan,
) -> Result<super::agent_account::AccountRouteIdentity, &'static str> {
    if !plan.project_identity_is_current() {
        return Err("project-identity-changed");
    }
    super::agent_account::fresh_account_route_identity(
        routes,
        plan.launch_key().provider(),
        plan.launch_key().account_id(),
    )
    .map_err(|_| "account-route-changed")
}

#[cfg(unix)]
fn memory_measurement_to_proto(
    measurement: &super::fleet_memory::MemoryMeasurement,
) -> MemoryMeasurement {
    let status = match measurement.status() {
        super::fleet_memory::MemoryMeasurementStatus::Measured => MemoryMeasurementStatus::Measured,
        super::fleet_memory::MemoryMeasurementStatus::Unavailable => {
            MemoryMeasurementStatus::Unavailable
        }
        super::fleet_memory::MemoryMeasurementStatus::Unsupported => {
            MemoryMeasurementStatus::Unsupported
        }
    };
    MemoryMeasurement {
        status: status.into(),
        bytes: measurement.bytes(),
        provenance: measurement.provenance().protocol_name().to_string(),
        diagnostic_code: measurement
            .diagnostic()
            .map(|diagnostic| diagnostic.protocol_code().to_string())
            .unwrap_or_default(),
    }
}

#[cfg(unix)]
fn managed_session_info(
    metadata: &super::managed_fleet::ManagedSessionMetadata,
    generation: u64,
) -> ManagedSessionInfo {
    managed_session_plan_info(metadata.plan(), generation)
}

#[cfg(unix)]
fn managed_session_plan_info(
    plan: &super::managed_fleet::ManagedLaunchPlan,
    generation: u64,
) -> ManagedSessionInfo {
    ManagedSessionInfo {
        schema_version: super::managed_fleet::MANAGED_FLEET_SCHEMA_VERSION,
        provider: plan.launch_key().provider().to_string(),
        account_id: plan.launch_key().account_id().to_string(),
        project_root: plan.launch_key().project_root().to_string(),
        launch_kind: plan.kind().protocol_name().to_string(),
        launch_id: plan.launch_id().to_string(),
        generation,
    }
}

#[cfg(unix)]
fn managed_launch_to_proto(plan: &super::managed_fleet::ManagedLaunchPlan) -> ManagedLaunch {
    let (spawn_mode, capacity, permission_mode, display_name) = match plan.claude_spec() {
        Some(spec) => (
            spec.spawn_mode().cli_value().to_string(),
            u32::from(spec.capacity()),
            spec.permission_mode()
                .map(|mode| mode.cli_value().to_string())
                .unwrap_or_default(),
            spec.display_name().unwrap_or_default().to_string(),
        ),
        None => (String::new(), 0, String::new(), String::new()),
    };
    ManagedLaunch {
        schema_version: super::managed_fleet::MANAGED_FLEET_SCHEMA_VERSION,
        launch_id: plan.launch_id().to_string(),
        provider: plan.launch_key().provider().to_string(),
        project_root: plan.launch_key().project_root().to_string(),
        kind: plan.kind().protocol_name().to_string(),
        spawn_mode,
        capacity,
        permission_mode,
        display_name,
    }
}

#[cfg(target_os = "linux")]
fn managed_lifecycle_response(
    request: &ManagedSessionLifecycleRequest,
    status: ManagedSessionLifecycleStatus,
    replacement: Option<&SessionOpened>,
    diagnostic_code: impl Into<String>,
) -> ManagedSessionLifecycleResponse {
    ManagedSessionLifecycleResponse {
        schema_version: super::managed_fleet::MANAGED_FLEET_SCHEMA_VERSION,
        action: request.action,
        status: status.into(),
        session_id: request.session_id.clone(),
        generation: request.expected_generation,
        replacement_session_id: replacement
            .map(|opened| opened.session_id.clone())
            .unwrap_or_default(),
        replacement_generation: replacement
            .map(|opened| opened.generation)
            .unwrap_or_default(),
        diagnostic_code: diagnostic_code.into(),
    }
}

#[cfg(unix)]
impl ServerModel {
    fn prune_recent_managed_exits(&mut self, now_epoch_millis: u64) {
        self.recent_managed_exits.retain(|record| {
            now_epoch_millis.saturating_sub(record.exited_at_epoch_millis)
                <= RECENT_MANAGED_EXIT_TTL_MILLIS
        });
    }

    fn record_managed_exit(
        &mut self,
        session_id: &str,
        session: &super::session_host::Session,
        exit_code: Option<i32>,
        diagnostic: ManagedExitDiagnostic,
    ) {
        let Some(metadata) = session.managed.as_ref() else {
            return;
        };
        let Some(account_route_identity) = metadata.account_route_identity().copied() else {
            return;
        };
        let now = now_epoch_millis();
        push_recent_managed_exit(
            &mut self.recent_managed_exits,
            ManagedExitRecord {
                plan: metadata.plan().clone(),
                account_route_identity,
                session_id: session_id.to_string(),
                generation: session.generation,
                exit_code,
                exited_at_epoch_millis: now,
                shell: session.shell.clone(),
                rows: session.rows,
                cols: session.cols,
                ring_ceiling_bytes: session.ring.capacity() as u64,
                diagnostic,
            },
        );
    }

    fn managed_lifecycle_target_matches(&self, request: &ManagedSessionLifecycleRequest) -> bool {
        let Some(session) = self.sessions.get(&request.session_id) else {
            return false;
        };
        let Some(metadata) = session.managed.as_ref() else {
            return false;
        };
        let Ok(key) = super::managed_fleet::ManagedLaunchKey::new(
            &self.host_id,
            &request.account_id,
            &request.project_root,
            &request.provider,
        ) else {
            return false;
        };
        let Ok(identity) = super::managed_fleet::ManagedFleetIdentity::new(
            metadata.plan().launch_key().clone(),
            &request.session_id,
            session.generation,
        ) else {
            return false;
        };
        metadata.plan().launch_id() == request.launch_id
            && identity.matches_action(&key, &request.session_id, request.expected_generation)
    }

    #[cfg(target_os = "linux")]
    fn handle_managed_session_lifecycle(
        &mut self,
        request_id: &RequestId,
        conn_id: ConnectionId,
        request: ManagedSessionLifecycleRequest,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        if !self.client_supports_managed_fleet(conn_id) {
            return HandlerOutcome::Sync(server_message::Message::ManagedSessionLifecycleResponse(
                managed_lifecycle_response(
                    &request,
                    ManagedSessionLifecycleStatus::CapabilityRequired,
                    None,
                    "capability-required",
                ),
            ));
        }
        let action = ManagedSessionLifecycleAction::try_from(request.action).ok();
        if request.schema_version != super::managed_fleet::MANAGED_FLEET_SCHEMA_VERSION
            || matches!(
                action,
                None | Some(ManagedSessionLifecycleAction::Unspecified)
            )
        {
            return HandlerOutcome::Sync(server_message::Message::ManagedSessionLifecycleResponse(
                managed_lifecycle_response(
                    &request,
                    ManagedSessionLifecycleStatus::Failed,
                    None,
                    "invalid-request",
                ),
            ));
        }
        let action = action.expect("validated lifecycle action");
        self.prune_recent_managed_exits(now_epoch_millis());
        let live_target = self.sessions.contains_key(&request.session_id);
        let (plan, account_route_identity, shell, rows, cols, ring_ceiling_bytes) = if live_target {
            if !self.managed_lifecycle_target_matches(&request) {
                return HandlerOutcome::Sync(
                    server_message::Message::ManagedSessionLifecycleResponse(
                        managed_lifecycle_response(
                            &request,
                            ManagedSessionLifecycleStatus::StaleIdentity,
                            None,
                            "stale-identity",
                        ),
                    ),
                );
            }
            let session = self
                .sessions
                .get(&request.session_id)
                .expect("validated live managed session exists");
            let metadata = session
                .managed
                .as_ref()
                .expect("validated managed metadata exists");
            let Some(account_route_identity) = metadata.account_route_identity().copied() else {
                return HandlerOutcome::Sync(
                    server_message::Message::ManagedSessionLifecycleResponse(
                        managed_lifecycle_response(
                            &request,
                            ManagedSessionLifecycleStatus::StaleIdentity,
                            None,
                            "account-route-identity-unavailable",
                        ),
                    ),
                );
            };
            (
                metadata.plan().clone(),
                account_route_identity,
                session.shell.clone(),
                session.rows,
                session.cols,
                session.ring.capacity() as u64,
            )
        } else {
            if action != ManagedSessionLifecycleAction::Restart {
                return HandlerOutcome::Sync(
                    server_message::Message::ManagedSessionLifecycleResponse(
                        managed_lifecycle_response(
                            &request,
                            ManagedSessionLifecycleStatus::NotRunning,
                            None,
                            "not-running",
                        ),
                    ),
                );
            }
            let Some(record) = self
                .recent_managed_exits
                .iter()
                .find(|record| record.matches(&request))
            else {
                return HandlerOutcome::Sync(
                    server_message::Message::ManagedSessionLifecycleResponse(
                        managed_lifecycle_response(
                            &request,
                            ManagedSessionLifecycleStatus::NotRunning,
                            None,
                            "not-running",
                        ),
                    ),
                );
            };
            (
                record.plan.clone(),
                record.account_route_identity,
                record.shell.clone(),
                record.rows,
                record.cols,
                record.ring_ceiling_bytes,
            )
        };

        let daemon_floor = match (action, self.managed_min_available_bytes) {
            (ManagedSessionLifecycleAction::Restart, Err(error)) => {
                return HandlerOutcome::Sync(
                    server_message::Message::ManagedSessionLifecycleResponse(
                        managed_lifecycle_response(
                            &request,
                            ManagedSessionLifecycleStatus::Blocked,
                            None,
                            error.protocol_code(),
                        ),
                    ),
                );
            }
            (ManagedSessionLifecycleAction::Restart, Ok(floor)) => Some(floor),
            (ManagedSessionLifecycleAction::Stop, _) => None,
            (ManagedSessionLifecycleAction::Unspecified, _) => {
                unreachable!("invalid lifecycle action returned before preflight")
            }
        };
        let open = OpenSession {
            cwd: Some(plan.launch_key().project_root().to_string()),
            shell: Some(shell),
            env: HashMap::new(),
            size: Some(SessionSize {
                rows: rows as u32,
                cols: cols as u32,
                pixel_width: 0,
                pixel_height: 0,
            }),
            ring_ceiling_bytes: Some(ring_ceiling_bytes),
            agent_launch_route: Some(super::proto::AgentLaunchRoute {
                schema_version: super::agent_account::ACCOUNT_ROUTING_SCHEMA_VERSION,
                provider: plan.launch_key().provider().to_string(),
                account_id: plan.launch_key().account_id().to_string(),
            }),
            managed_launch: Some(managed_launch_to_proto(&plan)),
            requested_min_available_bytes: None,
        };
        let collected_at = now_epoch_millis();
        let provider = plan.launch_key().provider().to_string();
        let account_id = plan.launch_key().account_id().to_string();
        let plan_for_preflight = plan.clone();
        let request_id_for_response = request_id.clone();
        let request_for_response = request.clone();
        #[cfg(test)]
        let fresh_routes_for_test = self.fresh_agent_account_routes_for_test.clone();
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                #[cfg(test)]
                let routes = match fresh_routes_for_test {
                    Some(routes) => routes,
                    None => super::agent_account::scan_agent_accounts().routes,
                };
                #[cfg(not(test))]
                let routes = super::agent_account::scan_agent_accounts().routes;
                let route_current = super::agent_account::fresh_account_route_identity(
                    &routes,
                    &provider,
                    &account_id,
                ) == Ok(account_route_identity);
                let project_current = plan_for_preflight.project_identity_is_current();
                let memory =
                    daemon_floor.map(|_| super::fleet_memory::collect_host_memory(collected_at));
                (routes, route_current, project_current, memory)
            },
            move |me, (routes, route_current, project_current, memory), ctx| {
                me.agent_account_routes.replace(routes);
                let target_current = if live_target {
                    me.managed_lifecycle_target_matches(&request_for_response)
                } else {
                    me.recent_managed_exits
                        .iter()
                        .any(|record| record.matches(&request_for_response))
                };
                let response = if !target_current {
                    managed_lifecycle_response(
                        &request_for_response,
                        ManagedSessionLifecycleStatus::StaleIdentity,
                        None,
                        "stale-identity",
                    )
                } else if !route_current {
                    managed_lifecycle_response(
                        &request_for_response,
                        ManagedSessionLifecycleStatus::StaleIdentity,
                        None,
                        "account-route-changed",
                    )
                } else if !project_current {
                    managed_lifecycle_response(
                        &request_for_response,
                        ManagedSessionLifecycleStatus::StaleIdentity,
                        None,
                        "project-identity-changed",
                    )
                } else if action == ManagedSessionLifecycleAction::Stop {
                    match me.handle_close_managed_session_verified(
                        &request_for_response.session_id,
                        ctx,
                    ) {
                        Ok(()) => managed_lifecycle_response(
                            &request_for_response,
                            ManagedSessionLifecycleStatus::Stopped,
                            None,
                            String::new(),
                        ),
                        Err(error) => managed_lifecycle_response(
                            &request_for_response,
                            ManagedSessionLifecycleStatus::Failed,
                            None,
                            error.protocol_code(),
                        ),
                    }
                } else {
                    let policy = super::managed_fleet::HeadroomPolicy::new(
                        daemon_floor.expect("restart daemon floor was validated"),
                        None,
                        super::managed_fleet::DEFAULT_MAX_MEASUREMENT_AGE_MILLIS,
                    )
                    .expect("validated daemon floor and constant freshness");
                    let snapshot = memory.expect("restart memory preflight was collected");
                    if let super::managed_fleet::HeadroomDecision::Denied { reason, .. } =
                        super::managed_fleet::evaluate_headroom(
                            policy,
                            &snapshot,
                            now_epoch_millis(),
                        )
                    {
                        managed_lifecycle_response(
                            &request_for_response,
                            ManagedSessionLifecycleStatus::Blocked,
                            None,
                            reason.protocol_code(),
                        )
                    } else {
                        let close_result = if live_target {
                            me.handle_close_managed_session_verified(
                                &request_for_response.session_id,
                                ctx,
                            )
                        } else {
                            Ok(())
                        };
                        if let Err(error) = close_result {
                            managed_lifecycle_response(
                                &request_for_response,
                                ManagedSessionLifecycleStatus::Failed,
                                None,
                                error.protocol_code(),
                            )
                        } else {
                            match me.open_session_ready(conn_id, open, Some(plan), ctx) {
                                HandlerOutcome::Sync(server_message::Message::SessionOpened(
                                    opened,
                                )) => {
                                    me.recent_managed_exits
                                        .retain(|record| !record.matches(&request_for_response));
                                    managed_lifecycle_response(
                                        &request_for_response,
                                        ManagedSessionLifecycleStatus::Restarted,
                                        Some(&opened),
                                        String::new(),
                                    )
                                }
                                HandlerOutcome::Sync(_) | HandlerOutcome::Async(_) => {
                                    managed_lifecycle_response(
                                        &request_for_response,
                                        ManagedSessionLifecycleStatus::Failed,
                                        None,
                                        "restart-start-failed",
                                    )
                                }
                            }
                        }
                    }
                };
                me.send_server_message(
                    Some(conn_id),
                    Some(&request_id_for_response),
                    server_message::Message::ManagedSessionLifecycleResponse(response),
                );
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    fn handle_bind_agent_pty(
        &mut self,
        request_id: &RequestId,
        conn_id: ConnectionId,
        msg: BindAgentPty,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        if !self.client_supports_agent_pty_binding(conn_id) {
            return HandlerOutcome::Sync(server_message::Message::AgentPtyBindingResponse(
                agent_pty_binding_response(
                    AgentPtyBindingStatus::CapabilityRequired,
                    "client did not negotiate agent-pty-binding-v2",
                ),
            ));
        }
        if msg.host_id != self.host_id {
            return HandlerOutcome::Sync(server_message::Message::AgentPtyBindingResponse(
                agent_pty_binding_response(
                    AgentPtyBindingStatus::ForeignDaemon,
                    "request targets another daemon instance",
                ),
            ));
        }
        let request_id_for_response = request_id.clone();
        let supports_account_routing = self.client_supports_agent_account_routing(conn_id);
        let transcript_cache = Arc::clone(&self.agent_transcript_cache);
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                let mut cache = transcript_cache.lock().unwrap_or_else(|poisoned| {
                    log::warn!(
                        "Daemon: agent transcript cache mutex was poisoned; recovering its state"
                    );
                    poisoned.into_inner()
                });
                let collected =
                    collect_agent_sessions_for_peer(&mut cache, supports_account_routing);
                let live_agents = live_agent_identities(&collected.sessions);
                (live_agents, collected.account_routes)
            },
            move |me, (live_agents, account_routes), _ctx| {
                if let Some(routes) = account_routes {
                    me.agent_account_routes.replace(routes);
                }
                let response = me.execute_bind_agent_pty(conn_id, msg, &live_agents);
                me.send_server_message(
                    Some(conn_id),
                    Some(&request_id_for_response),
                    server_message::Message::AgentPtyBindingResponse(response),
                );
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    fn execute_bind_agent_pty(
        &mut self,
        conn_id: ConnectionId,
        msg: BindAgentPty,
        live_agents: &HashSet<AgentIdentity>,
    ) -> AgentPtyBindingResponse {
        if !self.client_supports_agent_pty_binding(conn_id) {
            return agent_pty_binding_response(
                AgentPtyBindingStatus::CapabilityRequired,
                "client did not negotiate agent-pty-binding-v2",
            );
        }
        if !self.client_supports_agent_account_routing(conn_id)
            && (msg
                .agent
                .as_ref()
                .is_some_and(|identity| !identity.account_id.is_empty())
                || msg
                    .handoff_from
                    .as_ref()
                    .is_some_and(|identity| !identity.account_id.is_empty()))
        {
            return agent_pty_binding_response(
                AgentPtyBindingStatus::CapabilityRequired,
                "opaque account identity requires agent-account-routing-v1",
            );
        }
        let agent = match agent_identity_from_proto(msg.agent) {
            Ok(agent) => agent,
            Err(response) => return response,
        };
        let handoff_from = match msg.handoff_from {
            Some(identity) => match agent_identity_from_proto(Some(identity)) {
                Ok(identity) => Some(identity),
                Err(response) => return response,
            },
            None => None,
        };
        self.agent_pty_bindings.reconcile_live_agents(live_agents);
        if !live_agents.contains(&agent) {
            return agent_pty_binding_response(
                AgentPtyBindingStatus::IdentityNotDiscovered,
                "agent identity is not present in the current live inventory",
            );
        }
        match self.agent_pty_bindings.bind(
            conn_id.as_u128(),
            BindingRequest {
                host_id: msg.host_id,
                pty_session_id: msg.pty_session_id,
                pty_generation: msg.pty_session_generation,
                agent,
                handoff_from,
            },
        ) {
            Ok(()) => {
                agent_pty_binding_response(AgentPtyBindingStatus::Bound, "agent bound to PTY")
            }
            Err(error) => binding_error_response(error),
        }
    }

    fn reconcile_and_overlay_agent_bindings(
        &mut self,
        conn_id: ConnectionId,
        sessions: &mut [AgentSessionInfo],
    ) {
        let live_agents = live_agent_identities(sessions);
        self.agent_pty_bindings.reconcile_live_agents(&live_agents);
        if !self.client_supports_agent_pty_binding(conn_id) {
            return;
        }
        for session in sessions {
            let identity = AgentIdentity {
                provider: session.provider.clone(),
                session_id: session.session_id.clone(),
                account_email: (!session.account_email.is_empty())
                    .then(|| session.account_email.clone()),
                config_dir: session
                    .account_id
                    .is_empty()
                    .then(|| session.config_dir.clone())
                    .filter(|config_dir| !config_dir.is_empty()),
                account_id: (!session.account_id.is_empty()).then(|| session.account_id.clone()),
            };
            if let Some(binding) = self.agent_pty_bindings.binding_for(&identity) {
                session.pty_session_id = binding.pty_session_id.clone();
                session.pty_session_generation = binding.pty_generation;
                session.pty_foreground = binding.foreground;
            }
        }
    }

    fn handle_unbind_agent_pty(
        &mut self,
        conn_id: ConnectionId,
        msg: UnbindAgentPty,
    ) -> HandlerOutcome {
        if !self.client_supports_agent_pty_binding(conn_id) {
            return HandlerOutcome::Sync(server_message::Message::AgentPtyBindingResponse(
                agent_pty_binding_response(
                    AgentPtyBindingStatus::CapabilityRequired,
                    "client did not negotiate agent-pty-binding-v2",
                ),
            ));
        }
        if msg
            .agent
            .as_ref()
            .is_some_and(|identity| !identity.account_id.is_empty())
            && !self.client_supports_agent_account_routing(conn_id)
        {
            return HandlerOutcome::Sync(server_message::Message::AgentPtyBindingResponse(
                agent_pty_binding_response(
                    AgentPtyBindingStatus::CapabilityRequired,
                    "opaque account identity requires agent-account-routing-v1",
                ),
            ));
        }
        let agent = match agent_identity_from_proto(msg.agent) {
            Ok(agent) => agent,
            Err(response) => {
                return HandlerOutcome::Sync(server_message::Message::AgentPtyBindingResponse(
                    response,
                ));
            }
        };
        let response = match self.agent_pty_bindings.unbind(
            conn_id.as_u128(),
            &msg.host_id,
            &agent,
            &msg.pty_session_id,
            msg.pty_session_generation,
        ) {
            Ok(()) => {
                agent_pty_binding_response(AgentPtyBindingStatus::Unbound, "agent unbound from PTY")
            }
            Err(error) => binding_error_response(error),
        };
        HandlerOutcome::Sync(server_message::Message::AgentPtyBindingResponse(response))
    }

    fn existing_managed_launch(
        &self,
        plan: &super::managed_fleet::ManagedLaunchPlan,
    ) -> Result<Option<SessionOpened>, &'static str> {
        for (session_id, session) in &self.sessions {
            let Some(metadata) = session.managed.as_ref() else {
                continue;
            };
            let existing = metadata.plan();
            if !existing.project_identity_is_current() {
                return Err("project-identity-changed");
            }
            if existing.launch_id() == plan.launch_id() {
                return if existing.is_retry_of(plan) {
                    Ok(Some(SessionOpened {
                        session_id: session_id.clone(),
                        generation: session.generation,
                    }))
                } else {
                    Err("launch-id-conflict")
                };
            }
            if existing.launch_key() == plan.launch_key() {
                return if existing.same_route_and_configuration(plan) {
                    Ok(Some(SessionOpened {
                        session_id: session_id.clone(),
                        generation: session.generation,
                    }))
                } else {
                    Err("managed-route-conflict")
                };
            }
        }
        Ok(None)
    }

    fn handle_open_session(
        &mut self,
        request_id: &RequestId,
        conn_id: ConnectionId,
        mut msg: OpenSession,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        if msg.managed_launch.is_none() {
            return self.open_session_ready(conn_id, msg, None, ctx);
        }
        if !self.client_supports_managed_fleet(conn_id)
            || !self.client_supports_agent_account_routing(conn_id)
        {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "managed-agent-fleet-v1 capability was not negotiated".to_string(),
            }));
        }
        let plan = match managed_launch_plan(&self.host_id, &mut msg) {
            Ok(plan) => plan,
            Err(code) => {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: format!("managed launch rejected: {code}"),
                }));
            }
        };
        match self.existing_managed_launch(&plan) {
            Ok(Some(_)) | Ok(None) => {}
            Err(code) => {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: format!("managed launch rejected: {code}"),
                }));
            }
        }
        let daemon_floor = match self.managed_min_available_bytes {
            Ok(floor) => floor,
            Err(error) => {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: format!("managed launch rejected: {}", error.protocol_code()),
                }));
            }
        };
        let policy = match super::managed_fleet::HeadroomPolicy::new(
            daemon_floor,
            msg.requested_min_available_bytes,
            super::managed_fleet::DEFAULT_MAX_MEASUREMENT_AGE_MILLIS,
        ) {
            Ok(policy) => policy,
            Err(error) => {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: format!("managed launch rejected: {}", error.protocol_code()),
                }));
            }
        };
        let collected_at = now_epoch_millis();
        let request_id_for_response = request_id.clone();
        let plan_for_preflight = plan.clone();
        #[cfg(test)]
        let fresh_routes_for_test = self.fresh_agent_account_routes_for_test.clone();
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                #[cfg(test)]
                let routes = match fresh_routes_for_test {
                    Some(routes) => routes,
                    None => super::agent_account::scan_agent_accounts().routes,
                };
                #[cfg(not(test))]
                let routes = super::agent_account::scan_agent_accounts().routes;
                let route_identity = fresh_managed_launch_identity(&routes, &plan_for_preflight);
                let snapshot = super::fleet_memory::collect_host_memory(collected_at);
                (routes, route_identity, snapshot)
            },
            move |me, (routes, route_identity, snapshot), ctx| {
                me.agent_account_routes.replace(routes);
                let project_current = plan.project_identity_is_current();
                let route_current = route_identity.as_ref().is_ok_and(|expected| {
                    super::agent_account::current_account_route_identity(
                        &me.agent_account_routes,
                        plan.launch_key().provider(),
                        plan.launch_key().account_id(),
                    ) == Ok(*expected)
                });
                let message = match route_identity {
                    Err(code) => server_message::Message::Error(ErrorResponse {
                        code: ErrorCode::InvalidRequest.into(),
                        message: format!("managed launch rejected: {code}"),
                    }),
                    Ok(_) if !project_current => {
                        server_message::Message::Error(ErrorResponse {
                            code: ErrorCode::InvalidRequest.into(),
                            message:
                                "managed launch rejected: project-identity-changed".to_string(),
                        })
                    }
                    Ok(_) if !route_current => server_message::Message::Error(ErrorResponse {
                        code: ErrorCode::InvalidRequest.into(),
                        message: "managed launch rejected: account-route-changed".to_string(),
                    }),
                    Ok(_) => match me.existing_managed_launch(&plan) {
                        Ok(Some(opened)) => server_message::Message::SessionOpened(opened),
                        Ok(None) => match super::managed_fleet::evaluate_headroom(
                            policy,
                            &snapshot,
                            now_epoch_millis(),
                        ) {
                            super::managed_fleet::HeadroomDecision::Allowed { .. } => {
                                match me.open_session_ready(conn_id, msg, Some(plan), ctx) {
                                    HandlerOutcome::Sync(message) => message,
                                    HandlerOutcome::Async(_) => unreachable!(
                                        "ready managed session creation is synchronous"
                                    ),
                                }
                            }
                            super::managed_fleet::HeadroomDecision::Denied {
                                reason,
                                available_bytes,
                                required_bytes,
                            } => server_message::Message::Error(ErrorResponse {
                                code: ErrorCode::InvalidRequest.into(),
                                message: format!(
                                    "managed launch blocked: {}; available={}; required={required_bytes}",
                                    reason.protocol_code(),
                                    available_bytes
                                        .map(|bytes| bytes.to_string())
                                        .unwrap_or_else(|| "unavailable".to_string())
                                ),
                            }),
                        },
                        Err(code) => server_message::Message::Error(ErrorResponse {
                            code: ErrorCode::InvalidRequest.into(),
                            message: format!("managed launch rejected: {code}"),
                        }),
                    },
                };
                me.send_server_message(Some(conn_id), Some(&request_id_for_response), message);
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    /// Opens a new daemon-hosted session after any managed headroom gate has
    /// passed: allocates a PTY, registers it, and starts reader/writer tasks.
    fn open_session_ready(
        &mut self,
        conn_id: ConnectionId,
        mut msg: OpenSession,
        managed_plan: Option<super::managed_fleet::ManagedLaunchPlan>,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        let supports_agent_account_routing = self.client_supports_agent_account_routing(conn_id);
        if msg.agent_launch_route.is_some() && !supports_agent_account_routing {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "agent-account-routing-v1 capability was not negotiated".to_string(),
            }));
        }
        if supports_agent_account_routing {
            if let Err(message) = super::agent_account::prepare_launch_environment(
                &self.agent_account_routes,
                msg.agent_launch_route.as_ref(),
                &mut msg.env,
            ) {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message,
                }));
            }
        }
        #[cfg(target_os = "linux")]
        let managed_account_route_identity = match managed_plan.as_ref() {
            Some(plan) => match super::agent_account::current_account_route_identity(
                &self.agent_account_routes,
                plan.launch_key().provider(),
                plan.launch_key().account_id(),
            ) {
                Ok(identity) if plan.project_identity_is_current() => Some(identity),
                Ok(_) => {
                    return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                        code: ErrorCode::InvalidRequest.into(),
                        message: "managed launch rejected: project-identity-changed".to_string(),
                    }));
                }
                Err(_) => {
                    return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                        code: ErrorCode::InvalidRequest.into(),
                        message: "managed launch rejected: account-route-changed".to_string(),
                    }));
                }
            },
            None => None,
        };
        let generation = self.next_pty_generation;
        let Some(next_pty_generation) = generation.checked_add(1) else {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::Internal.into(),
                message: "PTY generation counter exhausted".to_string(),
            }));
        };
        let (rows, cols) = msg
            .size
            .as_ref()
            .map(|s| (s.rows.max(1) as usize, s.cols.max(1) as usize))
            .unwrap_or((24, 80));
        let shell = msg
            .shell
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/bash".to_string());
        let cwd = msg.cwd.filter(|c| !c.is_empty());
        // Per-host scrollback ceiling (bytes) for this session's OutputRing;
        // 0/absent → daemon default. Clamp to the host cap so a client can't
        // request an unbounded buffer.
        let ring_ceiling = msg
            .ring_ceiling_bytes
            .filter(|&b| b > 0)
            .map(|b| (b as usize).min(HOST_RING_CAP_BYTES))
            .unwrap_or(super::session_host::RING_CEILING_BYTES);

        let (leader_fd, mut child, bootstrap_file) =
            match crate::terminal::local_tty::spawn_session_pty(
                cwd.as_deref().map(std::path::Path::new),
                &shell,
                &msg.env,
                rows,
                cols,
            ) {
                Ok(pair) => pair,
                Err(e) => {
                    log::warn!("Daemon: OpenSession failed: {e:#}");
                    return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                        code: ErrorCode::Internal.into(),
                        message: format!("failed to open session: {e:#}"),
                    }));
                }
            };

        let async_leader = match async_io::Async::new(std::fs::File::from(leader_fd)) {
            Ok(a) => std::sync::Arc::new(a),
            Err(e) => {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::Internal.into(),
                    message: format!("failed to wrap session pty: {e}"),
                }));
            }
        };

        let session_id = uuid::Uuid::new_v4().to_string();
        let (input_tx, input_rx) = async_channel::unbounded::<super::session_host::PtyInput>();
        let shell_pid = child.id();
        let shell_type = crate::terminal::local_tty::shell::supported_shell_path_and_type(&shell)
            .map(|(_, shell_type)| shell_type);
        let managed_startup = managed_plan.as_ref().map(|plan| {
            plan.startup_command(
                shell_type
                    .map(ShellFamily::from)
                    .unwrap_or(ShellFamily::Posix),
            )
        });
        #[cfg(target_os = "linux")]
        let managed_process_root = match super::fleet_memory::managed_linux_process_identity(
            &super::fleet_memory::RealProcfs,
            shell_pid,
            managed_plan.is_some(),
        ) {
            Ok(identity) => identity,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::Internal.into(),
                    message: format!(
                        "managed launch rejected: process-identity-unavailable ({})",
                        error.protocol_code()
                    ),
                }));
            }
        };
        #[cfg(not(target_os = "linux"))]
        let managed_process_root = None;
        #[cfg(target_os = "linux")]
        let managed = match managed_plan {
            Some(plan) => {
                if !plan.project_identity_is_current() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                        code: ErrorCode::InvalidRequest.into(),
                        message: "managed launch rejected: project-identity-changed".to_string(),
                    }));
                }
                let expected_account_route = managed_account_route_identity
                    .expect("managed account route identity was required");
                if super::agent_account::current_account_route_identity(
                    &self.agent_account_routes,
                    plan.launch_key().provider(),
                    plan.launch_key().account_id(),
                ) != Ok(expected_account_route)
                {
                    let _ = child.kill();
                    let _ = child.wait();
                    return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                        code: ErrorCode::InvalidRequest.into(),
                        message: "managed launch rejected: account-route-changed".to_string(),
                    }));
                }
                Some(super::managed_fleet::ManagedSessionMetadata::new_verified(
                    plan,
                    managed_process_root.expect("managed process identity was required"),
                    expected_account_route,
                ))
            }
            None => None,
        };
        #[cfg(not(target_os = "linux"))]
        let managed = managed_plan.map(|plan| {
            super::managed_fleet::ManagedSessionMetadata::new(plan, managed_process_root)
        });
        self.next_pty_generation = next_pty_generation;
        self.sessions.insert(
            session_id.clone(),
            super::session_host::Session {
                generation,
                leader: async_leader.clone(),
                child,
                _bootstrap_file: bootstrap_file,
                ring: zaplex_remote_session::server::output_ring::OutputRing::new(ring_ceiling),
                rows,
                cols,
                attached: conn_id,
                input_tx,
                accepted_startup_commands: HashMap::new(),
                cwd,
                shell: shell.clone(),
                last_attached_ms: now_epoch_millis(),
                preamble: super::session_host::BootstrapPreamble::new(
                    super::session_host::BOOTSTRAP_PREAMBLE_CAP_BYTES,
                ),
                managed,
            },
        );
        self.agent_pty_bindings.register_pty(
            session_id.clone(),
            generation,
            self.host_id.clone(),
            conn_id.as_u128(),
        );

        let spawner = ctx.spawner();
        let exec = ctx.background_executor();
        exec.spawn(super::session_host::run_session_reader(
            session_id.clone(),
            async_leader.clone(),
            ctx.spawner(),
        ))
        .detach();
        exec.spawn(super::session_host::run_session_writer(
            async_leader,
            input_rx,
            shell_type,
        ))
        .detach();
        // Advisory probe: did the user's profile auto-attach tmux/screen into
        // this session despite the spawn-env opt-outs? See run_multiplexer_probe.
        exec.spawn(super::session_host::run_multiplexer_probe(
            session_id.clone(),
            shell_pid,
            spawner,
        ))
        .detach();

        // Deliver the Zaplexify shell integration as the session's first input
        // (ahead of any user input, via the ordered writer). The terminal
        // *identity* (TERM_PROGRAM=ZaplexTerminal etc.) is a spawn env var in
        // `spawn_session_pty`; here we feed the integration *scripts*.
        //
        // `spawn_session_pty` launches the shell with RC loading suppressed (the
        // same contract the local app uses), so — exactly like a local session —
        // we must deliver BOTH the InitShell emitter and the shell *body*. The
        // body is what sources the user's login RC, sets `ZAPLEX_BOOTSTRAPPED`,
        // and emits the `Bootstrapped` DCS hook carrying the remote `HISTFILE`
        // path. Without the body the session stops at InitShell (prompt marks
        // render) but never becomes bootstrapped, so `is_bootstrapped()` stays
        // false and history is never queryable — history-backed autosuggestions
        // and tab-completions never arm over the remote session.
        //
        // What to send per shell mirrors the local contract exactly
        // (`arguments_for_session_spawning_command` + `enqueue_init_script`):
        //   • zsh  — spawn args use `--no-rcs` (no embedded init) → init + body.
        //   • bash — init is embedded in `--rcfile <(echo …)>` at spawn → body only.
        //   • fish/pwsh — their spawn-time InitShell hook sources an idempotent
        //     body from the session-owned temporary file. Nothing is typed
        //     through the PTY, avoiding fish long-paste and pwsh input loss.
        //   • unclassified $SHELL — plain login shell, no integration.
        let bootstrap_delivery = daemon_bootstrap_delivery(shell_type);
        let bootstrap = match bootstrap_delivery {
            DaemonBootstrapDelivery::OrderedPty => match shell_type {
                Some(ShellType::Zsh) => {
                    let mut buf = crate::terminal::bootstrap::init_shell_script_for_shell(
                        ShellType::Zsh,
                        &crate::ASSETS,
                    )
                    .into_bytes();
                    buf.extend_from_slice(ShellType::Zsh.execute_command_bytes());
                    buf.extend_from_slice(&crate::terminal::bootstrap::script_for_shell(
                        ShellType::Zsh,
                        &crate::ASSETS,
                    ));
                    Some(buf)
                }
                Some(ShellType::Bash) => Some(
                    crate::terminal::bootstrap::script_for_shell(ShellType::Bash, &crate::ASSETS)
                        .into_owned(),
                ),
                Some(ShellType::Fish | ShellType::PowerShell) | None => {
                    unreachable!("ordered PTY delivery is only for bash/zsh")
                }
            },
            DaemonBootstrapDelivery::GuardedFile | DaemonBootstrapDelivery::NoIntegration => None,
        };
        match bootstrap {
            Some(bootstrap) => {
                if let Some(session) = self.sessions.get(&session_id) {
                    if let Err(e) = session
                        .input_tx
                        .try_send(super::session_host::PtyInput::Bootstrap(bootstrap))
                    {
                        log::warn!("Daemon: failed to enqueue bootstrap for {session_id}: {e}");
                    } else {
                        log::info!("Daemon: bootstrapped session {session_id} (shell={shell})");
                    }
                }
            }
            None => match bootstrap_delivery {
                DaemonBootstrapDelivery::GuardedFile => {
                    log::info!(
                        "Daemon: bootstrapped session {session_id} from a guarded body file \
                             (shell={shell})"
                    );
                }
                DaemonBootstrapDelivery::OrderedPty => {
                    unreachable!("bash/zsh always enqueue a bootstrap body")
                }
                DaemonBootstrapDelivery::NoIntegration => {
                    log::info!(
                        "Daemon: shell {shell:?} runs as a plain shell (no block integration); \
                             session {session_id}"
                    );
                }
            },
        }

        if let Some(startup) = managed_startup {
            if let Some(session) = self.sessions.get(&session_id) {
                if let Err(error) = session
                    .input_tx
                    .try_send(super::session_host::PtyInput::Startup(startup))
                {
                    log::warn!(
                        "Daemon: failed to enqueue managed startup for {session_id}: {error}"
                    );
                }
            }
        }

        log::info!("Daemon: opened session {session_id} ({rows}x{cols}, shell={shell})");
        HandlerOutcome::Sync(server_message::Message::SessionOpened(SessionOpened {
            session_id,
            generation,
        }))
    }

    /// Queues input bytes for the session's ordered writer task.
    ///
    /// Ordinary input has no startup id and remains a fire-and-forget
    /// notification. Startup input is a correlated request: the id is recorded
    /// only after `try_send` succeeds, duplicate retries receive the same
    /// positive acknowledgement without a second enqueue, and a writer failure
    /// leaves the id retryable.
    fn handle_session_input(
        &mut self,
        conn_id: ConnectionId,
        msg: SessionInput,
    ) -> Option<StartupCommandAck> {
        let SessionInput {
            session_id,
            bytes,
            startup_command_id,
        } = msg;
        if !self
            .sessions
            .get(&session_id)
            .is_some_and(|session| session.attached == conn_id)
        {
            log::warn!(
                "Daemon: rejecting input for session {session_id} from non-owning connection \
                 {conn_id}"
            );
            return (!startup_command_id.is_empty()).then_some(StartupCommandAck {
                session_id,
                startup_command_id,
                accepted: false,
            });
        }
        if startup_command_id.is_empty() {
            if let Some(session) = self.sessions.get(&session_id) {
                if let Err(e) = session
                    .input_tx
                    .try_send(super::session_host::PtyInput::Visible(bytes))
                {
                    log::warn!("Daemon: dropping input for session {session_id}: {e}");
                }
            }
            return None;
        }
        if !super::session_host::is_valid_startup_input(&bytes) {
            log::warn!(
                "Daemon: rejecting malformed startup command {startup_command_id} for session \
                 {session_id}; expected one non-empty LF-terminated line"
            );
            return Some(StartupCommandAck {
                session_id,
                startup_command_id,
                accepted: false,
            });
        }

        let accepted = if let Some(session) = self.sessions.get_mut(&session_id) {
            if let Some(accepted_bytes) = session.accepted_startup_commands.get(&startup_command_id)
            {
                if accepted_bytes == &bytes {
                    true
                } else {
                    log::warn!(
                        "Daemon: startup command id {startup_command_id} was reused with \
                         different bytes for session {session_id}"
                    );
                    false
                }
            } else if session.accepted_startup_commands.len()
                >= super::session_host::MAX_ACCEPTED_STARTUP_COMMANDS
            {
                log::warn!(
                    "Daemon: startup command ledger is full for session {session_id}; \
                     rejecting new id {startup_command_id}"
                );
                false
            } else {
                match session
                    .input_tx
                    .try_send(super::session_host::PtyInput::Startup(bytes.clone()))
                {
                    Ok(()) => {
                        session
                            .accepted_startup_commands
                            .insert(startup_command_id.clone(), bytes);
                        true
                    }
                    Err(e) => {
                        log::warn!(
                            "Daemon: failed to enqueue startup command {startup_command_id} \
                             for session {session_id}: {e}"
                        );
                        false
                    }
                }
            }
        } else {
            log::warn!(
                "Daemon: startup command {startup_command_id} targeted unknown session {session_id}"
            );
            false
        };

        Some(StartupCommandAck {
            session_id,
            startup_command_id,
            accepted,
        })
    }

    /// Re-attaches a (possibly reconnected) connection to a still-running
    /// session and replays the output it missed. Re-points the session's live
    /// stream at `conn_id`, so subsequent `SessionOutput` pushes go to the
    /// reconnected client. This is the heart of "survives the drop": the session
    /// kept running and buffering into its ring while the client was gone.
    fn handle_attach_session_request(
        &mut self,
        request_id: &RequestId,
        conn_id: ConnectionId,
        msg: AttachSession,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        let managed_session = self
            .sessions
            .get(&msg.session_id)
            .is_some_and(|session| session.managed.is_some());
        if managed_session
            && (msg
                .expected_generation
                .is_none_or(|generation| generation == 0)
                || msg.expected_agent_binding.is_none())
        {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "managed attach requires an exact nonzero generation and foreground agent binding"
                    .to_string(),
            }));
        }
        let Some(expected_proto) = msg.expected_agent_binding.clone() else {
            return self.handle_attach_session(conn_id, msg);
        };
        if !self.client_supports_agent_pty_binding(conn_id) {
            return self.handle_attach_session(conn_id, msg);
        }
        if !expected_proto.account_id.is_empty()
            && !self.client_supports_agent_account_routing(conn_id)
        {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "opaque account identity requires agent-account-routing-v1".to_string(),
            }));
        }
        let expected_agent = match agent_identity_from_proto(Some(expected_proto)) {
            Ok(identity) => identity,
            Err(response) => {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: response.message,
                }));
            }
        };
        if managed_session
            && !self
                .sessions
                .get(&msg.session_id)
                .and_then(|session| session.managed.as_ref())
                .is_some_and(|managed| {
                    managed.plan().launch_key().provider() == expected_agent.provider.as_str()
                        && Some(managed.plan().launch_key().account_id())
                            == expected_agent.account_id.as_deref()
                })
        {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "managed attach agent identity does not match the managed launch route"
                    .to_string(),
            }));
        }
        let request_id_for_response = request_id.clone();
        let supports_account_routing = self.client_supports_agent_account_routing(conn_id);
        let transcript_cache = Arc::clone(&self.agent_transcript_cache);
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                let mut cache = transcript_cache.lock().unwrap_or_else(|poisoned| {
                    log::warn!(
                        "Daemon: agent transcript cache mutex was poisoned; recovering its state"
                    );
                    poisoned.into_inner()
                });
                let collected =
                    collect_agent_sessions_for_peer(&mut cache, supports_account_routing);
                let live_agents = live_agent_identities(&collected.sessions);
                (live_agents, collected.account_routes)
            },
            move |me, (live_agents, account_routes), _ctx| {
                if let Some(routes) = account_routes {
                    me.agent_account_routes.replace(routes);
                }
                let message =
                    match me.validate_fresh_agent_attach(conn_id, &expected_agent, &live_agents) {
                        Ok(()) => match me.handle_attach_session(conn_id, msg) {
                            HandlerOutcome::Sync(message) => message,
                            HandlerOutcome::Async(_) => {
                                unreachable!("validated attach execution is synchronous")
                            }
                        },
                        Err(error) => server_message::Message::Error(error),
                    };
                me.send_server_message(Some(conn_id), Some(&request_id_for_response), message);
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    fn validate_fresh_agent_attach(
        &mut self,
        conn_id: ConnectionId,
        expected_agent: &AgentIdentity,
        live_agents: &HashSet<AgentIdentity>,
    ) -> Result<(), ErrorResponse> {
        self.agent_pty_bindings.reconcile_live_agents(live_agents);
        if !self.client_supports_agent_pty_binding(conn_id) {
            return Err(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "agent-pty-binding-v2 connection changed during inventory refresh"
                    .to_string(),
            });
        }
        if !live_agents.contains(expected_agent) {
            return Err(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "agent is no longer present in the current live inventory".to_string(),
            });
        }
        Ok(())
    }

    fn handle_attach_session(
        &mut self,
        conn_id: ConnectionId,
        msg: AttachSession,
    ) -> HandlerOutcome {
        let client_supports_agent_pty_binding = self.client_supports_agent_pty_binding(conn_id);
        let managed_session = self
            .sessions
            .get(&msg.session_id)
            .is_some_and(|session| session.managed.is_some());
        if managed_session
            && (msg
                .expected_generation
                .is_none_or(|generation| generation == 0)
                || msg.expected_agent_binding.is_none())
        {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "managed attach requires an exact nonzero generation and foreground agent binding"
                    .to_string(),
            }));
        }
        if client_supports_agent_pty_binding && msg.expected_generation.is_none() {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "agent-pty-binding-v2 attach requires a PTY generation".to_string(),
            }));
        }
        if msg.expected_agent_binding.is_some() && !client_supports_agent_pty_binding {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "agent-pty-binding-v2 capability was not negotiated".to_string(),
            }));
        }
        let expected_agent_binding = match msg.expected_agent_binding.clone() {
            Some(identity) => match agent_identity_from_proto(Some(identity)) {
                Ok(identity) => Some(identity),
                Err(response) => {
                    return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                        code: ErrorCode::InvalidRequest.into(),
                        message: response.message,
                    }));
                }
            },
            None => None,
        };
        let Some(session) = self.sessions.get_mut(&msg.session_id) else {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: format!("no such session: {}", msg.session_id),
            }));
        };
        if let Some(managed) = session.managed.as_ref() {
            let matches_managed_route = expected_agent_binding.as_ref().is_some_and(|agent| {
                managed.plan().launch_key().provider() == agent.provider.as_str()
                    && Some(managed.plan().launch_key().account_id()) == agent.account_id.as_deref()
            });
            if !matches_managed_route {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message:
                        "managed attach agent identity does not match the managed launch route"
                            .to_string(),
                }));
            }
        }
        if msg
            .expected_generation
            .is_some_and(|expected| expected != session.generation)
        {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: format!("stale generation for session {}", msg.session_id),
            }));
        }
        if session.attached != conn_id
            && session.attached != uuid::Uuid::nil()
            && self.connection_senders.contains_key(&session.attached)
        {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: format!(
                    "session {} is already attached to a live connection; detach it before handoff",
                    msg.session_id
                ),
            }));
        }
        // T1.3: on a *fresh adopt* (`last_seq == 0`) whose ring has already
        // evicted seq 0, the bootstrap handshake is gone from the replay and the
        // client could never arm bootstrap. `plan_attach` ships the frozen
        // preamble and starts the replay at the preamble's end so the two never
        // overlap; a reconnecting client (`last_seq > 0`) gets no preamble and a
        // normal replay from where it left off.
        let (base_seq, replay, bootstrap_preamble) = super::session_host::plan_attach(
            &session.ring,
            &session.preamble,
            msg.last_seq,
            msg.supports_bootstrap_preamble,
        );
        let generation = session.generation;
        if let Some(expected_agent_binding) = expected_agent_binding.as_ref() {
            let foreground = self
                .agent_pty_bindings
                .foreground_for_pty(&msg.session_id, generation);
            if foreground.map(|binding| &binding.agent) != Some(expected_agent_binding) {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: "foreground agent changed since inventory refresh".to_string(),
                }));
            }
        }
        match self
            .agent_pty_bindings
            .attach_pty(&msg.session_id, generation, conn_id.as_u128())
        {
            Ok(()) => {}
            Err(error) => {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: binding_error_response(error).message,
                }));
            }
        }
        // Route live output to the reconnected connection from now on.
        session.attached = conn_id;
        session.last_attached_ms = now_epoch_millis();
        let agent_binding = if client_supports_agent_pty_binding {
            self.agent_pty_bindings
                .foreground_for_pty(&msg.session_id, generation)
                .map(|binding| agent_identity_to_proto(&binding.agent))
        } else {
            None
        };
        let size = SessionSize {
            rows: session.rows as u32,
            cols: session.cols as u32,
            pixel_width: 0,
            pixel_height: 0,
        };
        log::info!(
            "Daemon: attached conn {conn_id} to session {} (replay {} bytes from seq {base_seq}, \
             preamble {} bytes)",
            msg.session_id,
            replay.len(),
            bootstrap_preamble.len(),
        );
        HandlerOutcome::Sync(server_message::Message::SessionAttached(SessionAttached {
            session_id: msg.session_id,
            size: Some(size),
            base_seq,
            replay,
            bootstrap_preamble,
            generation,
            agent_binding,
        }))
    }

    /// Freezes the session's bootstrap preamble at the boundary the opening
    /// client just reported (T1.3). The preamble was accumulated from seq 0 by
    /// `on_session_output`; here we truncate it to `end_seq` (the client's output
    /// cursor at bootstrap completion) and stop capturing. Idempotent: a session
    /// whose preamble is already frozen (or was abandoned at the cap) ignores
    /// repeats — only the first, opening client defines the boundary.
    fn handle_set_bootstrap_preamble(&mut self, msg: SetBootstrapPreamble) {
        let Some(session) = self.sessions.get_mut(&msg.session_id) else {
            log::debug!(
                "SetBootstrapPreamble for unknown session {}; ignoring",
                msg.session_id
            );
            return;
        };
        session.preamble.freeze(msg.end_seq);
        log::info!(
            "Daemon: froze bootstrap preamble for session {} (reported end_seq {}, {} bytes kept)",
            msg.session_id,
            msg.end_seq,
            session.preamble.frozen().map(<[u8]>::len).unwrap_or(0),
        );
    }

    /// Detaches a connection from a session without ending it: the session keeps
    /// running and its output accumulates in the ring (live pushes to the now
    /// non-attached connection become harmless no-ops) until a later attach.
    fn handle_detach_session(&mut self, conn_id: ConnectionId, msg: DetachSession) {
        if let Some(session) = self.sessions.get_mut(&msg.session_id) {
            // Only the connection that currently owns the attachment may clear it.
            // A late DetachSession from a previously-attached tab must not steal
            // the attachment from a newer tab that has since adopted this session
            // (which would silently cut off the new tab's live output).
            if session.attached == conn_id {
                session.attached = uuid::Uuid::nil();
                log::info!(
                    "Daemon: detached session {} (still running)",
                    msg.session_id
                );
            } else {
                log::debug!(
                    "Daemon: ignoring stale detach for session {} from conn {conn_id} \
                     (currently attached to {})",
                    msg.session_id,
                    session.attached
                );
            }
        }
    }

    /// Lists all live daemon-hosted sessions (Stage 4: multi-session UI / adopt).
    /// Registry membership means alive — exited sessions are removed (and
    /// `SessionExited`-announced) by the reader-EOF/close paths.
    fn handle_list_sessions(
        &mut self,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        self.prune_recent_managed_exits(now_epoch_millis());
        let recent_managed_exits: Vec<_> = self
            .recent_managed_exits
            .iter()
            .map(ManagedExitRecord::to_proto)
            .collect();
        let sessions: Vec<(
            SessionInfo,
            Option<super::fleet_memory::LinuxProcessIdentity>,
        )> = self
            .sessions
            .iter()
            .map(|(id, session)| {
                let title = session
                    .cwd
                    .as_deref()
                    .filter(|c| !c.is_empty())
                    .and_then(|cwd| {
                        std::path::Path::new(cwd)
                            .file_name()
                            .map(|b| b.to_string_lossy().into_owned())
                    })
                    .unwrap_or_else(|| {
                        std::path::Path::new(&session.shell)
                            .file_name()
                            .map(|b| b.to_string_lossy().into_owned())
                            .unwrap_or_else(|| session.shell.clone())
                    });
                (
                    SessionInfo {
                        session_id: id.clone(),
                        title,
                        cwd: session.cwd.clone().unwrap_or_default(),
                        alive: true,
                        last_attached_epoch_millis: session.last_attached_ms,
                        // Per-session output-ring footprint the memory governor
                        // accounts against the host cap (see `gc_sessions`).
                        ring_bytes: session.ring.len() as u64,
                        generation: session.generation,
                        managed: session
                            .managed
                            .as_ref()
                            .map(|managed| managed_session_info(managed, session.generation)),
                        process_memory: None,
                    },
                    session
                        .managed
                        .as_ref()
                        .and_then(super::managed_fleet::ManagedSessionMetadata::process_root),
                )
            })
            .collect();
        let daemon_min_available_bytes = self.managed_min_available_bytes.unwrap_or(0);
        let collected_at_epoch_millis = now_epoch_millis();
        let Some(memory_permit) =
            ManagedMemoryReadPermit::try_acquire(Arc::clone(&self.managed_memory_reads_in_flight))
        else {
            let host = super::fleet_memory::busy_host_memory_measurement();
            let sessions = sessions
                .into_iter()
                .map(|(mut info, _process_root)| {
                    if info.managed.is_some() {
                        info.process_memory = Some(memory_measurement_to_proto(
                            &super::fleet_memory::busy_process_memory_measurement(),
                        ));
                    }
                    info
                })
                .collect();
            return HandlerOutcome::Sync(server_message::Message::SessionList(SessionList {
                sessions,
                host_ring_cap_bytes: HOST_RING_CAP_BYTES as u64,
                host_available_memory: Some(memory_measurement_to_proto(&host)),
                daemon_min_available_bytes,
                collected_at_epoch_millis,
                recent_managed_exits,
            }));
        };
        let request_id_for_response = request_id.clone();
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                let _memory_permit = memory_permit;
                let host = super::fleet_memory::collect_host_memory(collected_at_epoch_millis);
                let sessions = sessions
                    .into_iter()
                    .map(|(mut info, process_root)| {
                        if info.managed.is_some() {
                            let process = match process_root {
                                Some(root) => {
                                    super::fleet_memory::collect_process_session_pss(
                                        root,
                                        collected_at_epoch_millis,
                                    )
                                    .pss
                                }
                                None => super::fleet_memory::missing_process_root_measurement(),
                            };
                            info.process_memory = Some(memory_measurement_to_proto(&process));
                        }
                        info
                    })
                    .collect();
                SessionList {
                    sessions,
                    host_ring_cap_bytes: HOST_RING_CAP_BYTES as u64,
                    host_available_memory: Some(memory_measurement_to_proto(&host.available)),
                    daemon_min_available_bytes,
                    collected_at_epoch_millis,
                    recent_managed_exits,
                }
            },
            move |me, inventory, _ctx| {
                me.send_server_message(
                    Some(conn_id),
                    Some(&request_id_for_response),
                    server_message::Message::SessionList(inventory),
                );
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    /// Memory governor (Stage 4): reaps detached sessions that are either idle
    /// past `max_detached_age_ms`, or — if total ring bytes exceed
    /// `host_ring_cap_bytes` — the oldest detached ones until back under the cap.
    /// Never touches a session with a live attached connection. Returns the
    /// number reaped. Wall-clock is passed in (`now_ms`) so it is unit-testable.
    fn gc_sessions(
        &mut self,
        now_ms: u64,
        max_detached_age_ms: u64,
        host_ring_cap_bytes: usize,
    ) -> usize {
        let mut reaped = 0;

        // Phase 1: detached and idle longer than the max age.
        let aged: Vec<String> = {
            let senders = &self.connection_senders;
            self.sessions
                .iter()
                .filter(|(_, s)| {
                    let detached =
                        s.attached == uuid::Uuid::nil() || !senders.contains_key(&s.attached);
                    super::managed_fleet::eligible_for_detached_age_gc(
                        s.managed.as_ref(),
                        detached,
                        now_ms,
                        s.last_attached_ms,
                        max_detached_age_ms,
                    )
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in aged {
            if let Some(mut session) = self.sessions.remove(&id) {
                self.agent_pty_bindings.remove_pty(&id, session.generation);
                let _ = session.child.kill();
                let _ = session.child.wait();
                reaped += 1;
            }
        }

        // Phase 2: enforce the host-wide ring-bytes cap by reaping oldest
        // detached sessions until back under it.
        let total: usize = self.sessions.values().map(|s| s.ring.len()).sum();
        if total > host_ring_cap_bytes {
            let mut over = total - host_ring_cap_bytes;
            let mut candidates: Vec<(u64, String)> = {
                let senders = &self.connection_senders;
                self.sessions
                    .iter()
                    .filter(|(_, s)| {
                        let detached =
                            s.attached == uuid::Uuid::nil() || !senders.contains_key(&s.attached);
                        super::managed_fleet::eligible_for_ring_pressure_gc(
                            s.managed.as_ref(),
                            detached,
                        )
                    })
                    .map(|(id, s)| (s.last_attached_ms, id.clone()))
                    .collect()
            };
            candidates.sort_by_key(|(age, _)| *age); // oldest (smallest ms) first
            for (_, id) in candidates {
                if over == 0 {
                    break;
                }
                if let Some(mut session) = self.sessions.remove(&id) {
                    self.agent_pty_bindings.remove_pty(&id, session.generation);
                    over = over.saturating_sub(session.ring.len());
                    let _ = session.child.kill();
                    let _ = session.child.wait();
                    reaped += 1;
                }
            }
        }

        if reaped > 0 {
            log::info!("Daemon GC: reaped {reaped} detached session(s)");
        }
        reaped
    }

    /// Arm the shutdown grace timer if the daemon is now fully idle: no
    /// connected proxies *and* no live sessions. Called from every place a
    /// session can cease to exist — explicit close (`handle_close_session`),
    /// PTY EOF (`on_session_reader_eof`) and the periodic GC sweep — so a
    /// daemon whose last session ends while no client is attached retires
    /// after [`GRACE_PERIOD`] instead of lingering until the next GC tick
    /// noticed (`deregister_connection` deliberately skips the timer while
    /// sessions exist). The `grace_timer_cancel.is_none()` guard avoids
    /// restarting an already-running timer (which would reset the countdown).
    fn maybe_arm_grace_when_idle(&mut self, ctx: &mut ModelContext<Self>) {
        if self.connection_senders.is_empty()
            && !self.has_live_sessions()
            && self.grace_timer_cancel.is_none()
        {
            log::info!(
                "Daemon: idle after GC (no connections, no sessions) — grace timer started ({GRACE_PERIOD:?})"
            );
            self.start_grace_timer(ctx);
        }
    }

    /// Starts the periodic detached-session GC sweep on the background executor,
    /// re-entering the model each tick. Runs for the daemon's lifetime.
    fn start_gc_timer(&self, ctx: &mut ModelContext<Self>) {
        let spawner = ctx.spawner();
        ctx.background_executor()
            .spawn(async move {
                loop {
                    async_io::Timer::after(GC_INTERVAL).await;
                    let now = now_epoch_millis();
                    let outcome = spawner
                        .spawn(move |me, ctx| {
                            me.gc_sessions(
                                now,
                                MAX_DETACHED_SESSION_AGE.as_millis() as u64,
                                HOST_RING_CAP_BYTES,
                            );
                            // GC may have reaped the last session. If no proxies
                            // are connected either, the daemon is now fully idle —
                            // arm the grace timer so it exits. deregister_connection
                            // deliberately skipped the timer while sessions existed,
                            // so without this the daemon would linger forever.
                            me.maybe_arm_grace_when_idle(ctx);
                        })
                        .await;
                    if outcome.is_err() {
                        break; // model gone — daemon shutting down
                    }
                }
            })
            .detach();
    }

    /// Applies a window resize to the session's PTY (TIOCSWINSZ).
    fn handle_resize_session(&mut self, conn_id: ConnectionId, msg: ResizeSession) {
        let Some(session) = self.sessions.get_mut(&msg.session_id) else {
            return;
        };
        if session.attached != conn_id {
            log::warn!(
                "Daemon: rejecting resize for session {} from non-owning connection {conn_id}",
                msg.session_id
            );
            return;
        }
        let Some(size) = msg.size else {
            return;
        };
        session.rows = size.rows.max(1) as usize;
        session.cols = size.cols.max(1) as usize;
        let win = libc::winsize {
            ws_row: session.rows as libc::c_ushort,
            ws_col: session.cols as libc::c_ushort,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let fd = session.leader.as_raw_fd();
        // SAFETY: `fd` is a live PTY master; TIOCSWINSZ takes a `*const winsize`.
        unsafe {
            libc::ioctl(fd, libc::TIOCSWINSZ, &win as *const libc::winsize);
        }
    }

    /// Closes one managed session only after its Linux process session has
    /// reached a verified fixed point without live processes.
    #[cfg(target_os = "linux")]
    fn handle_close_managed_session_verified(
        &mut self,
        session_id: &str,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), super::fleet_memory::MemoryDiagnostic> {
        let (plan, expected_account_route, process_root) = {
            let session = self
                .sessions
                .get(session_id)
                .ok_or(super::fleet_memory::MemoryDiagnostic::ProcessIdentityChanged)?;
            let metadata = session
                .managed
                .as_ref()
                .ok_or(super::fleet_memory::MemoryDiagnostic::ProcessIdentityChanged)?;
            (
                metadata.plan().clone(),
                metadata
                    .account_route_identity()
                    .copied()
                    .ok_or(super::fleet_memory::MemoryDiagnostic::AccountRouteChanged)?,
                metadata
                    .process_root()
                    .ok_or(super::fleet_memory::MemoryDiagnostic::ProcessIdentityChanged)?,
            )
        };
        if !plan.project_identity_is_current() {
            return Err(super::fleet_memory::MemoryDiagnostic::ProjectIdentityChanged);
        }
        if super::agent_account::current_account_route_identity(
            &self.agent_account_routes,
            plan.launch_key().provider(),
            plan.launch_key().account_id(),
        ) != Ok(expected_account_route)
        {
            return Err(super::fleet_memory::MemoryDiagnostic::AccountRouteChanged);
        }
        {
            let session = self
                .sessions
                .get_mut(session_id)
                .ok_or(super::fleet_memory::MemoryDiagnostic::ProcessIdentityChanged)?;

            // Keep the registry entry authoritative until the bounded process-
            // session termination has reached a verified live-process fixed point.
            super::fleet_memory::terminate_linux_process_session(process_root, || {
                let _ = session.child.try_wait();
            })?;
        }

        let mut session = self
            .sessions
            .remove(session_id)
            .ok_or(super::fleet_memory::MemoryDiagnostic::ProcessIdentityChanged)?;
        self.agent_pty_bindings
            .remove_pty(session_id, session.generation);
        let _ = session.child.kill();
        let exit_code = session.child.wait().ok().and_then(|status| status.code());
        self.record_managed_exit(
            session_id,
            &session,
            exit_code,
            ManagedExitDiagnostic::Stopped,
        );
        let conn = session.attached;
        self.send_server_message(
            Some(conn),
            None,
            server_message::Message::SessionExited(SessionExited {
                session_id: session_id.to_string(),
                exit_code,
            }),
        );
        self.maybe_arm_grace_when_idle(ctx);
        Ok(())
    }

    /// Closes a session: kills + reaps the shell and emits `SessionExited`.
    fn handle_close_session(&mut self, msg: CloseSession, ctx: &mut ModelContext<Self>) {
        if self
            .sessions
            .get(&msg.session_id)
            .is_some_and(|session| session.managed.is_some())
        {
            log::warn!(
                "Daemon: rejecting generic CloseSession for managed session {}; use the validated managed lifecycle RPC",
                msg.session_id
            );
            return;
        }
        let Some(mut session) = self.sessions.remove(&msg.session_id) else {
            return;
        };
        self.agent_pty_bindings
            .remove_pty(&msg.session_id, session.generation);
        let _ = session.child.kill();
        let exit_code = session.child.wait().ok().and_then(|s| s.code());
        let conn = session.attached;
        self.send_server_message(
            Some(conn),
            None,
            server_message::Message::SessionExited(SessionExited {
                session_id: msg.session_id,
                exit_code,
            }),
        );
        // Dropping `session` drops `input_tx` (writer task ends) and the last
        // app-side Arc; the reader task ends once it observes PTY EOF.
        // This may have been the last session with no proxy attached — retire
        // an idle daemon now, not at the next GC tick.
        self.maybe_arm_grace_when_idle(ctx);
    }

    /// Reader-task callback: append output to the ring and push it to the
    /// attached connection with the chunk's start seq.
    pub(super) fn on_session_output(&mut self, session_id: &str, bytes: Vec<u8>) {
        let Some((seq, conn)) = self.sessions.get_mut(session_id).map(|s| {
            let seq = s.ring.append(&bytes);
            // T1.3: mirror output into the bootstrap preamble (from seq 0, immune
            // to ring eviction) until the handshake boundary is reported.
            s.preamble.capture(&bytes);
            (seq, s.attached)
        }) else {
            return;
        };
        self.send_server_message(
            Some(conn),
            None,
            server_message::Message::SessionOutput(SessionOutput {
                session_id: session_id.to_string(),
                seq,
                bytes,
            }),
        );
    }

    /// Probe-task callback: the session landed inside a terminal multiplexer
    /// (hand-rolled auto-attach). Push an advisory `SessionNotice` to the
    /// attached connection — the client renders a tab notice + warning toast.
    pub(super) fn on_session_multiplexer_detected(&mut self, session_id: &str, mux: &str) {
        let Some(conn) = self.sessions.get(session_id).map(|s| s.attached) else {
            return;
        };
        log::info!("Daemon: session {session_id} is nested inside {mux} (auto-attach)");
        self.send_server_message(
            Some(conn),
            None,
            server_message::Message::SessionNotice(SessionNotice {
                session_id: session_id.to_string(),
                kind: "multiplexer-detected".to_string(),
                detail: mux.to_string(),
            }),
        );
    }

    /// Reader-task callback on PTY EOF: reap the shell and emit `SessionExited`.
    pub(super) fn on_session_reader_eof(&mut self, session_id: &str, ctx: &mut ModelContext<Self>) {
        let Some(mut session) = self.sessions.remove(session_id) else {
            return;
        };
        self.agent_pty_bindings
            .remove_pty(session_id, session.generation);
        let exit_code = session.child.wait().ok().and_then(|s| s.code());
        if session.managed.is_some() {
            self.record_managed_exit(
                session_id,
                &session,
                exit_code,
                ManagedExitDiagnostic::ProcessEnded,
            );
        }
        let conn = session.attached;
        self.send_server_message(
            Some(conn),
            None,
            server_message::Message::SessionExited(SessionExited {
                session_id: session_id.to_string(),
                exit_code,
            }),
        );
        // The shell ending on its own may leave the daemon fully idle (the
        // client may have long disconnected) — retire it now, not at the next
        // GC tick.
        self.maybe_arm_grace_when_idle(ctx);
    }
}

#[cfg(test)]
#[path = "server_model_tests.rs"]
mod tests;
