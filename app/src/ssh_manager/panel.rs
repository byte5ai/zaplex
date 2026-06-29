//! SSH manager main panel — the left Tool Panel content: tree list + toolbar + context menu
//! + inline folder rename.
//!
//! UX rules:
//! - **Click a server**: connect directly (open a terminal pane running ssh). Use right-click to edit.
//! - **Click a folder**: select only (highlight); rename via the right-click "Rename" or by typing right after creating it.
//! - **Enter rename mode immediately after creating a folder** (Drive-style).
//! - Right-click a server: Edit / Connect / Delete
//! - Right-click a folder: New folder / New server / Rename / Delete
//! - Right-click empty space: New folder / New server
//!
//! Visual polish follows the constants in `app/src/drive/index.rs` (ITEM_FONT_SIZE=14 / indent 16 /
//! row padding 4×8).

use std::collections::HashMap;

use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    AcceptedByDropTarget, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Dismiss, Draggable, DraggableState, DropTarget, DropTargetData, Element,
    Empty, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Radius, SavePosition, Stack, Text,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use warp_ssh_manager::{
    AuthType, KeychainSecretStore, NodeKind, SecretKind, SshNode, SshRepository, SshSecretStore,
    SshServerInfo,
};

use remote_server::proto::SessionInfo;

use settings::Setting;

use crate::editor::{
    EditorView, Event as EditorEvent, SingleLineEditorOptions, TextColors, TextOptions,
};
use crate::settings::SshSettings;
use crate::ssh_manager::candidates::{CandidateRow, CandidatesViewModel};
use crate::ssh_manager::{SshTreeChangedEvent, SshTreeChangedNotifier};

// ---- visual constants (see Drive) ----
const TOOLBAR_BUTTON_SIZE: f32 = 26.0;
const TOOLBAR_ICON_SIZE: f32 = 14.0;
const ITEM_PADDING_VERTICAL: f32 = 5.0;
const ITEM_PADDING_HORIZONTAL: f32 = 8.0;
const ITEM_ICON_TEXT_SPACING: f32 = 8.0;
const ITEM_MARGIN_BOTTOM: f32 = 2.0;
const ITEM_ICON_SIZE: f32 = 14.0;
const FOLDER_DEPTH_INDENT: f32 = 16.0;
const PANEL_HORIZONTAL_PADDING: f32 = 8.0;

const CONTEXT_MENU_WIDTH: f32 = 200.0;
const CONTEXT_MENU_ITEM_PADDING_V: f32 = 7.0;
const CONTEXT_MENU_ITEM_PADDING_H: f32 = 12.0;
const MAX_CONTEXT_MENU_ITEMS: usize = 6;
const SSH_PANEL_POSITION_ID: &str = "ssh_manager_panel_root";

#[derive(Clone, Debug)]
pub enum SshManagerPanelAction {
    /// Toolbar button: always creates a folder at the root level.
    AddRootFolder,
    /// Context menu: the parent is determined by context.
    AddFolder,
    AddServer,
    DeleteSelected,
    Connect,
    Edit,
    CloneServer(String),
    /// Context menu on a server: toggle the inline list of its running daemon
    /// sessions (fetched via connect-to-list on first expand).
    ToggleSessions(String),
    /// Click a listed daemon session: adopt it (attach + replay) in a new tab.
    AdoptSession {
        node_id: String,
        pty_session_id: String,
    },
    /// Click a row; the handling depends on the node kind:
    /// - server: select + emit OpenSshTerminal (connect directly)
    /// - folder: select only
    Click(String),
    StartRename(String),
    CommitRename,
    CancelRename,
    OpenContextMenu {
        target: Option<String>,
        position: Vector2F,
    },
    DismissContextMenu,
    /// Drag completed → move `node_id` under `new_parent_id` (None = root).
    MoveNode {
        node_id: String,
        new_parent_id: Option<String>,
    },
    /// Collapse/expand a single folder. Server nodes are ignored.
    ToggleNodeCollapsed(String),
    /// Top button: smart toggle — if any folder is still expanded → collapse all; otherwise expand all.
    ToggleAllFolders,
    /// Double-click a server row = connect (open a new tab). Double-clicking a folder = two toggles cancel out, a no-op.
    DoubleClick(String),
    /// Right-click a server → "File management": open the SFTP file browser pane.
    OpenSftp,
    /// Toolbar "+": open/close the guided "Add a host" block (blank server +
    /// on-demand `~/.ssh/config` suggestions). The saved list stays untouched
    /// until the user explicitly creates or imports.
    ToggleAddMode,
    /// "Candidates" section: copy one candidate from `~/.ssh/config` into the saved tree.
    ImportCandidate {
        alias: String,
    },
    /// Re-read `~/.ssh/config` (after the user edits the config and clicks the Refresh button).
    RefreshCandidates,
    /// Collapse/expand the "Candidates" section (manually collapse when the list is long).
    ToggleCandidatesSection,
}

#[derive(Clone, Debug)]
pub enum SshManagerPanelEvent {
    /// The user right-clicked "Edit" on a server; the central pane should open/focus that server's editor
    /// (`Workspace::open_ssh_server`).
    OpenServerEditor {
        node_id: String,
    },
    /// The user clicked a server or right-clicked "Connect", requesting a terminal pane running ssh +
    /// SecretInjector.
    OpenSshTerminal {
        node_id: String,
        server: SshServerInfo,
    },
    /// The user right-clicked "SFTP browse", requesting the SFTP file browser pane.
    OpenSftpPane {
        node_id: String,
        server: SshServerInfo,
    },
    /// The user clicked a listed (running) daemon session under a host, asking to
    /// adopt it in a new tab (attach + replay). The list comes from the
    /// multi-session sidebar (`headless_connect::list_daemon_sessions`).
    AdoptDaemonSession {
        server: SshServerInfo,
        pty_session_id: String,
    },
    PersistenceError(String),
}

struct RenameState {
    node_id: String,
    editor: ViewHandle<EditorView>,
    /// Whether the rename was auto-triggered by creating a new folder.
    is_newly_created: bool,
}

/// Content fields for a single candidate row — bundles the few Options that rendering cares about into one struct,
/// to keep `render_candidate_row`'s argument list from getting too long (clippy::too_many_arguments).
struct CandidateRowFields<'a> {
    alias: &'a str,
    hostname: Option<&'a str>,
    user: Option<&'a str>,
    port: Option<u16>,
    added: bool,
}

/// Theme color pair — imported rows use muted, normal rows use main.
#[derive(Copy, Clone)]
struct CandidateRowColors {
    main: warp_core::ui::theme::Fill,
    muted: warp_core::ui::theme::Fill,
}

/// Drop-target metadata. `parent_id = None` means dropping onto empty panel space (back to root);
/// `Some(folder_id)` means dropping into that folder; **dropping directly onto a server is not allowed** (a server
/// cannot have children) — in that case drop_data is interpreted as "drop at the server's sibling position", i.e.
/// `parent_id = server.parent_id`, which is already resolved when dispatching the action.
#[derive(Debug, Clone)]
struct SshDropData {
    parent_id: Option<String>,
}

impl DropTargetData for SshDropData {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub struct SshManagerPanel {
    nodes: Vec<SshNode>,
    depths: HashMap<String, usize>,
    selected_id: Option<String>,

    add_folder_btn: MouseStateHandle,
    add_server_btn: MouseStateHandle,
    toggle_all_btn: MouseStateHandle,
    row_states: HashMap<String, MouseStateHandle>,
    /// Per-row DraggableState — preserves drag progress across renders, so it must be cached in the view state.
    row_drag_states: HashMap<String, DraggableState>,

    context_menu_position: Option<Vector2F>,
    context_menu_target: Option<String>,
    context_menu_item_states: Vec<MouseStateHandle>,

    /// The node currently being renamed (editor + node_id).
    rename_state: Option<RenameState>,

    /// `~/.ssh/config` candidates view-model — PRODUCT.md decision A/B/C/D/E.
    candidates: ModelHandle<CandidatesViewModel>,
    /// Hover state for each candidate row (key = alias).
    candidate_row_states: HashMap<String, MouseStateHandle>,
    /// Hover state for each candidate row's "+" / "Added" button (key = alias).
    candidate_add_states: HashMap<String, MouseStateHandle>,
    /// Hover state for the section header's Refresh / Toggle buttons.
    candidates_refresh_btn: MouseStateHandle,
    candidates_toggle_btn: MouseStateHandle,
    /// "Add a host" guided block is open (toggled by the toolbar "+").
    /// The `~/.ssh/config` suggestions are shown **only** while this is true, so
    /// nothing unsolicited ever appears in the saved list (PRODUCT decision:
    /// suggestions on-demand-when-adding, not always-on).
    adding_mode: bool,
    /// Hover state for the "Create a blank server" button in the add block.
    add_blank_btn: MouseStateHandle,
    /// Hover state for the "Cancel" button in the add block.
    add_cancel_btn: MouseStateHandle,

    // --- Adopt-sidebar: per-host running daemon sessions (multi-session) ---
    /// Running daemon sessions per server node, fetched on demand via
    /// `headless_connect::list_daemon_sessions` (connect-to-list, so it also
    /// surfaces sessions that survived a restart / drop — the main use case).
    host_sessions: HashMap<String, Vec<SessionInfo>>,
    /// Server node_ids whose session list is currently shown (expanded).
    sessions_expanded: std::collections::HashSet<String>,
    /// Server node_ids with an in-flight session fetch.
    sessions_loading: std::collections::HashSet<String>,
    /// Last fetch error per server node_id (shown inline under the host).
    sessions_error: HashMap<String, String>,
    /// Hover/click state per session row (key = "<node_id>:<pty_session_id>").
    session_row_states: HashMap<String, MouseStateHandle>,
}

impl SshManagerPanel {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let candidates = ctx.add_model(|_| CandidatesViewModel::new());

        let mut me = Self {
            nodes: Vec::new(),
            depths: HashMap::new(),
            selected_id: None,
            add_folder_btn: MouseStateHandle::default(),
            add_server_btn: MouseStateHandle::default(),
            toggle_all_btn: MouseStateHandle::default(),
            row_states: HashMap::new(),
            row_drag_states: HashMap::new(),
            context_menu_position: None,
            context_menu_target: None,
            context_menu_item_states: (0..MAX_CONTEXT_MENU_ITEMS)
                .map(|_| MouseStateHandle::default())
                .collect(),
            rename_state: None,
            candidates,
            candidate_row_states: HashMap::new(),
            candidate_add_states: HashMap::new(),
            candidates_refresh_btn: MouseStateHandle::default(),
            candidates_toggle_btn: MouseStateHandle::default(),
            adding_mode: false,
            add_blank_btn: MouseStateHandle::default(),
            add_cancel_btn: MouseStateHandle::default(),
            host_sessions: HashMap::new(),
            sessions_expanded: std::collections::HashSet::new(),
            sessions_loading: std::collections::HashSet::new(),
            sessions_error: HashMap::new(),
            session_row_states: HashMap::new(),
        };
        // `~/.ssh/config` is read on-demand only when the user opens the "Add a
        // host" block (`on_toggle_add_mode`) — never unsolicited on mount, so the
        // saved list never shows hosts the user didn't deliberately add.
        me.refresh_tree(ctx);

