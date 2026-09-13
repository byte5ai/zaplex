//! Platform-independent session inventory and owning daemon routes.

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
