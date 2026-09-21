use lazy_static::lazy_static;
use pathfinder_geometry::rect::RectF;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use warpui::platform::FullscreenState;

use warpui::AppContext;

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_conversations_model::AgentManagementFilters;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::InputConfig;
use crate::ai::blocklist::SerializedBlockListItem;
use crate::code::editor_management::CodeSource;
use crate::drive::ZaplexDriveObjectSettings;
use crate::root_view::quake_mode_window_id;
use crate::server::ids::SyncId;
use crate::settings_view::SettingsSection;
use crate::tab::SelectedTabColor;
use crate::terminal::cli_agent_sessions::PersistedCLIAgentBinding;
use crate::terminal::{ShellLaunchData, TerminalView};
use crate::themes::theme::AnsiColorIdentifier;
use crate::workspace::view::left_panel::ToolPanelView;
use crate::workspace::WorkspaceRegistry;
use warp_core::SessionId;
use warpui::{EntityId, SingletonEntity as _, WeakViewHandle};

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub windows: Vec<WindowSnapshot>,
    pub active_window_index: Option<usize>,
    pub block_lists: Arc<HashMap<PaneUuid, Vec<SerializedBlockListItem>>>,
    pub running_mcp_servers: Vec<uuid::Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaneUuid(pub Vec<u8>);