        ctx.subscribe_to_model(
            &SshTreeChangedNotifier::handle(ctx),
            |me, _, event, ctx| match event {
                SshTreeChangedEvent::TreeChanged => me.refresh_tree(ctx),
            },
        );

        // Listen for SshSettings changes; refresh the candidates section when the auto-discovery toggle flips.
        ctx.subscribe_to_model(&SshSettings::handle(ctx), |me, _, _, ctx| {
            me.candidates.update(ctx, |vm, ctx| vm.refresh(ctx));
            me.sync_candidate_row_states(ctx);
            ctx.notify();
        });

        me
    }

    fn refresh_tree(&mut self, ctx: &mut ViewContext<Self>) {
        match warp_ssh_manager::with_conn(|c| Ok(SshRepository::list_nodes(c)?)) {
            Ok(nodes) => {
                self.depths = compute_depths(&nodes);
                self.nodes = sort_for_display(nodes, &self.depths);
                if let Some(id) = self.selected_id.clone() {
                    if !self.nodes.iter().any(|n| n.id == id) {
                        self.selected_id = None;
                    }
                }
                // If the node being renamed was deleted externally, clear rename_state
                if let Some(rs) = self.rename_state.as_ref() {
                    if !self.nodes.iter().any(|n| n.id == rs.node_id) {
                        self.rename_state = None;
                    }
                }
            }
            Err(e) => {
                log::error!("ssh_manager: failed to load tree: {e:?}");
                ctx.emit(SshManagerPanelEvent::PersistenceError(e.to_string()));
            }
        }

        let active_ids: std::collections::HashSet<&str> =
            self.nodes.iter().map(|n| n.id.as_str()).collect();
        self.row_states
            .retain(|k, _| active_ids.contains(k.as_str()));
        self.row_drag_states
            .retain(|k, _| active_ids.contains(k.as_str()));
        for n in &self.nodes {
            self.row_states.entry(n.id.clone()).or_default();
            self.row_drag_states.entry(n.id.clone()).or_default();
        }

        // Prune per-host adopt-session state for nodes that were deleted, so these
        // maps don't grow unbounded across deletions (keyed by node_id; the
        // row-state map is keyed by "<node_id>:<pty_session_id>").
        self.host_sessions
            .retain(|k, _| active_ids.contains(k.as_str()));
        self.sessions_expanded
            .retain(|k| active_ids.contains(k.as_str()));
        self.sessions_loading
            .retain(|k| active_ids.contains(k.as_str()));
        self.sessions_error
            .retain(|k, _| active_ids.contains(k.as_str()));
        self.session_row_states.retain(|k, _| {
            k.split(':')
                .next()
                .is_some_and(|node_id| active_ids.contains(node_id))
        });

        // Tree changed → recompute the "Added" set (PRODUCT.md decision E). "Imported" is determined by
        // `server.host == candidate.alias` — aligned with ImportCandidate's write
        // semantics (decision I: on import, `server.host = alias`).
        let auto_discover = *SshSettings::as_ref(ctx).enable_ssh_auto_discovery.value();
        if auto_discover {
            let hosts = list_server_hosts();
            self.candidates
                .update(ctx, |vm, ctx| vm.on_tree_changed(hosts, ctx));
            self.sync_candidate_row_states(ctx);
        }

        ctx.notify();
    }

    /// Keep the key sets of `candidate_row_states` / `candidate_add_states` in sync with the current
    /// candidates view-model's aliases. Surplus hover states are dropped (freeing memory),
    /// and missing aliases get a default state (so a newly added row doesn't lose state on its first hover).
    fn sync_candidate_row_states(&mut self, ctx: &mut ViewContext<Self>) {
        let aliases: Vec<String> = self
            .candidates
            .as_ref(ctx)
            .rows()
            .into_iter()
            .filter_map(|r| match r {
                CandidateRow::Candidate { alias, .. } => Some(alias),
                CandidateRow::Header { .. }
                | CandidateRow::NotFound { .. }
                | CandidateRow::Empty { .. }
                | CandidateRow::Error { .. } => None,
            })
            .collect();
        let alias_set: std::collections::HashSet<&str> =
            aliases.iter().map(|s| s.as_str()).collect();
        self.candidate_row_states
            .retain(|k, _| alias_set.contains(k.as_str()));
        self.candidate_add_states
            .retain(|k, _| alias_set.contains(k.as_str()));
        for a in aliases {
            self.candidate_row_states.entry(a.clone()).or_default();
            self.candidate_add_states.entry(a).or_default();
        }
    }

    /// Create a new folder. When `parent` is None, it is created at the root level.
    fn on_add_folder_with_parent(&mut self, parent: Option<String>, ctx: &mut ViewContext<Self>) {
        let result = warp_ssh_manager::with_conn(|c| {
            let name = unique_name(c, parent.as_deref(), "New folder")?;
            Ok(SshRepository::create_folder(c, parent.as_deref(), &name)?)
        });
        match result {
            Ok(node) => {
                let new_id = node.id.clone();
                self.selected_id = Some(new_id.clone());
                self.refresh_tree(ctx);
                // Rename right after creating — Drive convention.
                self.enter_rename(new_id, true, ctx);
            }
            Err(e) => {
                log::error!("ssh_manager: create folder failed: {e:?}");
                ctx.emit(SshManagerPanelEvent::PersistenceError(e.to_string()));
            }
        }
    }

    /// Import one candidate from `~/.ssh/config` as a new saved server.
    ///
    /// Field mapping follows TECH.md §3.3 / PRODUCT.md decision I/J/K strictly:
    /// - `server.host = alias` (preserves OpenSSH alias semantics, so launching `ssh` can still apply
    ///   `ProxyJump` and other directives from `~/.ssh/config`);
    /// - `port = candidate.port.unwrap_or(22)` (decision K's "port=None → 22");
    /// - `auth_type = Key if identity_file.is_some() else Password` (decision J);
    /// - `notes = "Imported from <resolved path>"` (so the user can later trace the source).
    ///
    /// Written via `SshRepository::create_server`, taking the same persistence path as a manual "New server"
    /// — so any schema change to that SQLite row is automatically followed by the import path.
    /// On completion it emits `OpenServerEditor` (same as manual creation) + broadcasts
    /// `SshTreeChangedEvent::TreeChanged` so the `Added` badge flips immediately.
    fn on_import_candidate(&mut self, alias: String, ctx: &mut ViewContext<Self>) {
        // Picking a suggestion is an explicit, deliberate add — close the add block.
        self.adding_mode = false;
        let candidate = self
            .candidates
            .read(ctx, |vm, _| vm.find_candidate(&alias).cloned());
        let Some(c) = candidate else {
            log::warn!("ssh_manager: ImportCandidate alias not found: {alias}");
            return;
        };
        let path_display = self
            .candidates
            .read(ctx, |vm, _| vm.path_display())
            .unwrap_or_default();

        let auth_type = if c.identity_file.is_some() {
            AuthType::Key
        } else {
            AuthType::Password
        };
        let info = SshServerInfo {
            node_id: String::new(),
            // PRODUCT.md decision I: store the alias rather than the resolved HostName.
            host: c.alias.clone(),
            port: c.port.unwrap_or(22),
            username: c.user.clone().unwrap_or_default(),
            auth_type,
            key_path: c
                .identity_file
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            credential_id: None,
            startup_command: None,
            notes: Some(format!("Imported from {path_display}")),
            last_connected_at: None,
            session_resilience: warp_ssh_manager::SessionResilience::default(),
            ring_ceiling_mb: 0,
        };

        let parent = self.parent_for_new_node();
        let result = warp_ssh_manager::with_conn(|conn| {
            // Same "auto-deduplicate" naming logic as the manual "New server" (unique_name);
            // the first candidate uses the alias as its name, and on collision appends " 2", " 3", …
            let name = unique_name(conn, parent.as_deref(), &c.alias)?;
            Ok(SshRepository::create_server(
                conn,
                parent.as_deref(),
                &name,
                &info,
            )?)
        });
        match result {
            Ok(node) => {
                let new_id = node.id.clone();
                self.selected_id = Some(new_id.clone());
                self.refresh_tree(ctx);
                // Consistent with manual creation: open the central editor pane so the user can fill in the password / tweak fields.
                ctx.emit(SshManagerPanelEvent::OpenServerEditor { node_id: new_id });
                // Broadcast the tree change — the Candidates section's added_aliases refreshes from it.
                SshTreeChangedNotifier::handle(ctx).update(ctx, |_, ctx| {
                    ctx.emit(SshTreeChangedEvent::TreeChanged);
                });
            }
            Err(e) => {
                log::error!("ssh_manager: import candidate failed: {e:?}");
                ctx.emit(SshManagerPanelEvent::PersistenceError(e.to_string()));
            }
        }
    }

    /// Toolbar "+" — toggle the guided "Add a host" block. When opening, re-read
    /// `~/.ssh/config` so the suggestions reflect the current file (the user may
    /// have edited it since the panel mounted). Closing is a pure UI toggle; it
    /// never touches the saved list.
    fn on_toggle_add_mode(&mut self, ctx: &mut ViewContext<Self>) {
        self.adding_mode = !self.adding_mode;
        if self.adding_mode {
            self.candidates.update(ctx, |vm, ctx| vm.refresh(ctx));
            self.sync_candidate_row_states(ctx);
        }
        ctx.notify();
    }

    fn on_add_server(&mut self, ctx: &mut ViewContext<Self>) {
        // Either path out of the add block (create blank / import) closes it.
        self.adding_mode = false;
        let parent = self.parent_for_new_node();
        let info_template = SshServerInfo::new_default(String::new());
        let result = warp_ssh_manager::with_conn(|c| {
            let name = unique_name(c, parent.as_deref(), "New server")?;
            Ok(SshRepository::create_server(
                c,
                parent.as_deref(),
                &name,
                &info_template,
            )?)
        });
        match result {
            Ok(node) => {
                let new_id = node.id.clone();
                self.selected_id = Some(new_id.clone());
                self.refresh_tree(ctx);
                // After creating a server, open the central editor pane (user fills in fields) — the name is edited
                // there together with the other fields, not inline in the tree.
                ctx.emit(SshManagerPanelEvent::OpenServerEditor { node_id: new_id });
            }
            Err(e) => {
                log::error!("ssh_manager: create server failed: {e:?}");
                ctx.emit(SshManagerPanelEvent::PersistenceError(e.to_string()));
            }
        }
    }

