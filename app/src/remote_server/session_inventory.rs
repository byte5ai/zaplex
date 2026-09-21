//! Platform-independent session inventory and owning daemon routes.

use instant::Instant;
use remote_server::proto::{MultiplexerSessionList, SessionList};
use remote_server::transport::DaemonRuntimeRoute;

/// Session inventory returned by authenticated daemon connections on one host.
///
/// Older daemons contribute their native sessions and an empty multiplexer
/// list. A client never replaces the typed multiplexer RPC with a host-shell
/// fallback.
#[derive(Clone, Debug, Default)]
pub struct HostSessionInventory {
    pub daemon: SessionList,
    pub sessions: Vec<RoutedDaemonSession>,
    pub multiplexers: MultiplexerSessionList,
}

#[derive(Clone, Debug)]
pub struct RoutedDaemonSession {
    pub session: remote_server::proto::SessionInfo,
    pub route: Option<DaemonRuntimeRoute>,
}

#[derive(Clone, Debug)]
pub struct DaemonRuntimeDiagnostics {
    /// Exact daemon socket filename discovered on the host. Together with
    /// `route`, this keeps diagnostics attributable across version recovery.
    pub runtime_filename: String,
    pub server_version: Option<String>,
    pub route: Option<DaemonRuntimeRoute>,
    /// Local monotonic time captured immediately after `SessionList` succeeds.
    /// Unavailable runtimes have no successful observation and therefore use
    /// `None`.
    pub observed_at: Option<Instant>,
    pub status: DaemonRuntimeDiagnosticsStatus,
}

#[derive(Clone, Debug)]
pub enum DaemonRuntimeDiagnosticsStatus {
    /// The daemon's `SessionList` measurement succeeded. Companion recovery
    /// inventory, such as the multiplexer RPC, may still have failed.
    Available(SessionList),
    /// The daemon returned a `SessionList` but does not advertise session-host
    /// support, so its diagnostic fields cannot be treated as compatible.
    Unsupported,
    /// The daemon route failed to return a `SessionList`, including by deadline.
    Unavailable,
}