/// Stable routing data for a remote terminal pane. This is persisted separately
/// from the generic terminal snapshot so remote routes never have to be inferred
/// from a tab title or working directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteTerminalIdentity {
    pub registry_node_id: String,
    pub host: String,
    pub transport: RemoteTerminalTransport,
    pub current_working_directory: Option<String>,
    pub input_draft: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteTerminalTransport {
    ClassicSsh,
    ClassicSshMultiplexer {
        multiplexer: PersistedClassicSshMultiplexer,
    },
    Daemon {
        daemon_host_id: String,
        daemon_runtime: Option<PersistedDaemonRuntime>,
        pty_session_id: String,
        pty_generation: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedClassicSshMultiplexer {
    pub mode: PersistedClassicSshMultiplexerMode,
    pub target: String,
    pub session_name: Option<String>,
    pub window_count: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistedClassicSshMultiplexerMode {
    Tmux,
    ScreenAttached,
    ScreenDetached,
}

impl From<warp_ssh_manager::MultiplexerAttachMode> for PersistedClassicSshMultiplexerMode {
    fn from(mode: warp_ssh_manager::MultiplexerAttachMode) -> Self {
        match mode {
            warp_ssh_manager::MultiplexerAttachMode::Tmux => Self::Tmux,
            warp_ssh_manager::MultiplexerAttachMode::ScreenAttached => Self::ScreenAttached,
            warp_ssh_manager::MultiplexerAttachMode::ScreenDetached => Self::ScreenDetached,
        }
    }
}

impl From<PersistedClassicSshMultiplexerMode> for warp_ssh_manager::MultiplexerAttachMode {
    fn from(mode: PersistedClassicSshMultiplexerMode) -> Self {
        match mode {
            PersistedClassicSshMultiplexerMode::Tmux => Self::Tmux,
            PersistedClassicSshMultiplexerMode::ScreenAttached => Self::ScreenAttached,
            PersistedClassicSshMultiplexerMode::ScreenDetached => Self::ScreenDetached,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedDaemonRuntime {
    pub runtime_filename: String,
    pub server_version: String,
}

/// Canonical identity of one daemon-owned PTY. Registry nodes and display host
/// labels are reconnect metadata, not PTY identity, so aliases intentionally
/// cannot create different keys.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DaemonPtyIdentity {
    pub daemon_host_id: String,
    pub runtime_filename: String,
    pub server_version: String,
    pub pty_session_id: String,
    pub pty_generation: u64,
}

#[derive(Clone)]
pub(crate) struct DaemonPtyClaimOwner {
    pub terminal_view_id: Option<EntityId>,
    pub connection_session_id: SessionId,
    pub terminal_view: Option<WeakViewHandle<TerminalView>>,
}

impl DaemonPtyClaimOwner {
    pub(crate) fn owns_same_terminal_surface(&self, other: &Self) -> bool {
        self.connection_session_id == other.connection_session_id
            && self.terminal_view_id.is_some()
            && self.terminal_view_id == other.terminal_view_id
    }

    pub(crate) fn shares_connection_with(&self, other: &Self) -> bool {
        self.connection_session_id == other.connection_session_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DaemonPtyClaimOutcome<Owner> {
    Claimed,
    Existing(Owner),
}

pub(crate) struct DaemonPtyClaims<Owner> {
    owners: HashMap<DaemonPtyIdentity, Owner>,
}

impl<Owner> Default for DaemonPtyClaims<Owner> {
    fn default() -> Self {
        Self {
            owners: HashMap::new(),
        }
    }
}

impl<Owner: Clone> DaemonPtyClaims<Owner> {
    pub(crate) fn claim(
        &mut self,
        identity: DaemonPtyIdentity,
        owner: Owner,
        is_live: impl FnOnce(&Owner) -> bool,
    ) -> DaemonPtyClaimOutcome<Owner> {
        match self.owners.get(&identity) {
            Some(existing) if is_live(existing) => {
                DaemonPtyClaimOutcome::Existing(existing.clone())
            }
            Some(_) | None => {
                self.owners.insert(identity, owner);
                DaemonPtyClaimOutcome::Claimed
            }
        }
    }

    pub(crate) fn release_where(&mut self, should_release: impl Fn(&Owner) -> bool) {
        self.owners.retain(|_, owner| !should_release(owner));
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.owners.len()
    }
}

lazy_static! {
    /// Sidecar for route data whose lifetime follows the stable terminal UUID.
    /// SQLite owns durability; this map only bridges snapshot construction and
    /// restoration without changing the broadly constructed TerminalPaneSnapshot.
    static ref REMOTE_TERMINAL_IDENTITIES: RwLock<HashMap<PaneUuid, RemoteTerminalIdentity>> =
        RwLock::new(HashMap::new());
    static ref FAILED_REMOTE_TERMINAL_RESTORES: RwLock<HashSet<PaneUuid>> =
        RwLock::new(HashSet::new());
    static ref DAEMON_PTY_CLAIMS: RwLock<DaemonPtyClaims<DaemonPtyClaimOwner>> =
        RwLock::new(DaemonPtyClaims::default());
}

pub(crate) fn claim_daemon_pty(
    identity: DaemonPtyIdentity,
    owner: DaemonPtyClaimOwner,
    app: &AppContext,
) -> DaemonPtyClaimOutcome<DaemonPtyClaimOwner> {
    DAEMON_PTY_CLAIMS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .claim(identity, owner, |existing| {
            existing
                .terminal_view
                .as_ref()
                .is_none_or(|terminal_view| terminal_view.upgrade(app).is_some())
        })
}

pub(crate) fn bind_daemon_pty_claim_owner(
    identity: &DaemonPtyIdentity,
    connection_session_id: SessionId,
    terminal_view: &warpui::ViewHandle<TerminalView>,
) -> bool {
    let mut claims = DAEMON_PTY_CLAIMS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(owner) = claims.owners.get_mut(identity) else {
        return false;
    };
    if owner.connection_session_id != connection_session_id {
        return false;
    }
    if owner
        .terminal_view_id
        .is_some_and(|terminal_view_id| terminal_view_id != terminal_view.id())
    {
        return false;
    }
    owner.terminal_view_id = Some(terminal_view.id());
    owner.terminal_view = Some(terminal_view.downgrade());
    true
}

pub(crate) fn daemon_pty_claim(
    identity: &DaemonPtyIdentity,
    app: &AppContext,
) -> Option<DaemonPtyClaimOwner> {
    DAEMON_PTY_CLAIMS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .owners
        .get(identity)
        .filter(|owner| {
            owner
                .terminal_view
                .as_ref()
                .is_none_or(|terminal_view| terminal_view.upgrade(app).is_some())
        })
        .cloned()
}

pub(crate) fn release_daemon_pty_claim_reservation(
    identity: &DaemonPtyIdentity,
    connection_session_id: SessionId,
) -> bool {
    let mut claims = DAEMON_PTY_CLAIMS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let is_unbound_reservation = claims.owners.get(identity).is_some_and(|owner| {
        owner.connection_session_id == connection_session_id && owner.terminal_view_id.is_none()
    });
    if is_unbound_reservation {
        claims.owners.remove(identity);
    }
    is_unbound_reservation
}

pub(crate) fn release_daemon_pty_claim_for_terminal_view(terminal_view_id: EntityId) {
    DAEMON_PTY_CLAIMS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .release_where(|owner| owner.terminal_view_id == Some(terminal_view_id));
}

pub(crate) fn release_daemon_pty_claim_for_connection(connection_session_id: SessionId) {
    DAEMON_PTY_CLAIMS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .release_where(|owner| owner.connection_session_id == connection_session_id);
}

pub(crate) fn register_remote_terminal_identity(uuid: &[u8], identity: RemoteTerminalIdentity) {
    REMOTE_TERMINAL_IDENTITIES
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(PaneUuid(uuid.to_vec()), identity);
    clear_failed_remote_terminal_restore(uuid);
}

pub(crate) fn remote_terminal_identity(uuid: &[u8]) -> Option<RemoteTerminalIdentity> {
    REMOTE_TERMINAL_IDENTITIES
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&PaneUuid(uuid.to_vec()))
        .cloned()
}

pub(crate) fn remove_remote_terminal_identity(uuid: &[u8]) {
    REMOTE_TERMINAL_IDENTITIES
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&PaneUuid(uuid.to_vec()));
}

pub(crate) fn mark_failed_remote_terminal_restore(uuid: &[u8]) {
    FAILED_REMOTE_TERMINAL_RESTORES
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(PaneUuid(uuid.to_vec()));
}

pub(crate) fn clear_failed_remote_terminal_restore(uuid: &[u8]) {
    FAILED_REMOTE_TERMINAL_RESTORES
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&PaneUuid(uuid.to_vec()));
}

pub(crate) fn failed_remote_terminal_restore(uuid: &[u8]) -> bool {
    FAILED_REMOTE_TERMINAL_RESTORES
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(&PaneUuid(uuid.to_vec()))
}

pub(crate) fn update_remote_terminal_pane_state(
    uuid: &[u8],
    current_working_directory: Option<String>,
    input_draft: String,
) {
    if let Some(identity) = REMOTE_TERMINAL_IDENTITIES
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_mut(&PaneUuid(uuid.to_vec()))
    {
        if current_working_directory.is_some() {
            identity.current_working_directory = current_working_directory;
        }
        identity.input_draft = input_draft;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporaryFileManagerSnapshot {
    pub node_id: String,
    pub mode: FileManagerPaneMode,
    pub current_path: PathBuf,
}

lazy_static! {
    static ref TEMPORARY_FILE_MANAGER_REPLACEMENTS: RwLock<HashMap<PaneUuid, TemporaryFileManagerSnapshot>> =
        RwLock::new(HashMap::new());
}

pub(crate) fn register_temporary_file_manager_replacement(
    uuid: &[u8],
    snapshot: TemporaryFileManagerSnapshot,
) {
    TEMPORARY_FILE_MANAGER_REPLACEMENTS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(PaneUuid(uuid.to_vec()), snapshot);
}

pub(crate) fn temporary_file_manager_replacement(
    uuid: &[u8],
) -> Option<TemporaryFileManagerSnapshot> {
    TEMPORARY_FILE_MANAGER_REPLACEMENTS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&PaneUuid(uuid.to_vec()))
        .cloned()
}

pub(crate) fn remove_temporary_file_manager_replacement(uuid: &[u8]) {
    TEMPORARY_FILE_MANAGER_REPLACEMENTS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&PaneUuid(uuid.to_vec()));
}

/// Wrapper for persisting agent management filters to restore.
#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedAgentManagementFilters {
    pub filters: AgentManagementFilters,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowSnapshot {
    pub tabs: Vec<TabSnapshot>,
    pub active_tab_index: usize,
    pub bounds: Option<RectF>,
    pub fullscreen_state: FullscreenState,
    pub quake_mode: bool,
    pub universal_search_width: Option<f32>,
    pub warp_ai_width: Option<f32>,
    pub voltron_width: Option<f32>,
    pub warp_drive_index_width: Option<f32>,
    pub left_panel_open: bool,
    pub vertical_tabs_panel_open: bool,
    pub left_panel_width: Option<f32>,
    pub right_panel_width: Option<f32>,
    pub agent_management_filters: Option<PersistedAgentManagementFilters>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabSnapshot {
    pub custom_title: Option<String>,
    pub is_pinned: bool,
    pub root: PaneNodeSnapshot,
    pub default_directory_color: Option<AnsiColorIdentifier>,
    pub selected_color: SelectedTabColor,
    pub left_panel: Option<LeftPanelSnapshot>,
    pub right_panel: Option<RightPanelSnapshot>,
}

impl TabSnapshot {
    pub(crate) fn color(&self) -> Option<AnsiColorIdentifier> {
        self.selected_color.resolve(self.default_directory_color)
    }
}

#[derive(Clone, Debug, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "LeafSnapshot is significantly larger than BranchSnapshot due to nested snapshot types."
)]
pub enum PaneNodeSnapshot {
    Branch(BranchSnapshot),
    Leaf(LeafSnapshot),
}

impl PaneNodeSnapshot {
    pub fn has_horizontal_split(&self) -> bool {
        match self {
            PaneNodeSnapshot::Leaf(_) => false,
            PaneNodeSnapshot::Branch(BranchSnapshot {
                direction,
                children,
            }) => {
                let self_has_split = *direction == SplitDirection::Horizontal && children.len() > 1;
                self_has_split
                    || children
                        .iter()
                        .any(|(_, child)| child.has_horizontal_split())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BranchSnapshot {
    pub direction: SplitDirection,
    pub children: Vec<(PaneFlex, PaneNodeSnapshot)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeafSnapshot {
    pub is_focused: bool,
    pub custom_vertical_tabs_title: Option<String>,
    pub contents: LeafContents,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LeafContents {
    Terminal(TerminalPaneSnapshot),
    Notebook(NotebookPaneSnapshot),
    /// A read-only image viewer pane backed by a local file.
    Image {
        path: Option<PathBuf>,
    },
    AIDocument(AIDocumentPaneSnapshot),
    Code(CodePaneSnapShot),
    EnvVarCollection(EnvVarCollectionPaneSnapshot),
    // Zaplex Wave 7-3: the `EnvironmentManagement` LeafContents variant was physically removed
    // along with the Ambient Agent UI subsystem.
    Workflow(WorkflowPaneSnapshot),
    Settings(SettingsPaneSnapshot),
    AIFact(AIFactPaneSnapshot),
    ExecutionProfileEditor,
    CodeReview(CodeReviewPaneSnapshot),
    AmbientAgent(AmbientAgentPaneSnapshot),
    /// An entrypoint pane type to launch other pane types from a search palette. The default view
    /// when creating a tab.
    Welcome {
        startup_directory: Option<PathBuf>,
    },
    /// A new first-time user experience which prioritizes choosing a coding repository.
    GetStarted,
    /// SSH server editor pane (openWarp-specific). Loads/saves using the `ssh_servers.node_id`
    /// primary key. **Not persisted** — after a restart the user reopens it from the left-hand SSH manager tree.
    SshServer {
        node_id: String,
    },
    /// File Manager pane. Remote panes use the stable SSH registry node id;
    /// local panes keep an empty node id. Mode and path are part of the pane
    /// snapshot so mixed layouts restore without deriving identity from a tab.
    Sftp {
        node_id: String,
        mode: FileManagerPaneMode,
        current_path: PathBuf,
    },
    /// Cockpit dashboard pane (account usage/cost/heat overview). **Not
    /// persisted** — after a restart the user reopens it from the cockpit
    /// sidebar; all data is re-derived from the data spine anyway.
    Cockpit,
}

#[cfg(feature = "local_fs")]
impl LeafContents {
    /// Whether this pane content should be written to (and later restored
    /// from) the SQLite app-state database.
    ///
    /// Non-persisted pane types are skipped entirely during the pane tree
    /// traversal in `save_app_state`, so no `pane_nodes` row is inserted for
    /// them. This is important: inserting a `pane_nodes` row with
    /// `is_leaf = true` but no matching `pane_leaves` row leaves an orphan
    /// that `read_node` cannot resolve, which causes the surrounding tab's
    /// restoration to fail and the whole tab to disappear on restart.
    pub(crate) fn is_persisted(&self) -> bool {
        match self {
            // Zaplex Wave 7-3: the `EnvironmentManagement` arm was physically removed along with the variant.
            // SSH server editor: the data (host/user/...) is persisted in the ssh_servers table,
            // the pane itself is just a view, so closing and reopening makes no difference.
            LeafContents::SshServer { .. } => false,
            // Directory pickers are transient modal helpers. Ordinary local and
            // remote File Manager panes are fully restorable.
            LeafContents::Sftp { mode, .. } => !mode.is_picker(),
            // Image viewer panes are intentionally not persisted: they render in-session but
            // are not restored after restart.
            LeafContents::Image { .. } => false,
            // Cockpit dashboard: pure lens over the data spine; reopened from the sidebar.
            LeafContents::Cockpit => false,
            // Ephemeral/generated and remote-file code panes are not restorable. Persisting one
            // would leave behind an orphan `Code` row that is skipped during the restore phase,
            // causing the whole tab to be lost.
            LeafContents::Code(CodePaneSnapShot::Local { source, .. }) => {
                source.as_ref().map(|s| s.is_restorable()).unwrap_or(true)
            }
            LeafContents::Terminal(_)
            | LeafContents::Notebook(_)
            | LeafContents::AIDocument(_)
            | LeafContents::EnvVarCollection(_)
            | LeafContents::Workflow(_)
            | LeafContents::Settings(_)
            | LeafContents::AIFact(_)
            | LeafContents::ExecutionProfileEditor
            | LeafContents::CodeReview(_)
            | LeafContents::AmbientAgent(_)
            | LeafContents::Welcome { .. }
            | LeafContents::GetStarted => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileManagerPaneMode {
    Local,
    Remote,
    RemotePicker,
}

impl FileManagerPaneMode {
    pub fn is_picker(self) -> bool {
        matches!(self, Self::RemotePicker)
    }
}

/// Snapshot of an ambient agent pane.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientAgentPaneSnapshot {
    pub uuid: Vec<u8>,
    // `task_id` is purposefully optional,
    // as you can have a valid state (i.e. an empty ambient-agent pane) where it is None.
    pub task_id: Option<AmbientAgentTaskId>,
}

/// Snapshot of the contents of a terminal pane.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalPaneSnapshot {
    pub uuid: Vec<u8>,
    pub cwd: Option<String>,
    pub cli_agent_binding: Option<PersistedCLIAgentBinding>,
    pub shell_launch_data: Option<ShellLaunchData>,
    pub is_active: bool,
    pub is_read_only: bool,
    pub input_config: Option<InputConfig>,
    pub llm_model_override: Option<String>,
    pub active_profile_id: Option<SyncId>,
    pub conversation_ids_to_restore: Vec<AIConversationId>,
    /// The active conversation ID if the agent view was open in fullscreen mode.
    /// When `Some`, the agent view should be restored to fullscreen for this conversation.
    pub active_conversation_id: Option<AIConversationId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NotebookPaneSnapshot {
    NotebookObject {
        /// The ID of the notebook that was open in this pane. There are 3 possibilities:
        /// 1. The pane contains a newly-created notebook that has not been edited yet. It might not
        ///    have an ID yet (client or server), so this will be `None`.
        /// 2. The pane contains a notebook that hasn't been synced to the server yet, so this will
        ///    contain a client ID that should exist in SQLite.
        /// 3. The pane contains a notebook that's known to the server, so this will contain the
        ///    server ID.
        notebook_id: Option<SyncId>,
        // Settings for the notebook pane when it's opened (such as a folder to focus upon opening)
        settings: ZaplexDriveObjectSettings,
    },
    LocalFileNotebook {
        /// The path to the local file that was open in this pane. This may be `None` if
        /// the pane contained an unreadable file.
        path: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum AIDocumentPaneSnapshot {
    Local {
        document_id: String,
        version: i32,
        content: Option<String>,
        title: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodePaneTabSnapshot {
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodePaneSnapShot {
    Local {
        tabs: Vec<CodePaneTabSnapshot>,
        active_tab_index: usize,
        /// The full `CodeSource` for this pane, serialized as JSON in the DB.
        source: Option<CodeSource>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkflowPaneSnapshot {
    WorkflowObject {
        workflow_id: Option<SyncId>,
        // Settings for the workflow pane when it's opened (such as a folder to focus upon opening)
        settings: ZaplexDriveObjectSettings,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum EnvVarCollectionPaneSnapshot {
    // EnvVarCollectionObject snapshots operate under the same heuristics
    // as NotebookPaneSnapshot::NotebookObject
    EnvVarCollectionObject {
        env_var_collection_id: Option<SyncId>,
    },
}

// Zaplex Wave 7-3: `EnvironmentManagementPaneSnapshot` was physically removed along with the LeafContents variant.

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsPaneSnapshot {
    Local {
        current_page: SettingsSection,
        search_query: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum AIFactPaneSnapshot {
    Personal,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodeReviewPaneSnapshot {
    Local {
        terminal_uuid: Vec<u8>,
        repo_path: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LeftPanelDisplayedTab {
    FileTree,
    GlobalSearch,
    ZaplexDrive,
    ConversationListView,
    SshManager,
    ServerFileBrowser,
    SkillManager,
    Cockpit,
}

impl From<ToolPanelView> for LeftPanelDisplayedTab {
    fn from(view: ToolPanelView) -> Self {
        match view {
            ToolPanelView::ProjectExplorer => LeftPanelDisplayedTab::FileTree,
            ToolPanelView::GlobalSearch { .. } => LeftPanelDisplayedTab::GlobalSearch,
            ToolPanelView::ZaplexDrive => LeftPanelDisplayedTab::ZaplexDrive,
            ToolPanelView::ConversationListView => LeftPanelDisplayedTab::ConversationListView,
            ToolPanelView::SshManager => LeftPanelDisplayedTab::SshManager,
            ToolPanelView::ServerFileBrowser => LeftPanelDisplayedTab::ServerFileBrowser,
            ToolPanelView::SkillManager => LeftPanelDisplayedTab::SkillManager,
            ToolPanelView::Cockpit => LeftPanelDisplayedTab::Cockpit,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeftPanelSnapshot {
    pub left_panel_displayed_tab: LeftPanelDisplayedTab,
    pub pane_group_id: String,
    pub width: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RightPanelSnapshot {
    pub pane_group_id: String,
    pub width: usize,
    pub is_maximized: bool,
}

/// Copied from pane group model, which should be private to pane group.
#[derive(Clone, Debug, PartialEq)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneFlex(pub f32);

pub fn get_app_state(app: &AppContext) -> AppState {
    let active_window_id = app.windows().active_window();
    let quake_mode_id = quake_mode_window_id();

    let mut active_window_index = None;

    let mut windows = vec![];

    for (index, window_id) in app.window_ids().enumerate() {
        // Determine index of active window
        if let Some(active_window_id) = active_window_id {
            if active_window_id == window_id {
                active_window_index = Some(index);
            }
        }

        if let Some(workspace) = WorkspaceRegistry::as_ref(app).get(window_id, app) {
            let ws = workspace.as_ref(app);
            // Transient drag-preview windows are not real user-visible
            // workspaces; skip them so they never end up in the persisted
            // session. (Persistence is also short-circuited entirely while a
            // cross-window drag is active; see `save_app` in
            // `workspace/global_actions.rs`.)
            if ws.is_tab_drag_preview() {
                continue;
            }
            let snapshot = ws.snapshot(
                window_id,
                quake_mode_id.map(|id| id == window_id).unwrap_or(false),
                app,
            );
            if !snapshot.tabs.is_empty() {
                windows.push(snapshot);
            }
        }
    }

    AppState {
        windows,
        active_window_index,
        block_lists: Default::default(),
        running_mcp_servers: Vec::new(),
    }
}

#[cfg(test)]
#[path = "app_state_tests.rs"]
mod tests;