    fn on_clone_server(&mut self, source_id: &str, ctx: &mut ViewContext<Self>) {
        let source_id = source_id.to_string();
        let result = warp_ssh_manager::with_conn(|c| {
            let source_info = SshRepository::get_server(c, &source_id)?
                .ok_or_else(|| warp_ssh_manager::SshRepositoryError::NotFound(source_id.clone()))?;
            let source_node = SshRepository::list_nodes(c)?
                .into_iter()
                .find(|n| n.id == source_id)
                .ok_or_else(|| warp_ssh_manager::SshRepositoryError::NotFound(source_id.clone()))?;

            let parent = source_node.parent_id;
            let cloned_info = SshServerInfo::clone_from_template(&source_info, String::new());
            let name = unique_name(
                c,
                parent.as_deref(),
                &format!("{} (copy)", source_node.name),
            )?;

            let new_node = SshRepository::create_server(c, parent.as_deref(), &name, &cloned_info)?;

            // The source server was already verified to exist above; copy its keychain password / key passphrase directly to the new node.
            let store = KeychainSecretStore;
            if let Ok(Some(password)) = store.get(&source_id, SecretKind::Password) {
                let _ = store.set(&new_node.id, SecretKind::Password, &password);
            }
            if let Ok(Some(passphrase)) = store.get(&source_id, SecretKind::Passphrase) {
                let _ = store.set(&new_node.id, SecretKind::Passphrase, &passphrase);
            }

            Ok(new_node)
        });
        match result {
            Ok(node) => {
                let new_id = node.id.clone();
                self.selected_id = Some(new_id.clone());
                self.refresh_tree(ctx);
                ctx.emit(SshManagerPanelEvent::OpenServerEditor { node_id: new_id });
            }
            Err(e) => {
                log::error!("ssh_manager: clone server failed: {e:?}");
                ctx.emit(SshManagerPanelEvent::PersistenceError(e.to_string()));
            }
        }
    }

    fn on_delete_selected(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(id) = self.selected_id.clone() else {
            return;
        };
        let result = warp_ssh_manager::with_conn(|c| Ok(SshRepository::delete_node(c, &id)?));
        if let Err(e) = result {
            log::error!("ssh_manager: delete failed: {e:?}");
            ctx.emit(SshManagerPanelEvent::PersistenceError(e.to_string()));
            return;
        }
        let store = KeychainSecretStore;
        let _ = store.delete(&id, SecretKind::Password);
        let _ = store.delete(&id, SecretKind::Passphrase);
        let _ = store.delete(&id, SecretKind::RootPassword);

        self.selected_id = None;
        self.refresh_tree(ctx);
        SshTreeChangedNotifier::handle(ctx).update(ctx, |_, ctx| {
            ctx.emit(SshTreeChangedEvent::TreeChanged);
        });
    }

    fn on_connect(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(id) = self.selected_id.clone() else {
            return;
        };
        self.dispatch_connect_for(&id, ctx);
    }

    /// Right-click "SFTP browse": emit the OpenSftpPane event.
    fn on_open_sftp(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(id) = self.selected_id.clone() else {
            return;
        };
        let kind = self.nodes.iter().find(|n| n.id == id).map(|n| n.kind);
        if !matches!(kind, Some(NodeKind::Server)) {
            return;
        }
        let server = warp_ssh_manager::with_conn(|c| Ok(SshRepository::get_server(c, &id)?))
            .ok()
            .flatten();
        if let Some(server) = server {
            ctx.emit(SshManagerPanelEvent::OpenSftpPane {
                node_id: id,
                server,
            });
        }
    }

    fn dispatch_connect_for(&self, id: &str, ctx: &mut ViewContext<Self>) {
        let kind = self.nodes.iter().find(|n| n.id == id).map(|n| n.kind);
        if !matches!(kind, Some(NodeKind::Server)) {
            return;
        }
        let server = warp_ssh_manager::with_conn(|c| Ok(SshRepository::get_server(c, id)?))
            .ok()
            .flatten();
        if let Some(server) = server {
            ctx.emit(SshManagerPanelEvent::OpenSshTerminal {
                node_id: id.to_string(),
                server,
            });
        }
    }

    /// Whether `server` can host a persistent daemon session (key/onekey auth +
    /// session_resilience enabled) — only those have sessions to list / adopt.
    fn is_daemon_capable(server: &SshServerInfo) -> bool {
        server.session_resilience.is_enabled()
            && matches!(server.auth_type, AuthType::Key | AuthType::OneKey)
    }

    /// Toggle the inline running-sessions list for a server node; the first
    /// expand kicks off a connect-to-list fetch.
    fn on_toggle_sessions(&mut self, id: String, ctx: &mut ViewContext<Self>) {
        if self.sessions_expanded.remove(&id) {
            ctx.notify();
            return;
        }
        self.sessions_expanded.insert(id.clone());
        self.fetch_sessions(id, ctx);
        ctx.notify();
    }

    /// Fetches a server's running daemon sessions via connect-to-list and stores
    /// them in `host_sessions` (or records `sessions_error`).
    #[allow(unused_variables)]
    fn fetch_sessions(&mut self, id: String, ctx: &mut ViewContext<Self>) {
        let server = warp_ssh_manager::with_conn(|c| Ok(SshRepository::get_server(c, &id)?))
            .ok()
            .flatten();
        let Some(server) = server else {
            return;
        };
        self.sessions_error.remove(&id);

        #[cfg(unix)]
        {
            use crate::auth::AuthStateProvider;
            use crate::remote_server::auth_context::server_api_auth_context;
            use crate::remote_server::headless_connect;

            if !Self::is_daemon_capable(&server) {
                self.sessions_error.insert(
                    id,
                    crate::t!("workspace-left-panel-ssh-manager-sessions-not-persistent"),
                );
                return;
            }
            self.sessions_loading.insert(id.clone());
            let auth_context = std::sync::Arc::new(server_api_auth_context(
                AuthStateProvider::as_ref(ctx).get().clone(),
            ));
            let socket_path = headless_connect::control_socket_path(&server);
            let executor = ctx.background_executor().clone();
            ctx.spawn(
                headless_connect::list_daemon_sessions(server, socket_path, auth_context, executor),
                move |me, result, ctx| {
                    me.sessions_loading.remove(&id);
                    match result {
                        Ok(sessions) => {
                            me.host_sessions.insert(id, sessions);
                        }
                        Err(e) => {
                            me.sessions_error.insert(id, e);
                        }
                    }
                    ctx.notify();
                },
            );
        }
        #[cfg(not(unix))]
        {
            self.sessions_error.insert(
                id,
                "Daemon sessions are only supported on Unix hosts.".to_string(),
            );
        }
    }

    /// Adopt a listed daemon session in a new tab (attach + replay).
    fn on_adopt_session(
        &mut self,
        node_id: String,
        pty_session_id: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let server = warp_ssh_manager::with_conn(|c| Ok(SshRepository::get_server(c, &node_id)?))
            .ok()
            .flatten();
        if let Some(server) = server {
            ctx.emit(SshManagerPanelEvent::AdoptDaemonSession {
                server,
                pty_session_id,
            });
        }
    }

    fn on_edit(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(id) = self.selected_id.clone() else {
            return;
        };
        let kind = self.nodes.iter().find(|n| n.id == id).map(|n| n.kind);
        if !matches!(kind, Some(NodeKind::Server)) {
            // "Edit" on a folder = rename
            self.enter_rename(id, false, ctx);
            return;
        }
        ctx.emit(SshManagerPanelEvent::OpenServerEditor { node_id: id });
    }

    /// Double-click a server = connect (open a new tab). Double-clicking a folder = two toggles cancel out, a no-op.
    fn on_double_click(&mut self, id: String, ctx: &mut ViewContext<Self>) {
        let kind = self.nodes.iter().find(|n| n.id == id).map(|n| n.kind);
        if matches!(kind, Some(NodeKind::Server)) {
            self.dispatch_connect_for(&id, ctx);
        }
    }

    /// Toggle a single folder's collapsed state; server nodes are ignored.
    fn on_toggle_node_collapsed(&mut self, node_id: &str, ctx: &mut ViewContext<Self>) {
        let kind = self.nodes.iter().find(|n| n.id == node_id).map(|n| n.kind);
        if !matches!(kind, Some(NodeKind::Folder)) {
            return;
        }
        let new_collapsed = !self
            .nodes
            .iter()
            .find(|n| n.id == node_id)
            .map(|n| n.is_collapsed)
            .unwrap_or(false);
        let id = node_id.to_string();
        let result = warp_ssh_manager::with_conn(move |c| {
            Ok(SshRepository::set_collapsed(c, &id, new_collapsed)?)
        });
        if let Err(e) = result {
            log::error!("ssh_manager: toggle collapse failed: {e:?}");
            ctx.emit(SshManagerPanelEvent::PersistenceError(e.to_string()));
            return;
        }
        self.refresh_tree(ctx);
        SshTreeChangedNotifier::handle(ctx).update(ctx, |_, ctx| {
            ctx.emit(SshTreeChangedEvent::TreeChanged);
        });
    }

