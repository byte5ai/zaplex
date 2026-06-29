use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// Connection status, used for UI display only; not persisted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionStatus {
    Unknown,
    Online,
    Offline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    Folder,
    Server,
}

impl NodeKind {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            NodeKind::Folder => "folder",
            NodeKind::Server => "server",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "folder" => Some(NodeKind::Folder),
            "server" => Some(NodeKind::Server),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AuthType {
    Password,
    Key,
    OneKey,
}

impl AuthType {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            AuthType::Password => "password",
            AuthType::Key => "key",
            AuthType::OneKey => "onekey",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "password" => Some(AuthType::Password),
            "key" => Some(AuthType::Key),
            "onekey" => Some(AuthType::OneKey),
            _ => None,
        }
    }
}

/// Per-host opt-in for the native persistent remote-session layer.
///
/// `Off` (the default) keeps today's behavior: SSH is a local PTY running the
/// `ssh` binary. The other tiers make the session daemon-hosted — the remote
/// daemon owns the PTY and a replay buffer, so the session survives transport
/// drops and can be reattached.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default, Serialize, Deserialize)]
pub enum SessionResilience {
    /// No persistence; classic local-PTY-runs-ssh.
    #[default]
    Off,
    /// Daemon-hosted session with server-side persistence + replay/reattach.
    PersistOnly,
    /// Persistence plus the mosh-grade UDP transport (Phase B3).
    PersistPlusMosh,
}

impl SessionResilience {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            SessionResilience::Off => "off",
            SessionResilience::PersistOnly => "persist_only",
            SessionResilience::PersistPlusMosh => "persist_plus_mosh",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(SessionResilience::Off),
            "persist_only" => Some(SessionResilience::PersistOnly),
            "persist_plus_mosh" => Some(SessionResilience::PersistPlusMosh),
            _ => None,
        }
    }

    /// Whether this host should run as a daemon-hosted session at all.
    pub fn is_enabled(&self) -> bool {
        !matches!(self, SessionResilience::Off)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OneKeyCredentialKind {
    Password,
    Key,
}

impl OneKeyCredentialKind {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            OneKeyCredentialKind::Password => "password",
            OneKeyCredentialKind::Key => "key",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "password" => Some(OneKeyCredentialKind::Password),
            "key" => Some(OneKeyCredentialKind::Key),
            _ => None,
        }
    }
}

/// Tree node (folder or server), excluding server-only metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SshNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: NodeKind,
    pub name: String,
    pub sort_order: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    /// Meaningful only for folders; UI uses this to decide whether to hide child nodes.
    /// SQLite persistence maintains state across restarts.
    pub is_collapsed: bool,
}

/// Connection configuration for a server node. `password` / `passphrase` are not here;
/// they use keychain.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SshServerInfo {
    pub node_id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub key_path: Option<String>,
    pub credential_id: Option<String>,
    pub startup_command: Option<String>,
    pub notes: Option<String>,
    pub last_connected_at: Option<NaiveDateTime>,
    /// Per-host opt-in for the native persistent remote-session layer.
    pub session_resilience: SessionResilience,
    /// Per-host scrollback/replay buffer ceiling for a daemon session, in MiB.
    /// `0` means "use the daemon default". Only meaningful when
    /// `session_resilience` is enabled (it sizes the daemon-side OutputRing).
    pub ring_ceiling_mb: u32,
}

impl SshServerInfo {
    pub fn new_default(node_id: String) -> Self {
        Self {
            node_id,
            host: String::new(),
            port: 22,
            username: String::new(),
            auth_type: AuthType::Password,
            key_path: None,
            credential_id: None,
            startup_command: None,
            notes: None,
            last_connected_at: None,
            session_resilience: SessionResilience::default(),
            ring_ceiling_mb: 0,
        }
    }

    /// Clones configuration from an existing server, generating a new node_id.
    pub fn clone_from_template(source: &Self, new_node_id: String) -> Self {
        Self {
            node_id: new_node_id,
            host: source.host.clone(),
            port: source.port,
            username: source.username.clone(),
            auth_type: source.auth_type,
            key_path: source.key_path.clone(),
            credential_id: source.credential_id.clone(),
            startup_command: source.startup_command.clone(),
            notes: source.notes.clone(),
            last_connected_at: None,
            session_resilience: source.session_resilience,
            ring_ceiling_mb: source.ring_ceiling_mb,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SshOneKeyCredential {
    pub id: String,
    pub label: String,
    pub username: String,
    pub kind: OneKeyCredentialKind,
    pub key_path: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl SshOneKeyCredential {
    pub fn display_label(&self) -> String {
        if self.username.is_empty() {
            self.label.clone()
        } else {
            format!("{} ({})", self.label, self.username)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSshAuth {
    pub username: String,
    pub auth_type: AuthType,
    pub key_path: Option<String>,
    pub secret_lookup_id: String,
    pub secret_kind: crate::secrets::SecretKind,
}