    /// Top button: if any folder is currently expanded → collapse all; if all are already collapsed → expand all.
    fn on_toggle_all_folders(&mut self, ctx: &mut ViewContext<Self>) {
        let any_expanded = self
            .nodes
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Folder) && !n.is_collapsed);
        let new_collapsed = any_expanded; // at least one expanded → collapse all; otherwise expand all
        let result = warp_ssh_manager::with_conn(|c| {
            Ok(SshRepository::set_all_folders_collapsed(c, new_collapsed)?)
        });
        if let Err(e) = result {
            log::error!("ssh_manager: toggle all failed: {e:?}");
            ctx.emit(SshManagerPanelEvent::PersistenceError(e.to_string()));
            return;
        }
        self.refresh_tree(ctx);
        SshTreeChangedNotifier::handle(ctx).update(ctx, |_, ctx| {
            ctx.emit(SshTreeChangedEvent::TreeChanged);
        });
    }

    /// Whether the node is visually visible — hidden if any ancestor folder is collapsed.
    /// Root-level nodes are always visible.
    fn is_visible(&self, node: &SshNode) -> bool {
        let mut cursor = node.parent_id.as_deref();
        while let Some(pid) = cursor {
            let parent = match self.nodes.iter().find(|n| n.id == pid) {
                Some(p) => p,
                None => return true, // data inconsistency; show it to be safe
            };
            if matches!(parent.kind, NodeKind::Folder) && parent.is_collapsed {
                return false;
            }
            cursor = parent.parent_id.as_deref();
        }
        true
    }

    fn on_click(&mut self, id: String, ctx: &mut ViewContext<Self>) {
        // Clicking another row = exit the current rename (commit)
        if self
            .rename_state
            .as_ref()
            .map(|rs| rs.node_id != id)
            .unwrap_or(false)
        {
            self.commit_rename(ctx);
        }

        // commit_rename clears selected_id for a newly created folder (is_newly_created), but the semantics of a
        // single click are to select the clicked item, so immediately overwriting with the new id here is expected
        // — the clear only applies to exit paths with no new selection (Enter/ESC/blur to empty space); a click
        // itself already provides a new selection context.
        self.selected_id = Some(id.clone());
        // Navigating the tree dismisses the guided add block — it's only relevant
        // while the user is actively adding a host from the toolbar.
        self.adding_mode = false;
        let kind = self.nodes.iter().find(|n| n.id == id).map(|n| n.kind);
        match kind {
            Some(NodeKind::Server) => {
                // Click a server = select only. **Connecting is on double-click** (`on_double_click`).
            }
            Some(NodeKind::Folder) => {
                // Click a folder = toggle collapse/expand (selection already done above)
                self.on_toggle_node_collapsed(&id, ctx);
                return; // on_toggle already calls ctx.notify internally
            }
            None => {}
        }
        ctx.notify();
    }

    fn on_open_context_menu(
        &mut self,
        target: Option<String>,
        position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        // Close rename before opening the menu (otherwise the rename buffer is lost).
        if self.rename_state.is_some() {
            self.commit_rename(ctx);
        }
        if let Some(t) = target.as_ref() {
            self.selected_id = Some(t.clone());
        } else {
            // Right-clicking empty space means operating at the root level; clear the old selection state.
            self.selected_id = None;
        }
        self.context_menu_target = target;
        self.context_menu_position = Some(position);
        ctx.notify();
    }

    fn on_dismiss_context_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.context_menu_position = None;
        self.context_menu_target = None;
        ctx.notify();
    }

    fn enter_rename(
        &mut self,
        node_id: String,
        is_newly_created: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let current_name = self
            .nodes
            .iter()
            .find(|n| n.id == node_id)
            .map(|n| n.name.clone())
            .unwrap_or_default();

        let editor = ctx.add_typed_action_view(move |ctx| {
            let appearance = warp_core::ui::appearance::Appearance::as_ref(ctx);
            let theme = appearance.theme();
            let options = SingleLineEditorOptions {
                is_password: false,
                text: TextOptions {
                    font_size_override: Some(appearance.ui_font_subheading()),
                    font_family_override: Some(appearance.ui_font_family()),
                    text_colors_override: Some(TextColors {
                        default_color: theme.active_ui_text_color(),
                        disabled_color: theme.disabled_ui_text_color(),
                        hint_color: theme.disabled_ui_text_color(),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_buffer_text(&current_name, ctx);
            editor
        });

        // Listen for Enter / Blurred → commit; Escape → cancel.
        ctx.subscribe_to_view(&editor, |me, _, event, ctx| match event {
            EditorEvent::Enter => me.commit_rename(ctx),
            EditorEvent::Blurred => me.commit_rename(ctx),
            EditorEvent::Escape => me.cancel_rename(ctx),
            _ => {}
        });

        ctx.focus(&editor);
        self.rename_state = Some(RenameState {
            node_id,
            editor,
            is_newly_created,
        });
        ctx.notify();
    }

    fn commit_rename(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(rs) = self.rename_state.take() else {
            return;
        };
        let new_name = rs.editor.as_ref(ctx).buffer_text(ctx).trim().to_string();
        let id = rs.node_id.clone();
        let was_newly_created = rs.is_newly_created;
        if new_name.is_empty() {
            // The name cannot be empty: cancel. For a newly created folder, also clear the selection.
            if was_newly_created {
                self.selected_id = None;
            }
            ctx.notify();
            return;
        }
        let result =
            warp_ssh_manager::with_conn(|c| Ok(SshRepository::rename_node(c, &id, &new_name)?));
        if let Err(e) = result {
            log::error!("ssh_manager: rename failed: {e:?}");
            ctx.emit(SshManagerPanelEvent::PersistenceError(e.to_string()));
            return;
        }
        // After finishing the rename of a newly created folder, clear the selection so the next "New folder" is created at the root level.
        if was_newly_created {
            self.selected_id = None;
        }
        self.refresh_tree(ctx);
        SshTreeChangedNotifier::handle(ctx).update(ctx, |_, ctx| {
            ctx.emit(SshTreeChangedEvent::TreeChanged);
        });
    }

    fn cancel_rename(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(rs) = self.rename_state.take() {
            // After canceling the rename of a newly created folder, clear the selection so the next "New folder" is created at the root level.
            if rs.is_newly_created {
                self.selected_id = None;
            }
        }
        ctx.notify();
    }

    /// Check whether moving `dragged` under `new_parent` would create a cycle (also reject outright
    /// when `new_parent` is a descendant of `dragged` / is itself / is already the current parent, saving a write).
    fn move_is_legal(&self, dragged: &str, new_parent: Option<&str>) -> bool {
        // Move under itself: forbidden
        if Some(dragged) == new_parent {
            return false;
        }
        // No movement: reject (avoid an idempotent write)
        let current_parent = self
            .nodes
            .iter()
            .find(|n| n.id == dragged)
            .and_then(|n| n.parent_id.as_deref());
        if current_parent == new_parent {
            return false;
        }
        // Move a folder under one of its own descendants: forbidden (cycle)
        if let Some(target_parent) = new_parent {
            let mut cursor = Some(target_parent);
            while let Some(id) = cursor {
                if id == dragged {
                    return false;
                }
                cursor = self
                    .nodes
                    .iter()
                    .find(|n| n.id == id)
                    .and_then(|n| n.parent_id.as_deref());
            }
        }
        true
    }

    fn on_move_node(
        &mut self,
        node_id: String,
        new_parent_id: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.move_is_legal(&node_id, new_parent_id.as_deref()) {
            // Bumped to warn: when a drag has no visible effect, this log is easier to find than debug.
            // Most `false` results come from "dropping onto the current parent / dropping onto itself".
            let current_parent = self
                .nodes
                .iter()
                .find(|n| n.id == node_id)
                .and_then(|n| n.parent_id.clone());
            log::warn!(
                "ssh_manager: move rejected. node={node_id} current_parent={current_parent:?} target_parent={new_parent_id:?}"
            );
            return;
        }
        // sort_order takes the target parent's current max +1 (placed at the end). A simpler approach:
        // use i32::MAX/2 to let the SQL layer put it last (normalize later). Here we run a SQL
        // query to get the real next_sort_order.
        let result = warp_ssh_manager::with_conn(|c| {
            use diesel::prelude::*;
            use persistence::schema::ssh_nodes;
            let max: Option<i32> = match new_parent_id.as_deref() {
                Some(p) => ssh_nodes::table
                    .filter(ssh_nodes::parent_id.eq(p))
                    .select(diesel::dsl::max(ssh_nodes::sort_order))
                    .first(c)?,
                None => ssh_nodes::table
                    .filter(ssh_nodes::parent_id.is_null())
                    .select(diesel::dsl::max(ssh_nodes::sort_order))
                    .first(c)?,
            };
            let next_sort = max.unwrap_or(-1) + 1;
            Ok(SshRepository::move_node(
                c,
                &node_id,
                new_parent_id.as_deref(),
                next_sort,
            )?)
        });
        if let Err(e) = result {
            log::error!("ssh_manager: move failed: {e:?}");
            ctx.emit(SshManagerPanelEvent::PersistenceError(e.to_string()));
            return;
        }
        self.refresh_tree(ctx);
        SshTreeChangedNotifier::handle(ctx).update(ctx, |_, ctx| {
            ctx.emit(SshTreeChangedEvent::TreeChanged);
        });
    }

    fn parent_for_new_node(&self) -> Option<String> {
        resolve_parent_for_new_node(self.selected_id.as_deref(), &self.nodes)
    }

    fn render_toolbar(
        &self,
        appearance: &warp_core::ui::appearance::Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let icon_color = theme.sub_text_color(theme.background());

        let make_btn = |icon: crate::ui_components::icons::Icon,
                        state: MouseStateHandle,
                        action: SshManagerPanelAction|
         -> Box<dyn Element> {
            let icon_el = ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
                .with_width(TOOLBAR_ICON_SIZE)
                .with_height(TOOLBAR_ICON_SIZE)
                .finish();
            Hoverable::new(state, move |_| {
                Container::new(
                    ConstrainedBox::new(icon_el)
                        .with_width(TOOLBAR_BUTTON_SIZE)
                        .with_height(TOOLBAR_BUTTON_SIZE)
                        .finish(),
                )
                .with_uniform_padding(2.0)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action.clone());
            })
            .finish()
        };

        // Left group: create buttons
        let left_group = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.0)
            .with_child(make_btn(
                crate::ui_components::icons::Icon::Folder,
                self.add_folder_btn.clone(),
                SshManagerPanelAction::AddRootFolder,
            ))
            .with_child(make_btn(
                crate::ui_components::icons::Icon::Plus,
                self.add_server_btn.clone(),
                SshManagerPanelAction::ToggleAddMode,
            ))
            .with_main_axis_size(MainAxisSize::Min)
            .finish();

        // Right side: the collapse/expand-all button — smart toggle. If any folder is currently expanded → show
        // ChevronUp (meaning "collapse"), otherwise show ChevronDown (meaning "expand").
        let any_expanded = self
            .nodes
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Folder) && !n.is_collapsed);
        let toggle_icon = if any_expanded {
            crate::ui_components::icons::Icon::ChevronUp
        } else {
            crate::ui_components::icons::Icon::ChevronDown
        };
        let right_group = make_btn(
            toggle_icon,
            self.toggle_all_btn.clone(),
            SshManagerPanelAction::ToggleAllFolders,
        );

        // The whole toolbar: align both ends (MainAxisAlignment::SpaceBetween).
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(warpui::elements::MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(left_group)
            .with_child(right_group)
            .finish()
    }

    /// Guided "Add a host" block (toolbar "+"). Renders a prominent
    /// "Create a blank server" action plus the on-demand `~/.ssh/config`
    /// suggestions (`render_candidates`, which renders nothing when
    /// auto-discovery is off or the config has no importable hosts).
    ///
    /// Shown only while `adding_mode` is true; the saved tree below stays
    /// untouched until the user explicitly creates or imports.
    fn render_add_block(
        &self,
        appearance: &warp_core::ui::appearance::Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let muted = theme.sub_text_color(theme.background());
        let main = theme.main_text_color(theme.background());
        let icon_color = muted;

        // Header: "Add a host" + a Cancel button on the right.
        let heading = Text::new_inline(
            crate::t!("workspace-left-panel-ssh-manager-add-heading"),
            appearance.ui_font_family(),
            appearance.ui_font_subheading(),
        )
        .with_color(muted.into())
        .finish();
        let cancel_label = Text::new_inline(
            crate::t!("workspace-left-panel-ssh-manager-add-cancel"),
            appearance.ui_font_family(),
            appearance.ui_font_body(),
        )
        .with_color(muted.into())
        .finish();
        let cancel_btn = Hoverable::new(self.add_cancel_btn.clone(), move |mouse| {
            let mut c = Container::new(cancel_label)
                .with_padding_top(ITEM_PADDING_VERTICAL)
                .with_padding_bottom(ITEM_PADDING_VERTICAL)
                .with_padding_left(ITEM_PADDING_HORIZONTAL)
                .with_padding_right(ITEM_PADDING_HORIZONTAL)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
            if mouse.is_hovered() {
                c = c.with_background(internal_colors::fg_overlay_3(theme));
            }
            c.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(SshManagerPanelAction::ToggleAddMode);
        })
        .finish();
        let header_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(
                Container::new(heading)
                    .with_padding_top(ITEM_PADDING_VERTICAL)
                    .with_padding_bottom(ITEM_PADDING_VERTICAL)
                    .with_padding_left(ITEM_PADDING_HORIZONTAL)
                    .finish(),
            )
            .with_child(cancel_btn)
            .finish();

        // Primary action: create a blank server (the manual path) and open its editor.
        let plus_icon = ConstrainedBox::new(
            crate::ui_components::icons::Icon::Plus
                .to_warpui_icon(icon_color)
                .finish(),
        )
        .with_width(ITEM_ICON_SIZE)
        .with_height(ITEM_ICON_SIZE)
        .finish();
        let blank_label = Text::new_inline(
            crate::t!("workspace-left-panel-ssh-manager-add-blank"),
            appearance.ui_font_family(),
            appearance.ui_font_subheading(),
        )
        .with_color(main.into())
        .finish();
        let blank_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(ITEM_ICON_TEXT_SPACING)
            .with_child(plus_icon)
            .with_child(blank_label)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .finish();
        let blank_btn = Hoverable::new(self.add_blank_btn.clone(), move |mouse| {
            let mut c = Container::new(blank_row)
                .with_padding_top(ITEM_PADDING_VERTICAL)
                .with_padding_bottom(ITEM_PADDING_VERTICAL)
                .with_padding_left(ITEM_PADDING_HORIZONTAL)
                .with_padding_right(ITEM_PADDING_HORIZONTAL)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
            if mouse.is_hovered() {
                c = c.with_background(internal_colors::fg_overlay_3(theme));
            }
            c.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(SshManagerPanelAction::AddServer);
        })
        .finish();

        let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        col.add_child(header_row);
        col.add_child(blank_btn);
        // Suggestions from ~/.ssh/config — renders nothing when auto-discovery is
        // off or the config has no importable hosts. When present, give it a small
        // top margin so the "from ~/.ssh/config" suggestions read as a distinct
        // group from the blank-server CTA above (no dangling gap when empty).
        if !self.candidates.as_ref(app).rows().is_empty() {
            col.add_child(
                Container::new(self.render_candidates(appearance, app))
                    .with_margin_top(ITEM_PADDING_VERTICAL)
                    .finish(),
            );
        }
        col.with_main_axis_size(MainAxisSize::Min).finish()
    }

    /// "Candidates" section — the list of importable hosts parsed from `~/.ssh/config`.
    ///
    /// Rendered inside the guided "Add a host" block (`render_add_block`), shown only while the user is
    /// actively adding. Its layout style (row height, indent, font size) matches the tree, with just an extra
    /// Refresh button + collapse chevron in the section header. Each candidate row ends with a "+" or "Added"
    /// badge (PRODUCT.md decision E). Returns Empty when auto-discovery is off or the config has no hosts.
    fn render_candidates(
        &self,
        appearance: &warp_core::ui::appearance::Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let rows = self.candidates.as_ref(app).rows();
        if rows.is_empty() {
            // refresh has not been called yet — don't render the section at all (shouldn't happen once the panel mounts,
            // since new() calls it immediately, but kept as a safety fallback).
            return Empty::new().finish();
        }

        let muted = theme.sub_text_color(theme.background());
        let main = theme.main_text_color(theme.background());

        let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        for row in rows {
            match row {
                CandidateRow::Header {
                    path_display,
                    count,
                    can_refresh,
                } => {
                    col.add_child(self.render_candidates_header(
                        &path_display,
                        count,
                        can_refresh,
                        appearance,
                        app,
                    ));
                }
                CandidateRow::NotFound { path_display } => {
                    col.add_child(self.render_candidates_message(
                        &crate::t!(
                            "workspace-left-panel-ssh-manager-candidates-not-found",
                            path = path_display
                        ),
                        muted,
                        appearance,
                    ));
                }
                CandidateRow::Empty { path_display } => {
                    col.add_child(self.render_candidates_message(
                        &crate::t!(
                            "workspace-left-panel-ssh-manager-candidates-empty",
                            path = path_display
                        ),
                        muted,
                        appearance,
                    ));
                }
                CandidateRow::Error {
                    path_display,
                    message,
                } => {
                    // Error rows use the error red — `ui_error_color` returns a ColorU directly,
                    // the same approach as the "over-limit character counter" in `ai_assistant/panel.rs`.
                    let err_color: pathfinder_color::ColorU = theme.ui_error_color();
                    col.add_child(self.render_candidates_message_color(
                        &crate::t!(
                            "workspace-left-panel-ssh-manager-candidates-error",
                            path = path_display,
                            error = message
                        ),
                        err_color,
                        appearance,
                    ));
                }
                CandidateRow::Candidate {
                    alias,
                    hostname,
                    user,
                    port,
                    identity_file: _,
                    added,
                } => {
                    col.add_child(self.render_candidate_row(
                        CandidateRowFields {
                            alias: &alias,
                            hostname: hostname.as_deref(),
                            user: user.as_deref(),
                            port,
                            added,
                        },
                        CandidateRowColors { main, muted },
                        appearance,
                    ));
                }
            }
        }

        col.with_main_axis_size(MainAxisSize::Min).finish()
    }

    fn render_candidates_header(
        &self,
        path_display: &str,
        count: usize,
        can_refresh: bool,
        appearance: &warp_core::ui::appearance::Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let icon_color = theme.sub_text_color(theme.background());
        let muted = theme.sub_text_color(theme.background());

        // Collapsed-state chevron (▶) vs expanded-state (▼) — is_expanded comes straight from the view-model.
        let expanded = self.candidates.as_ref(app).is_expanded();
        let chevron_icon = if expanded {
            crate::ui_components::icons::Icon::ChevronDown
        } else {
            crate::ui_components::icons::Icon::ChevronRight
        };
        let chevron_el = ConstrainedBox::new(chevron_icon.to_warpui_icon(icon_color).finish())
            .with_width(ITEM_ICON_SIZE)
            .with_height(ITEM_ICON_SIZE)
            .finish();

        let header_text = crate::t!(
            "workspace-left-panel-ssh-manager-candidates-header",
            path = path_display
        );
        let label = Text::new_inline(
            header_text,
            appearance.ui_font_family(),
            appearance.ui_font_subheading(),
        )
        .with_color(muted.into())
        .finish();

        let count_label = Text::new_inline(
            format!("({count})"),
            appearance.ui_font_family(),
            appearance.ui_font_body(),
        )
        .with_color(muted.into())
        .finish();

        // Right-side Refresh button — refresh is allowed in any state (NotFound / Error / Loaded).
        let refresh_state = self.candidates_refresh_btn.clone();
        let refresh_icon = ConstrainedBox::new(
            crate::ui_components::icons::Icon::Refresh
                .to_warpui_icon(icon_color)
                .finish(),
        )
        .with_width(ITEM_ICON_SIZE)
        .with_height(ITEM_ICON_SIZE)
        .finish();
        let refresh_btn = if can_refresh {
            Hoverable::new(refresh_state, move |_| {
                Container::new(refresh_icon)
                    .with_uniform_padding(2.0)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                    .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(SshManagerPanelAction::RefreshCandidates);
            })
            .finish()
        } else {
            refresh_icon
        };

        // chevron + label + count grouped at the left; the Refresh button pinned to
        // the right edge via SpaceBetween (same pattern as render_toolbar) so the
        // trailing action right-aligns instead of floating after the label.
        let left_group = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(ITEM_ICON_TEXT_SPACING)
            .with_child(chevron_el)
            .with_child(label)
            .with_child(count_label)
            .with_main_axis_size(MainAxisSize::Min)
            .finish();
        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(left_group)
            .with_child(refresh_btn)
            .finish();

        // Clicking the whole header = toggle (similar to a folder row's single-click).
        let toggle_state = self.candidates_toggle_btn.clone();
        Hoverable::new(toggle_state, move |mouse| {
            let mut c = Container::new(row)
                .with_padding_top(ITEM_PADDING_VERTICAL)
                .with_padding_bottom(ITEM_PADDING_VERTICAL)
                .with_padding_left(ITEM_PADDING_HORIZONTAL)
                .with_padding_right(ITEM_PADDING_HORIZONTAL)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
            if mouse.is_hovered() {
                c = c.with_background(internal_colors::fg_overlay_3(theme));
            }
            c.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(SshManagerPanelAction::ToggleCandidatesSection);
        })
        .finish()
    }

    fn render_candidates_message(
        &self,
        text: &str,
        color: warp_core::ui::theme::Fill,
        appearance: &warp_core::ui::appearance::Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            Text::new_inline(
                text.to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_body(),
            )
            .with_color(color.into())
            .finish(),
        )
        .with_padding_top(ITEM_PADDING_VERTICAL)
        .with_padding_bottom(ITEM_PADDING_VERTICAL)
        .with_padding_left(ITEM_PADDING_HORIZONTAL + FOLDER_DEPTH_INDENT)
        .with_padding_right(ITEM_PADDING_HORIZONTAL)
        .finish()
    }

    /// Same as `render_candidates_message`, but takes a `ColorU` — Error rows use the red returned directly by
    /// the theme's `ui_error_color()`, avoiding another Fill wrapping.
    fn render_candidates_message_color(
        &self,
        text: &str,
        color: pathfinder_color::ColorU,
        appearance: &warp_core::ui::appearance::Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            Text::new_inline(
                text.to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_body(),
            )
            .with_color(color)
            .finish(),
        )
        .with_padding_top(ITEM_PADDING_VERTICAL)
        .with_padding_bottom(ITEM_PADDING_VERTICAL)
        .with_padding_left(ITEM_PADDING_HORIZONTAL + FOLDER_DEPTH_INDENT)
        .with_padding_right(ITEM_PADDING_HORIZONTAL)
        .finish()
    }

    fn render_candidate_row(
        &self,
        fields: CandidateRowFields<'_>,
        colors: CandidateRowColors,
        appearance: &warp_core::ui::appearance::Appearance,
    ) -> Box<dyn Element> {
        let CandidateRowFields {
            alias,
            hostname,
            user,
            port,
            added,
        } = fields;
        let CandidateRowColors { main, muted } = colors;
        let theme = appearance.theme();
        let icon = crate::ui_components::icons::Icon::Key
            .to_warpui_icon(theme.sub_text_color(theme.background()))
            .finish();
        let icon_el = ConstrainedBox::new(icon)
            .with_width(ITEM_ICON_SIZE)
            .with_height(ITEM_ICON_SIZE)
            .finish();

        // Main label = alias; subtitle = the "user@hostname:port" shorthand, both assembled from optionals.
        // When imported, the whole row's font color is dimmed (decision E: dimmed).
        let label_color = if added { muted } else { main };
        let alias_text = Text::new_inline(
            alias.to_string(),
            appearance.ui_font_family(),
            appearance.ui_font_subheading(),
        )
        .with_color(label_color.into())
        .finish();

        let mut subtitle_parts: Vec<String> = Vec::new();
        if let Some(u) = user {
            subtitle_parts.push(u.to_string());
        }
        if let Some(h) = hostname {
            // user@host; show only host when there is no user
            let last = subtitle_parts.last_mut();
            match last {
                Some(s) => *s = format!("{s}@{h}"),
                None => subtitle_parts.push(h.to_string()),
            }
        }
        if let Some(p) = port {
            // Append :port to the end of the last segment; if both user and hostname are missing, use :port alone.
            if let Some(last) = subtitle_parts.last_mut() {
                *last = format!("{last}:{p}");
            } else {
                subtitle_parts.push(format!(":{p}"));
            }
        }
        let subtitle: Option<Box<dyn Element>> = if subtitle_parts.is_empty() {
            None
        } else {
            Some(
                Text::new_inline(
                    subtitle_parts.join(" "),
                    appearance.ui_font_family(),
                    appearance.ui_font_body(),
                )
                .with_color(muted.into())
                .finish(),
            )
        };

        let mut label_col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(alias_text);
        if let Some(s) = subtitle {
            label_col.add_child(s);
        }
        let label_block = label_col.with_main_axis_size(MainAxisSize::Min).finish();

        // The trailing "+" button or "Added" badge.
        let add_state = self
            .candidate_add_states
            .get(alias)
            .cloned()
            .unwrap_or_default();
        let alias_for_click = alias.to_string();
        let trailing: Box<dyn Element> = if added {
            // PRODUCT.md decision E: imported → show "Added" (no click interaction).
            Text::new_inline(
                crate::t!("workspace-left-panel-ssh-manager-candidates-added"),
                appearance.ui_font_family(),
                appearance.ui_font_body(),
            )
            .with_color(muted.into())
            .finish()
        } else {
            let plus_icon = ConstrainedBox::new(
                crate::ui_components::icons::Icon::Plus
                    .to_warpui_icon(theme.sub_text_color(theme.background()))
                    .finish(),
            )
            .with_width(ITEM_ICON_SIZE)
            .with_height(ITEM_ICON_SIZE)
            .finish();
            Hoverable::new(add_state, move |_| {
                Container::new(plus_icon)
                    .with_uniform_padding(2.0)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                    .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SshManagerPanelAction::ImportCandidate {
                    alias: alias_for_click.clone(),
                });
            })
            .finish()
        };

        // indent + icon + label grouped at the left; the trailing "+"/"Added"
        // pinned to the right edge via SpaceBetween, so it right-aligns instead of
        // floating right after the label.
        let left_group = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(ITEM_ICON_TEXT_SPACING)
            .with_child(
                ConstrainedBox::new(Empty::new().finish())
                    .with_width(FOLDER_DEPTH_INDENT)
                    .finish(),
            )
            .with_child(icon_el)
            .with_child(label_block)
            .with_main_axis_size(MainAxisSize::Min)
            .finish();
        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(left_group)
            .with_child(trailing)
            .finish();

        let row_state = self
            .candidate_row_states
            .get(alias)
            .cloned()
            .unwrap_or_default();
        Hoverable::new(row_state, move |mouse| {
            let mut c = Container::new(row)
                .with_padding_top(ITEM_PADDING_VERTICAL)
                .with_padding_bottom(ITEM_PADDING_VERTICAL)
                .with_padding_left(ITEM_PADDING_HORIZONTAL)
                .with_padding_right(ITEM_PADDING_HORIZONTAL)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
            if mouse.is_hovered() {
                c = c.with_background(internal_colors::fg_overlay_3(theme));
            }
            c.finish()
        })
        .finish()
    }

    /// Renders the inline running-daemon-session rows shown under an expanded
    /// host: a loading / error / empty message, or one clickable row per session
    /// (click → adopt). Indented one level past the host row.
    fn render_session_rows(
        &self,
        node: &SshNode,
        appearance: &warp_core::ui::appearance::Appearance,
    ) -> Vec<Box<dyn Element>> {
        let theme = appearance.theme();
        let muted: pathfinder_color::ColorU = theme.sub_text_color(theme.background()).into();
        let depth = self.depths.get(&node.id).copied().unwrap_or(0);
        // Align the session title under the host *name*: the tree row places its
        // label after the depth indent + chevron + icon (each ITEM_ICON_SIZE) with
        // ITEM_ICON_TEXT_SPACING between, so a child session lines up on that grid.
        let indent =
            depth as f32 * FOLDER_DEPTH_INDENT + 2.0 * ITEM_ICON_SIZE + 2.0 * ITEM_ICON_TEXT_SPACING;

        let message = |text: String, color: pathfinder_color::ColorU| -> Box<dyn Element> {
            Container::new(
                Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_body())
                    .with_color(color)
                    .finish(),
            )
            .with_padding_top(ITEM_PADDING_VERTICAL)
            .with_padding_bottom(ITEM_PADDING_VERTICAL)
            .with_padding_left(indent)
            .with_padding_right(ITEM_PADDING_HORIZONTAL)
            .with_margin_bottom(ITEM_MARGIN_BOTTOM)
            .finish()
        };

        if self.sessions_loading.contains(&node.id) {
            return vec![message(
                crate::t!("workspace-left-panel-ssh-manager-sessions-loading"),
                muted,
            )];
        }
        if let Some(err) = self.sessions_error.get(&node.id) {
            // A failed session fetch is an error — render it in the theme's error
            // color, matching the candidates error row (no glyph needed).
            return vec![message(err.clone(), theme.ui_error_color())];
        }
        let sessions = match self.host_sessions.get(&node.id) {
            Some(sessions) if !sessions.is_empty() => sessions,
            _ => {
                return vec![message(
                    crate::t!("workspace-left-panel-ssh-manager-sessions-empty"),
                    muted,
                )]
            }
        };

        sessions
            .iter()
            .map(|session| {
                let key = format!("{}:{}", node.id, session.session_id);
                let state = self
                    .session_row_states
                    .get(&key)
                    .cloned()
                    .unwrap_or_default();
                let node_id = node.id.clone();
                let pty_session_id = session.session_id.clone();
                let title = if !session.title.is_empty() {
                    session.title.clone()
                } else if !session.cwd.is_empty() {
                    session.cwd.clone()
                } else {
                    pty_session_id.clone()
                };
                let row = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(ITEM_ICON_TEXT_SPACING)
                    .with_child(
                        ConstrainedBox::new(Empty::new().finish())
                            .with_width(indent)
                            .finish(),
                    )
                    .with_child(
                        Text::new_inline(
                            title,
                            appearance.ui_font_family(),
                            appearance.ui_font_subheading(),
                        )
                        .with_color(theme.main_text_color(theme.background()).into())
                        .finish(),
                    )
                    .with_main_axis_size(MainAxisSize::Max)
                    .finish();
                Hoverable::new(state, move |mouse| {
                    let mut c = Container::new(row)
                        .with_padding_top(ITEM_PADDING_VERTICAL)
                        .with_padding_bottom(ITEM_PADDING_VERTICAL)
                        .with_padding_left(ITEM_PADDING_HORIZONTAL)
                        .with_padding_right(ITEM_PADDING_HORIZONTAL)
                        .with_margin_bottom(ITEM_MARGIN_BOTTOM)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
                    if mouse.is_hovered() {
                        c = c.with_background(internal_colors::fg_overlay_3(theme));
                    }
                    c.finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(SshManagerPanelAction::AdoptSession {
                        node_id: node_id.clone(),
                        pty_session_id: pty_session_id.clone(),
                    });
                })
                .finish()
            })
            .collect()
    }

    fn render_tree(&self, appearance: &warp_core::ui::appearance::Appearance) -> Box<dyn Element> {
        let mut col = Flex::column();

        if self.nodes.is_empty() {
            let theme = appearance.theme();
            let muted = theme.sub_text_color(theme.background());
            col.add_child(
                Container::new(
                    Text::new_inline(
                        crate::t!("workspace-left-panel-ssh-manager-tree-empty"),
                        appearance.ui_font_family(),
                        appearance.ui_font_subheading(),
                    )
                    .with_color(muted.into())
                    .finish(),
                )
                .with_padding_top(20.0)
                .with_padding_bottom(20.0)
                .with_padding_left(ITEM_PADDING_HORIZONTAL)
                .with_padding_right(ITEM_PADDING_HORIZONTAL)
                .finish(),
            );
        } else {
            for node in &self.nodes {
                if !self.is_visible(node) {
                    continue;
                }
                col.add_child(self.render_row(node, appearance));
                // Adopt-sidebar: inline running daemon sessions under an expanded host.
                if matches!(node.kind, NodeKind::Server)
                    && self.sessions_expanded.contains(&node.id)
                {
                    for child in self.render_session_rows(node, appearance) {
                        col.add_child(child);
                    }
                }
            }
        }
        let inner = col
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .finish();
        // Right-click on empty space = OpenContextMenu with node None.
        let hoverable = Hoverable::new(MouseStateHandle::default(), move |_| inner)
            .on_right_click(|ctx, _, position| {
                let offset = match ctx.element_position_by_id(SSH_PANEL_POSITION_ID) {
                    Some(bounds) => position - bounds.origin(),
                    None => position,
                };
                ctx.dispatch_typed_action(SshManagerPanelAction::OpenContextMenu {
                    target: None,
                    position: offset,
                });
            })
            .finish();
        // The whole tree area is also a drop target; parent_id=None means dropping to root.
        // Row-level DropTargets have higher priority (smaller), so dropping onto a folder still goes into the folder.
        DropTarget::new(hoverable, SshDropData { parent_id: None }).finish()
    }

    fn render_row(
        &self,
        node: &SshNode,
        appearance: &warp_core::ui::appearance::Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let depth = self.depths.get(&node.id).copied().unwrap_or(0);
        let is_selected = self.selected_id.as_deref() == Some(node.id.as_str());
        let is_renaming = self
            .rename_state
            .as_ref()
            .map(|rs| rs.node_id == node.id)
            .unwrap_or(false);

        let icon = match node.kind {
            NodeKind::Folder => crate::ui_components::icons::Icon::Folder,
            NodeKind::Server => crate::ui_components::icons::Icon::Key,
        };
        let icon_color = theme.sub_text_color(theme.background());
        let icon_el = ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
            .with_width(ITEM_ICON_SIZE)
            .with_height(ITEM_ICON_SIZE)
            .finish();

        // Folder rows get a leading chevron (▼ expanded / ▶ collapsed); Server rows use equal-width blank padding
        // so all rows' icons line up.
        let chevron_el: Box<dyn Element> = match node.kind {
            NodeKind::Folder => {
                let chevron_icon = if node.is_collapsed {
                    crate::ui_components::icons::Icon::ChevronRight
                } else {
                    crate::ui_components::icons::Icon::ChevronDown
                };
                ConstrainedBox::new(chevron_icon.to_warpui_icon(icon_color).finish())
                    .with_width(ITEM_ICON_SIZE)
                    .with_height(ITEM_ICON_SIZE)
                    .finish()
            }
            NodeKind::Server => ConstrainedBox::new(Empty::new().finish())
                .with_width(ITEM_ICON_SIZE)
                .finish(),
        };

        // Right half — text or the rename input box.
        // EditorView must be rendered inside a finite-width container, otherwise element.rs:1670 will
        // panic("infinite width constraint on buffer elements"). A Flex::row child
        // has no column-stretch semantics, so wrap it in a ConstrainedBox to give a fixed width.
        let label_or_editor: Box<dyn Element> = if is_renaming {
            let editor_handle = self
                .rename_state
                .as_ref()
                .map(|rs| rs.editor.clone())
                .expect("is_renaming implies rename_state.is_some");
            let input = appearance
                .ui_builder()
                .text_input(editor_handle)
                .with_style(UiComponentStyles {
                    padding: Some(Coords {
                        left: 4.0,
                        right: 4.0,
                        top: 1.0,
                        bottom: 1.0,
                    }),
                    background: Some(theme.surface_2().into()),
                    border_color: Some(theme.accent().into()),
                    border_width: Some(1.0),
                    border_radius: Some(CornerRadius::with_all(Radius::Pixels(3.0))),
                    font_size: Some(appearance.ui_font_subheading()),
                    ..Default::default()
                })
                .build()
                .finish();
            ConstrainedBox::new(input).with_width(180.0).finish()
        } else {
            Text::new_inline(
                node.name.clone(),
                appearance.ui_font_family(),
                appearance.ui_font_subheading(),
            )
            .with_color(theme.main_text_color(theme.background()).into())
            .finish()
        };

        // Use MainAxisSize::Max so the tree node row fills the panel width, eliminating the gap on the right.
        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(ITEM_ICON_TEXT_SPACING)
            .with_child(
                ConstrainedBox::new(Empty::new().finish())
                    .with_width(depth as f32 * FOLDER_DEPTH_INDENT)
                    .finish(),
            )
            .with_child(chevron_el)
            .with_child(icon_el)
            .with_child(label_or_editor)
            .with_main_axis_size(MainAxisSize::Max)
            .finish();

        let state = self.row_states.get(&node.id).cloned().unwrap_or_default();
        let id_for_click = node.id.clone();
        let id_for_double_click = node.id.clone();
        let id_for_right_click = node.id.clone();

        // While renaming, don't accept clicks/right-clicks (let EditorView handle them).
        // Padding must match the normal (hoverable) branch exactly so the row does
        // not shift when rename mode toggles — the normal branch adds no bottom
        // margin, so this one must not either.
        if is_renaming {
            return Container::new(row)
                .with_padding_top(ITEM_PADDING_VERTICAL)
                .with_padding_bottom(ITEM_PADDING_VERTICAL)
                .with_padding_left(ITEM_PADDING_HORIZONTAL)
                .with_padding_right(ITEM_PADDING_HORIZONTAL)
                .finish();
        }

        let hoverable = Hoverable::new(state, move |_| {
            let mut c = Container::new(row)
                .with_padding_top(ITEM_PADDING_VERTICAL)
                .with_padding_bottom(ITEM_PADDING_VERTICAL)
                .with_padding_left(ITEM_PADDING_HORIZONTAL)
                .with_padding_right(ITEM_PADDING_HORIZONTAL)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
            if is_selected {
                c = c.with_background(internal_colors::fg_overlay_3(theme));
            }
            c.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(SshManagerPanelAction::Click(id_for_click.clone()));
        })
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(SshManagerPanelAction::DoubleClick(
                id_for_double_click.clone(),
            ));
        })
        .on_right_click(move |ctx, _, position| {
            let offset = match ctx.element_position_by_id(SSH_PANEL_POSITION_ID) {
                Some(bounds) => position - bounds.origin(),
                None => position,
            };
            ctx.dispatch_typed_action(SshManagerPanelAction::OpenContextMenu {
                target: Some(id_for_right_click.clone()),
                position: offset,
            });
        })
        .finish();

        // Wrap the row into an element that is "both draggable and accepts drops".
        //
        // **Key nesting**: `DropTarget(Container(Draggable(Hoverable)))`.
        // Without the Container layer there is a bug — `Draggable::origin()` returns `child.origin()`
        // (`crates/warpui_core/src/elements/drag/draggable.rs:746-757`), and the
        // child is painted at drag_origin while in the Dragging state, so child.origin() =
        // the ghost position. As a result, when DropTarget wraps Draggable directly, the bounds follow the ghost
        // → the drop target is always under the cursor and can never land on another row. Container.origin/size
        // lock the layout values in their own paint (`container.rs:288 self.origin = ...`),
        // giving the DropTarget stable bounds.
        let drag_state = self
            .row_drag_states
            .get(&node.id)
            .cloned()
            .unwrap_or_default();
        let dragged_id = node.id.clone();
        let draggable = Draggable::new(drag_state, hoverable)
            .with_accepted_by_drop_target_fn(move |drop_data, _app| {
                if drop_data.as_any().downcast_ref::<SshDropData>().is_some() {
                    AcceptedByDropTarget::Yes
                } else {
                    AcceptedByDropTarget::No
                }
            })
            .on_drop(move |ctx, _app, _bounds, data| {
                if let Some(drop) = data.and_then(|d| d.as_any().downcast_ref::<SshDropData>()) {
                    ctx.dispatch_typed_action(SshManagerPanelAction::MoveNode {
                        node_id: dragged_id.clone(),
                        new_parent_id: drop.parent_id.clone(),
                    });
                }
            })
            .finish();

        // The intermediate Container that locks the layout origin — see the comment above.
        let stable_anchor = Container::new(draggable).finish();

        let drop_parent_id = match node.kind {
            NodeKind::Folder => Some(node.id.clone()),
            NodeKind::Server => node.parent_id.clone(),
        };
        DropTarget::new(
            stable_anchor,
            SshDropData {
                parent_id: drop_parent_id,
            },
        )
        .finish()
    }

    fn context_menu_items(&self) -> Vec<(String, SshManagerPanelAction)> {
        match self.context_menu_target.as_ref() {
            None => vec![
                (
                    crate::t!("workspace-left-panel-ssh-manager-menu-new-folder"),
                    SshManagerPanelAction::AddFolder,
                ),
                (
                    crate::t!("workspace-left-panel-ssh-manager-menu-new-server"),
                    SshManagerPanelAction::AddServer,
                ),
            ],
            Some(id) => {
                let kind = self.nodes.iter().find(|n| &n.id == id).map(|n| n.kind);
                match kind {
                    Some(NodeKind::Folder) => vec![
                        (
                            crate::t!("workspace-left-panel-ssh-manager-menu-new-folder"),
                            SshManagerPanelAction::AddFolder,
                        ),
                        (
                            crate::t!("workspace-left-panel-ssh-manager-menu-new-server"),
                            SshManagerPanelAction::AddServer,
                        ),
                        (
                            crate::t!("workspace-left-panel-ssh-manager-menu-rename"),
                            SshManagerPanelAction::StartRename(id.clone()),
                        ),
                        (
                            crate::t!("workspace-left-panel-ssh-manager-menu-delete"),
                            SshManagerPanelAction::DeleteSelected,
                        ),
                    ],
                    Some(NodeKind::Server) => vec![
                        (
                            crate::t!("workspace-left-panel-ssh-manager-menu-edit"),
                            SshManagerPanelAction::Edit,
                        ),
                        (
                            crate::t!("workspace-left-panel-ssh-manager-menu-connect"),
                            SshManagerPanelAction::Connect,
                        ),
                        (
                            crate::t!("workspace-left-panel-ssh-manager-menu-sessions"),
                            SshManagerPanelAction::ToggleSessions(id.clone()),
                        ),
                        (
                            crate::t!("workspace-left-panel-ssh-manager-menu-sftp"),
                            SshManagerPanelAction::OpenSftp,
                        ),
                        (
                            crate::t!("workspace-left-panel-ssh-manager-menu-clone"),
                            SshManagerPanelAction::CloneServer(id.clone()),
                        ),
                        (
                            crate::t!("workspace-left-panel-ssh-manager-menu-delete"),
                            SshManagerPanelAction::DeleteSelected,
                        ),
                    ],
                    None => vec![],
                }
            }
        }
    }

    fn render_context_menu(
        &self,
        appearance: &warp_core::ui::appearance::Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let items = self.context_menu_items();
        let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (i, (label, action)) in items.into_iter().enumerate() {
            let state = self
                .context_menu_item_states
                .get(i)
                .cloned()
                .unwrap_or_default();
            let label_el = Text::new_inline(
                label,
                appearance.ui_font_family(),
                appearance.ui_font_subheading(),
            )
            .with_color(theme.main_text_color(theme.background()).into())
            .finish();
            let row_action = action.clone();
            let item = Hoverable::new(state, move |mouse| {
                let mut c = Container::new(label_el)
                    .with_padding_top(CONTEXT_MENU_ITEM_PADDING_V)
                    .with_padding_bottom(CONTEXT_MENU_ITEM_PADDING_V)
                    .with_padding_left(CONTEXT_MENU_ITEM_PADDING_H)
                    .with_padding_right(CONTEXT_MENU_ITEM_PADDING_H)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.0)));
                if mouse.is_hovered() {
                    c = c.with_background(internal_colors::fg_overlay_3(theme));
                }
                c.finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(row_action.clone());
                ctx.dispatch_typed_action(SshManagerPanelAction::DismissContextMenu);
            })
            .finish();
            col.add_child(item);
        }
        let menu_inner = ConstrainedBox::new(
            Container::new(col.with_main_axis_size(MainAxisSize::Min).finish())
                .with_background(theme.surface_2())
                .with_border(Border::all(1.0).with_border_color(theme.surface_3().into()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.0)))
                .with_uniform_padding(4.0)
                .finish(),
        )
        .with_width(CONTEXT_MENU_WIDTH)
        .finish();

        Dismiss::new(menu_inner)
            .on_dismiss(|ctx, _| {
                ctx.dispatch_typed_action(SshManagerPanelAction::DismissContextMenu);
            })
            .finish()
    }
}

impl Entity for SshManagerPanel {
    type Event = SshManagerPanelEvent;
}

impl TypedActionView for SshManagerPanel {
    type Action = SshManagerPanelAction;

    fn handle_action(&mut self, action: &SshManagerPanelAction, ctx: &mut ViewContext<Self>) {
        match action {
            SshManagerPanelAction::AddRootFolder => self.on_add_folder_with_parent(None, ctx),
            SshManagerPanelAction::AddFolder => {
                let parent = self.parent_for_new_node();
                self.on_add_folder_with_parent(parent, ctx)
            }
            SshManagerPanelAction::ToggleAddMode => self.on_toggle_add_mode(ctx),
            SshManagerPanelAction::AddServer => self.on_add_server(ctx),
            SshManagerPanelAction::DeleteSelected => self.on_delete_selected(ctx),
            SshManagerPanelAction::Connect => self.on_connect(ctx),
            SshManagerPanelAction::Edit => self.on_edit(ctx),
            SshManagerPanelAction::CloneServer(id) => self.on_clone_server(id, ctx),
            SshManagerPanelAction::ToggleSessions(id) => self.on_toggle_sessions(id.clone(), ctx),
            SshManagerPanelAction::AdoptSession {
                node_id,
                pty_session_id,
            } => self.on_adopt_session(node_id.clone(), pty_session_id.clone(), ctx),
            SshManagerPanelAction::Click(id) => self.on_click(id.clone(), ctx),
            SshManagerPanelAction::StartRename(id) => self.enter_rename(id.clone(), false, ctx),
            SshManagerPanelAction::CommitRename => self.commit_rename(ctx),
            SshManagerPanelAction::CancelRename => self.cancel_rename(ctx),
            SshManagerPanelAction::OpenContextMenu { target, position } => {
                self.on_open_context_menu(target.clone(), *position, ctx)
            }
            SshManagerPanelAction::DismissContextMenu => self.on_dismiss_context_menu(ctx),
            SshManagerPanelAction::MoveNode {
                node_id,
                new_parent_id,
            } => self.on_move_node(node_id.clone(), new_parent_id.clone(), ctx),
            SshManagerPanelAction::ToggleNodeCollapsed(id) => {
                self.on_toggle_node_collapsed(id, ctx)
            }
            SshManagerPanelAction::ToggleAllFolders => self.on_toggle_all_folders(ctx),
            SshManagerPanelAction::DoubleClick(id) => self.on_double_click(id.clone(), ctx),
            SshManagerPanelAction::OpenSftp => self.on_open_sftp(ctx),
            SshManagerPanelAction::ImportCandidate { alias } => {
                self.on_import_candidate(alias.clone(), ctx)
            }
            SshManagerPanelAction::RefreshCandidates => {
                self.candidates.update(ctx, |vm, ctx| vm.refresh(ctx));
                self.sync_candidate_row_states(ctx);
                ctx.notify();
            }
            SshManagerPanelAction::ToggleCandidatesSection => {
                self.candidates
                    .update(ctx, |vm, ctx| vm.toggle_expanded(ctx));
                ctx.notify();
            }
        }
    }
}

impl View for SshManagerPanel {
    fn ui_name() -> &'static str {
        "SshManagerPanel"
    }

    fn on_focus(&mut self, _focus_ctx: &FocusContext, _ctx: &mut ViewContext<Self>) {}

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = warp_core::ui::appearance::Appearance::as_ref(app);

        let toolbar = Container::new(self.render_toolbar(appearance))
            .with_uniform_padding(PANEL_HORIZONTAL_PADDING)
            .finish();

        // The saved tree shows **only** what the user deliberately added. The
        // guided "Add a host" block — a blank-server action plus on-demand
        // `~/.ssh/config` suggestions — is shown above the tree only while the
        // user is actively adding (toolbar "+"), so nothing unsolicited ever
        // appears in the list.
        let candidates_section = if self.adding_mode {
            Container::new(self.render_add_block(appearance, app))
                .with_padding_left(PANEL_HORIZONTAL_PADDING - ITEM_PADDING_HORIZONTAL)
                .with_padding_right(PANEL_HORIZONTAL_PADDING - ITEM_PADDING_HORIZONTAL)
                // Separate the "Add a host" block from the saved tree below, so
                // "what I can add" reads as distinct from "what I have".
                .with_padding_bottom(ITEM_ICON_TEXT_SPACING)
                .finish()
        } else {
            Empty::new().finish()
        };

        let tree = Container::new(self.render_tree(appearance))
            .with_padding_left(PANEL_HORIZONTAL_PADDING - ITEM_PADDING_HORIZONTAL)
            .with_padding_right(PANEL_HORIZONTAL_PADDING - ITEM_PADDING_HORIZONTAL)
            .finish();

        // Let the tree fill the remaining vertical space — so the root DropTarget covers down to the panel bottom,
        // and dragging into the blank area below the tree can still land at root (`SshDropData{parent_id:None}`).
        let tree_filled = warpui::elements::Shrinkable::new(1.0, tree).finish();

        let panel_content = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(toolbar)
                .with_child(candidates_section)
                .with_child(tree_filled)
                .finish(),
        )
        .finish();

        let positioned_panel = SavePosition::new(panel_content, SSH_PANEL_POSITION_ID).finish();

        let Some(position) = self.context_menu_position else {
            return positioned_panel;
        };

        let menu_el = self.render_context_menu(appearance);
        let positioning = OffsetPositioning::offset_from_parent(
            position,
            ParentOffsetBounds::ParentByPosition,
            ParentAnchor::TopLeft,
            ChildAnchor::TopLeft,
        );

        let mut stack = Stack::new();
        stack.add_child(positioned_panel);
        stack.add_positioned_overlay_child(menu_el, positioning);
        stack.finish()
    }
}

// --- helpers --------------------------------------------------------------

/// Compute the parent ID for a new node based on the current selection and the node list.
/// - Folder selected → create it as a child under that folder
/// - Server selected → create it as a sibling (inheriting the server's parent)
/// - Nothing selected → create at the root level (return None)
fn resolve_parent_for_new_node(selected_id: Option<&str>, nodes: &[SshNode]) -> Option<String> {
    let id = selected_id?;
    let node = nodes.iter().find(|n| n.id == id)?;
    match node.kind {
        NodeKind::Folder => Some(node.id.clone()),
        NodeKind::Server => node.parent_id.clone(),
    }
}

fn sort_for_display(nodes: Vec<SshNode>, depths: &HashMap<String, usize>) -> Vec<SshNode> {
    use std::collections::{BTreeMap, HashSet};
    let ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let mut by_parent: BTreeMap<Option<String>, Vec<SshNode>> = BTreeMap::new();
    for n in nodes {
        by_parent.entry(n.parent_id.clone()).or_default().push(n);
    }
    for v in by_parent.values_mut() {
        v.sort_by_key(|n| (n.sort_order, n.name.clone()));
    }
    let mut out = Vec::with_capacity(depths.len());
    fn walk(
        parent: Option<&String>,
        by_parent: &BTreeMap<Option<String>, Vec<SshNode>>,
        out: &mut Vec<SshNode>,
        seen: &mut HashSet<String>,
    ) {
        if let Some(children) = by_parent.get(&parent.cloned()) {
            for c in children {
                if !seen.insert(c.id.clone()) {
                    continue;
                }
                out.push(c.clone());
                walk(Some(&c.id), by_parent, out, seen);
            }
        }
    }
    let root_parents: Vec<Option<String>> = by_parent
        .keys()
        .filter(|parent| parent.as_ref().is_none_or(|id| !ids.contains(id)))
        .cloned()
        .collect();
    let mut seen = HashSet::new();
    for parent in root_parents {
        walk(parent.as_ref(), &by_parent, &mut out, &mut seen);
    }
    for children in by_parent.values() {
        for child in children {
            if !seen.insert(child.id.clone()) {
                continue;
            }
            out.push(child.clone());
            walk(Some(&child.id), &by_parent, &mut out, &mut seen);
        }
    }
    out
}

fn compute_depths(nodes: &[SshNode]) -> HashMap<String, usize> {
    let by_id: HashMap<&str, &SshNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut depths = HashMap::with_capacity(nodes.len());
    for n in nodes {
        let mut d = 0;
        let mut p = n.parent_id.as_deref();
        while let Some(pid) = p {
            let Some(parent) = by_id.get(pid) else {
                break;
            };
            d += 1;
            p = parent.parent_id.as_deref();
            if d > 64 {
                break;
            }
        }
        depths.insert(n.id.clone(), d);
    }
    depths
}

/// Pull the `host` field of all ssh_servers rows in one go. On failure, return an empty Vec — when SQLite
/// is temporarily down, the candidates section's "Added" badge renders as "no imported items at all", rather than
/// crashing the whole panel.
fn list_server_hosts() -> Vec<String> {
    use diesel::prelude::*;
    use persistence::schema::ssh_servers;
    warp_ssh_manager::with_conn(|conn| {
        let hosts: Vec<String> = ssh_servers::table.select(ssh_servers::host).load(conn)?;
        Ok(hosts)
    })
    .unwrap_or_else(|e| {
        log::warn!("ssh_manager: failed to list server hosts for candidates: {e:?}");
        Vec::new()
    })
}

fn unique_name(
    conn: &mut diesel::sqlite::SqliteConnection,
    parent: Option<&str>,
    base: &str,
) -> Result<String, anyhow::Error> {
    use diesel::prelude::*;
    use persistence::schema::ssh_nodes;
    let existing: Vec<String> = match parent {
        Some(p) => ssh_nodes::table
            .filter(ssh_nodes::parent_id.eq(p))
            .select(ssh_nodes::name)
            .load(conn)?,
        None => ssh_nodes::table
            .filter(ssh_nodes::parent_id.is_null())
            .select(ssh_nodes::name)
            .load(conn)?,
    };
    let set: std::collections::HashSet<String> = existing.into_iter().collect();
    if !set.contains(base) {
        return Ok(base.to_string());
    }
    for i in 2..1000 {
        let cand = format!("{base} {i}");
        if !set.contains(&cand) {
            return Ok(cand);
        }
    }
    Ok(format!("{base} {}", uuid::Uuid::new_v4()))
}

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
