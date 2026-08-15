//! `CockpitPanel` — the cockpit **sidebar** (left toolbelt tab): a compact, glanceable
//! list of account cards over the `zaplex_cockpit` data spine. Read-only in C2; the
//! live-session quick-list + quick-launch land in later increments (see the cockpit
//! native-integration design doc). The roomy full dashboard is the main-area pane (C2b).

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, Element, Fill as ElementFill, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, Padding, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Rect, SavePosition, ScrollbarWidth, Shrinkable,
    Stack, Text,
};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};
use zaplex_cockpit::{
    fleet_is_large, format_cost, format_relative, heat_fill, heat_pct_label_with_provenance,
    host_auto_collapsed, host_ident, host_session_count, session_glyph, session_key, AccountUsage,
    Favorite, FavoriteKind, FleetTree, HostAvailability, HostNode, SessionSnapshot, SessionState,
    TaskItem, TaskState, TaskStatus, UsageProvenance,
};

use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::style::{
    attention_coloru, ctx_pct_element, glyph_cell, hover_row, icon_verb_button_tooltip,
    provider_color_on, provider_label, session_metric_column_width, status_dot_coloru,
    utilisation_coloru, verb_button_colored, zone_card, BLOCK_RADIUS, CONTROL_RADIUS,
    GLYPH_COL_WIDTH,
};
use crate::ui_components::compact_row_action::CompactRowAction;
use crate::ui_components::icons;
use crate::WorkspaceAction;

const CARD_PADDING: f32 = 8.0;
const CARD_SPACING: f32 = 4.0;
const HEAT_BAR_WIDTH: f32 = 90.0;
const HEAT_BAR_HEIGHT: f32 = 6.0;
const TASK_PEEK_WIDTH: f32 = 390.0;
pub(super) const TASK_PEEK_DELAY: Duration = Duration::from_millis(350);

fn open_registered_host_action(node_id: &str) -> WorkspaceAction {
    WorkspaceAction::OpenSshTerminalByNode {
        node_id: node_id.to_string(),
    }
}

fn toggle_host_favorite_action(node_id: &str, label: &str) -> WorkspaceAction {
    WorkspaceAction::ToggleFavorite {
        kind: FavoriteKind::Host,
        target: node_id.to_string(),
        label: label.to_string(),
    }
}

fn manage_registered_host_action(node_id: &str) -> WorkspaceAction {
    WorkspaceAction::ManageSshHost {
        node_id: node_id.to_string(),
    }
}

fn open_registered_host_agent_action(node_id: &str, host: &str) -> WorkspaceAction {
    WorkspaceAction::OpenSpawnCard {
        registry_node_id: Some(node_id.to_string()),
        host_id: None,
        host: Some(host.to_string()),
        project: None,
    }
}

fn open_registered_host_files_action(node_id: &str) -> WorkspaceAction {
    WorkspaceAction::OpenSftpPaneByNode {
        node_id: node_id.to_string(),
    }
}

fn open_removed_host_repair_action() -> WorkspaceAction {
    WorkspaceAction::OpenSshManager
}

fn available_registry_node_id(host: &HostNode) -> Option<&str> {
    match host.availability {
        HostAvailability::Available => host.registry_node_id.as_deref(),
        HostAvailability::Removed => None,
    }
}

fn host_display_label(host: &HostNode, removed_label: &str) -> String {
    match host.availability {
        HostAvailability::Available => host.host.clone(),
        HostAvailability::Removed => format!("{} — {removed_label}", host.host),
    }
}

fn available_registered_host<'a>(tree: &'a FleetTree, node_id: &str) -> Option<&'a HostNode> {
    tree.hosts
        .iter()
        .find(|host| available_registry_node_id(host) == Some(node_id))
}

fn registered_host_click_target(
    node_id: &str,
    content: Box<dyn Element>,
    state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let action = open_registered_host_action(node_id);
    let position_id = format!("cockpit_host:{node_id}");
    let target = Hoverable::new(state, move |mouse| {
        hover_row(content, mouse.is_hovered(), appearance)
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
    .finish();
    SavePosition::new(target, &position_id).finish()
}

fn removed_host_click_target(
    host_ident: &str,
    content: Box<dyn Element>,
    state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let action = open_removed_host_repair_action();
    let position_id = format!("cockpit_removed_host:{host_ident}");
    let target = Hoverable::new(state, move |mouse| {
        hover_row(content, mouse.is_hovered(), appearance)
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
    .finish();
    SavePosition::new(target, &position_id).finish()
}

/// Events the sidebar emits toward the workspace (via the left panel).
pub enum CockpitPanelEvent {
    /// Open the cockpit pane in the main area: `None` = the fleet dashboard
    /// (every account), `Some(account.key)` = that account's own pane.
    ///
    /// The key travels with the request because the pane IS the account —
    /// opening dedupes on it, so two accounts open two panes rather than one
    /// dashboard that can only look at whichever was clicked last.
    OpenCockpitPane(Option<String>),
}

pub struct CockpitPanel {
    scroll_state: ClippedScrollStateHandle,
    /// Hover/click state per Conductor session row (complete host + provider +
    /// account + conversation identity), synced against the unified inventory.
    /// Clicking a row attaches the agent.
    conductor_row_states: HashMap<String, MouseStateHandle>,
    conductor_peek_states: HashMap<String, MouseStateHandle>,
    /// Hover state per account card (key = account `key`). The whole card is a
    /// click target that opens the roomy dashboard pane.
    card_states: HashMap<String, MouseStateHandle>,
    /// Hover/click state per registered-host spine row, keyed by its registry
    /// `node_id`. Clicking a registered host row (with no live agent) opens a
    /// terminal on that host.
    conductor_host_states: HashMap<String, MouseStateHandle>,
    /// Stable repair-click state for removed daemon roots, keyed by stable
    /// daemon identity rather than their display label.
    removed_host_states: HashMap<String, MouseStateHandle>,
    /// Fixed-width ☆ actions for adding a host favorite.
    conductor_host_favorite_actions: HashMap<String, CompactRowAction>,
    /// Fixed-width ★ actions for removing a host favorite.
    conductor_host_unfavorite_actions: HashMap<String, CompactRowAction>,
    /// Fixed-width icon actions for host management, keyed by registry
    /// `node_id`. Compact rows never spend identity width on repeated labels.
    conductor_host_manage_actions: HashMap<String, CompactRowAction>,
    /// Fixed-width icon actions for opening an agent on each registered host.
    conductor_host_agent_actions: HashMap<String, CompactRowAction>,
    /// Fixed-width icon actions for opening files on each registered host.
    conductor_host_files_actions: HashMap<String, CompactRowAction>,
    /// Hover state of the „VERBINDUNGEN" zone-header gear (opens the SSH manager,
    /// which owns host add/edit — spec v3 §S1/§S2).
    zone_gear_btn: MouseStateHandle,
    /// Hover state of the „KI-KONTEN" header's fleet total — the cross-account
    /// spend figure doubles as the entry point to the fleet pane (spec v3 §S1).
    fleet_total_btn: MouseStateHandle,
    /// Hover/click state for the account-zone "try again" retry (the loading /
    /// scan-failed / empty placeholder). A **stable** handle: `Hoverable` tracks
    /// mouse-down in it, so a fresh one each render would drop the click.
    rescan_btn: MouseStateHandle,
    /// Hover/click state of each **project group header** (the collapsible
    /// Host → Projekt → Session level), keyed by `project_key`. Clicking the
    /// header folds/unfolds that project's sessions.
    conductor_project_states: HashMap<String, MouseStateHandle>,
    /// Which project groups are collapsed, keyed by `project_key`. **Absent =
    /// expanded** (the calm default — you see the sessions); present + `false`
    /// means the user collapsed it. Retained across the 45s reconcile like the
    /// hover maps, so toggling doesn't flicker.
    expanded_projects: HashMap<String, bool>,
}

/// Stable identity of a project group within the tree: the host identity plus
/// the project's **root path** (`ProjectNode::root`, its unique key — NOT the
/// display name, so two same-named repos on one host don't share collapse
/// state), joined by a unit separator so `host` + `a/b` can never collide with
/// `host/a` + `b`. Keys the collapse state + the header hover handle across
/// renders.
fn project_key(host_ident: &str, project_root: &str) -> String {
    format!("{host_ident}\u{1f}{project_root}")
}

/// The branch-first label that identifies a session in the redesigned sidebar
/// (spec §2.2): the session's own registry name if it has one, else its git
/// branch, else its linked-worktree name, else the project + cwd basename. The
/// **model is never** the identity — several parallel Opus agents differ by
/// worktree/branch, not model.
fn session_identity_label(session: &SessionSnapshot, project_name: &str) -> String {
    if !session.name.is_empty() {
        return session.name.clone();
    }
    if let Some(branch) = session.branch.as_deref().filter(|b| !b.is_empty()) {
        return branch.to_string();
    }
    if let Some(worktree) = session.worktree.as_deref().filter(|w| !w.is_empty()) {
        return worktree.to_string();
    }
    let dir = Path::new(&session.cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| session.cwd.clone());
    if !project_name.is_empty() {
        format!("{project_name} — {dir}")
    } else {
        dir
    }
}

#[derive(Debug, PartialEq, Eq)]
struct TaskGlance<'a> {
    completed: usize,
    total: usize,
    current: Option<&'a str>,
    tasks: &'a [TaskItem],
}

fn task_glance(session: &SessionSnapshot) -> Option<TaskGlance<'_>> {
    let state = session.task_state.as_ref()?;
    Some(task_glance_from_state(state))
}

fn task_glance_from_state(state: &TaskState) -> TaskGlance<'_> {
    let completed = state
        .tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Completed)
        .count();
    let current = state
        .tasks
        .iter()
        .find(|task| task.status == TaskStatus::InProgress)
        .or_else(|| {
            state
                .tasks
                .iter()
                .find(|task| task.status == TaskStatus::Pending)
        })
        .map(|task| task.title.as_str());
    TaskGlance {
        completed,
        total: state.tasks.len(),
        current,
        tasks: &state.tasks,
    }
}

pub(super) fn task_activity_label(task_state: Option<&TaskState>, relative: &str) -> String {
    task_state
        .map(task_glance_from_state)
        .and_then(|task| task.current)
        .map_or_else(
            || relative.to_owned(),
            |current| format!("{current} · {relative}"),
        )
}

impl CockpitPanel {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        // Re-render on theme change and whenever the snapshot updates.
        ctx.subscribe_to_model(&Appearance::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&CockpitModel::handle(ctx), |me, _, event, ctx| {
            if matches!(event, CockpitEvent::Updated) {
                me.sync_conductor_states(ctx);
                ctx.notify();
            }
        });
        // Re-render when favorites change so the ★ fill state updates at once.
        ctx.subscribe_to_model(
            &crate::cockpit::favorites::FavoritesStore::handle(ctx),
            |_, _, _, ctx| ctx.notify(),
        );
        let mut me = Self {
            scroll_state: ClippedScrollStateHandle::default(),
            conductor_row_states: HashMap::new(),
            conductor_peek_states: HashMap::new(),
            card_states: HashMap::new(),
            conductor_host_states: HashMap::new(),
            removed_host_states: HashMap::new(),
            conductor_host_favorite_actions: HashMap::new(),
            conductor_host_unfavorite_actions: HashMap::new(),
            conductor_host_manage_actions: HashMap::new(),
            conductor_host_agent_actions: HashMap::new(),
            conductor_host_files_actions: HashMap::new(),
            zone_gear_btn: MouseStateHandle::default(),
            fleet_total_btn: MouseStateHandle::default(),
            rescan_btn: MouseStateHandle::default(),
            conductor_project_states: HashMap::new(),
            expanded_projects: HashMap::new(),
        };
        me.sync_conductor_states(ctx);
        me
    }

    /// Keep one stable row handle per live fleet session (hover needs a stable
    /// handle across renders); drop handles of sessions that disappeared.
    fn sync_conductor_states(&mut self, ctx: &mut ViewContext<Self>) {
        let (routable, visible, host_nodes, removed_hosts, project_keys) = {
            let inv = CockpitModel::as_ref(ctx).inventory();
            let routable: std::collections::HashSet<String> = inv
                .hosts
                .iter()
                .filter(|h| h.is_available())
                .flat_map(|h| {
                    h.projects.iter().flat_map(move |p| {
                        p.sessions
                            .iter()
                            .map(move |s| session_key(h.is_local, h.host_id.as_deref(), s))
                    })
                })
                .collect();
            let visible: std::collections::HashSet<String> = inv
                .hosts
                .iter()
                .flat_map(|h| {
                    h.projects.iter().flat_map(move |p| {
                        p.sessions
                            .iter()
                            .map(move |s| session_key(h.is_local, h.host_id.as_deref(), s))
                    })
                })
                .collect();
            let host_nodes: std::collections::HashSet<String> = inv
                .hosts
                .iter()
                .filter_map(|h| available_registry_node_id(h).map(str::to_string))
                .collect();
            let removed_hosts: std::collections::HashSet<String> = inv
                .hosts
                .iter()
                .filter(|h| h.availability == HostAvailability::Removed)
                .map(|h| host_ident(h.is_local, h.host_id.as_deref()))
                .collect();
            let project_keys: std::collections::HashSet<String> = inv
                .hosts
                .iter()
                .flat_map(|h| {
                    let ident = host_ident(h.is_local, h.host_id.as_deref());
                    h.projects.iter().map(move |p| project_key(&ident, &p.root))
                })
                .collect();
            (routable, visible, host_nodes, removed_hosts, project_keys)
        };
        self.conductor_row_states
            .retain(|k, _| routable.contains(k));
        self.conductor_peek_states
            .retain(|k, _| visible.contains(k));
        for key in routable {
            self.conductor_row_states.entry(key).or_default();
        }
        for key in visible {
            self.conductor_peek_states.entry(key).or_default();
        }
        // Card hover handles, keyed by account `key` (one stable handle per card
        // across renders); drop handles of accounts that disappeared.
        let acct_keys: std::collections::HashSet<String> = CockpitModel::as_ref(ctx)
            .snapshot()
            .accounts
            .iter()
            .map(|a| a.account.key.clone())
            .collect();
        self.card_states.retain(|k, _| acct_keys.contains(k));
        for key in acct_keys {
            self.card_states.entry(key).or_default();
        }
        // Registered-host row handles, keyed by registry `node_id` (one stable
        // handle per clickable host header); drop handles of hosts that vanished.
        self.conductor_host_states
            .retain(|k, _| host_nodes.contains(k));
        self.removed_host_states
            .retain(|k, _| removed_hosts.contains(k));
        self.conductor_host_favorite_actions
            .retain(|k, _| host_nodes.contains(k));
        self.conductor_host_unfavorite_actions
            .retain(|k, _| host_nodes.contains(k));
        self.conductor_host_manage_actions
            .retain(|k, _| host_nodes.contains(k));
        self.conductor_host_agent_actions
            .retain(|k, _| host_nodes.contains(k));
        self.conductor_host_files_actions
            .retain(|k, _| host_nodes.contains(k));
        for node_id in host_nodes {
            self.conductor_host_states
                .entry(node_id.clone())
                .or_default();
            if !self.conductor_host_favorite_actions.contains_key(&node_id) {
                self.conductor_host_favorite_actions.insert(
                    node_id.clone(),
                    CompactRowAction::new(
                        icons::Icon::Star,
                        crate::t!("cockpit-tt-favorite-add"),
                        CockpitPanelAction::ToggleHostFavorite(node_id.clone()),
                        ctx,
                    ),
                );
            }
            if !self
                .conductor_host_unfavorite_actions
                .contains_key(&node_id)
            {
                self.conductor_host_unfavorite_actions.insert(
                    node_id.clone(),
                    CompactRowAction::new(
                        icons::Icon::StarFilled,
                        crate::t!("cockpit-tt-favorite-remove"),
                        CockpitPanelAction::ToggleHostFavorite(node_id.clone()),
                        ctx,
                    ),
                );
            }
            if !self.conductor_host_agent_actions.contains_key(&node_id) {
                self.conductor_host_agent_actions.insert(
                    node_id.clone(),
                    CompactRowAction::new(
                        icons::Icon::AiAssistant,
                        crate::t!("cockpit-host-action-agent"),
                        CockpitPanelAction::OpenHostAgent(node_id.clone()),
                        ctx,
                    ),
                );
            }
            if !self.conductor_host_files_actions.contains_key(&node_id) {
                self.conductor_host_files_actions.insert(
                    node_id.clone(),
                    CompactRowAction::new(
                        icons::Icon::Folder,
                        crate::t!("cockpit-host-action-files"),
                        CockpitPanelAction::OpenHostFiles(node_id.clone()),
                        ctx,
                    ),
                );
            }
            if !self.conductor_host_manage_actions.contains_key(&node_id) {
                self.conductor_host_manage_actions.insert(
                    node_id.clone(),
                    CompactRowAction::new(
                        icons::Icon::DotsHorizontal,
                        crate::t!("cockpit-tt-manage-host"),
                        CockpitPanelAction::ManageHost(node_id),
                        ctx,
                    ),
                );
            }
        }
        for host_ident in removed_hosts {
            self.removed_host_states.entry(host_ident).or_default();
        }
        // Project-group header handles + collapse overrides, keyed by
        // `project_key` (host identity + project name — never the label alone).
        // Drop projects that vanished so the maps don't grow unbounded; the
        // collapse map keeps only live keys, so absent still means "expanded".
        self.conductor_project_states
            .retain(|k, _| project_keys.contains(k));
        self.expanded_projects
            .retain(|k, _| project_keys.contains(k));
        for key in project_keys {
            self.conductor_project_states.entry(key).or_default();
        }
    }

    fn text(
        s: String,
        family: warpui::fonts::FamilyId,
        size: f32,
        color: ColorU,
    ) -> Box<dyn Element> {
        Text::new_inline(s, family, size).with_color(color).finish()
    }

    fn identity_text(
        s: String,
        family: warpui::fonts::FamilyId,
        size: f32,
        color: ColorU,
    ) -> Box<dyn Element> {
        Text::new_inline(s, family, size)
            .with_color(color)
            .with_clip(ClipConfig::ellipsis())
            .finish()
    }

    /// The account-zone placeholder, disambiguated by scan health so an empty
    /// account list no longer reads the same whether the first scan is still
    /// running, a config/dir failed to load, or there genuinely are no accounts.
    /// The failed and genuine-empty cases offer a retry (re-run the scan).
    fn render_scan_placeholder(
        &self,
        health: &zaplex_cockpit::ScanHealth,
        enabled: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        use zaplex_cockpit::ScanHealth;
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let accent = theme.accent().into_solid();
        // A deliberately-disabled cockpit is neither "empty" nor "loading" — say so,
        // and offer no retry (re-scanning cannot help while it is off).
        if !enabled {
            return Self::text(
                crate::t!("cockpit-disabled").to_string(),
                family,
                body,
                muted,
            );
        }
        let (msg, retry) = match health {
            ScanHealth::Pending => (crate::t!("cockpit-loading").to_string(), false),
            ScanHealth::Degraded(_) => (crate::t!("cockpit-scan-failed").to_string(), true),
            ScanHealth::Loaded => (
                crate::t!("workspace-left-panel-cockpit-empty").to_string(),
                true,
            ),
        };
        let msg_el = Self::text(msg, family, body, muted);
        if !retry {
            return msg_el;
        }
        let retry_el = Hoverable::new(self.rescan_btn.clone(), move |mouse| {
            let c = if mouse.is_hovered() { muted } else { accent };
            Text::new_inline(crate::t!("cockpit-retry").to_string(), family, body)
                .with_color(c)
                .finish()
        })
        .with_cursor(warpui::platform::Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(CockpitPanelAction::Rescan))
        .finish();
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.0)
            .with_child(msg_el)
            .with_child(retry_el)
            .finish()
    }

    /// A labelled heat bar: `5h [▓▓▓░░] 62%`, coloured by band. Estimate-driven
    /// bars carry a subtle `~` on the percentage (C3b provenance); real numbers
    /// get no extra chrome.
    fn heat_bar(
        &self,
        label: &str,
        fraction: f64,
        provenance: UsageProvenance,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let size = appearance.ui_font_body();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        // Utilisation is not attention: one shared rule (spec v3 §1.2) — calm grey,
        // true red only at/above the single "fast voll" threshold, contrast-adapted.
        // The bar's WIDTH carries the level; the colour only flags "nearly full".
        let bar_color = utilisation_coloru(fraction, appearance);
        let fill_w = (heat_fill(fraction) as f32) * HEAT_BAR_WIDTH;

        let fill = ConstrainedBox::new(Rect::new().with_background_color(bar_color).finish())
            .with_width(fill_w)
            .with_height(HEAT_BAR_HEIGHT)
            .finish();

        let track = ConstrainedBox::new(
            Container::new(fill)
                .with_background(internal_colors::fg_overlay_1(theme))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.0)))
                .finish(),
        )
        .with_width(HEAT_BAR_WIDTH)
        .with_height(HEAT_BAR_HEIGHT)
        .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(Self::text(label.to_string(), family, size, muted))
            .with_child(track)
            .with_child(Self::text(
                heat_pct_label_with_provenance(fraction, provenance),
                family,
                size,
                bar_color,
            ))
            .with_main_axis_size(MainAxisSize::Min)
            .finish()
    }

    fn render_card(
        &self,
        acct: &AccountUsage,
        is_selected: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let sub = appearance.ui_font_subheading();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        // Header: a provider-colour swatch leads (spec §1 — this is the ONE place
        // provider colour appears: Claude clay / Codex blue), then the account
        // label (alias/email — which account, not which plan).
        let header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(6.0)
            .with_child(
                ConstrainedBox::new(
                    Rect::new()
                        .with_background_color(provider_color_on(
                            acct.account.provider,
                            theme.background().into_solid(),
                        ))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                        .finish(),
                )
                .with_width(12.0)
                .with_height(12.0)
                .finish(),
            )
            .with_child(
                Shrinkable::new(
                    1.0,
                    Self::text(acct.account.label.clone(), family, sub, main),
                )
                .finish(),
            );

        // Provider and Plan are two **separate slots**, not one string (spec v3
        // §S3): the provider is a quiet word, the plan a small badge. Flattening
        // them into "Claude · Max" made the plan read as part of the provider name
        // and gave the two different things one weight. The badge only appears when
        // a plan is actually known — an empty "—" chip is chrome for nothing.
        let mut provider_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(Self::text(
                provider_label(acct.account.provider).to_string(),
                family,
                body,
                muted,
            ));
        if let Some(plan) = acct.account.plan_tier.clone() {
            provider_row = provider_row.with_child(
                Container::new(Self::text(plan, family, body, main))
                    .with_background(internal_colors::fg_overlay_1(theme))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CONTROL_RADIUS)))
                    .with_padding_left(5.0)
                    .with_padding_right(5.0)
                    .finish(),
            );
        }
        let provider_plan = provider_row.with_main_axis_size(MainAxisSize::Min).finish();

        // The card carries ONE live-session line: „N laufende Sessions" (running
        // = active + waiting + monitor; idle is not counted). Waiting is
        // attention, so the count goes amber when any session waits, else muted.
        // The spend/token breakdown moved to the pane (spec §2.2: the card's one
        // metric signal is the 5h meter).
        let waiting = acct
            .sessions
            .iter()
            .filter(|s| s.state == SessionState::Waiting)
            .count();
        let running = acct
            .sessions
            .iter()
            .filter(|s| !matches!(s.state, SessionState::Idle))
            .count();
        let session_line = (running > 0)
            .then(|| crate::t!("cockpit-card-sessions-count", count = (running as i64)));

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(CARD_SPACING)
            .with_child(header.finish())
            .with_child(provider_plan)
            // ONE metric signal on the card: the rolling 5h block (spec §2.2). The
            // week meter, spend and tokens live in the pane, where there is room.
            .with_child(self.heat_bar("5h", acct.heat, acct.provenance, appearance));
        if let Some(session_line) = session_line {
            // Amber here is the attention signal (someone waits on this account),
            // not a heat level — name the intent so it can never drift into the
            // utilisation palette (spec v3 §1.3).
            let color = if waiting > 0 {
                attention_coloru(appearance)
            } else {
                muted
            };
            col = col.with_child(Self::text(session_line, family, body, color));
        }
        // (Reset countdowns intentionally live only in the dashboard pane now —
        // WS4 S5: the sidebar stays a glance surface, the pane carries detail.)

        // A flat account block inside the AI-Accounts zone-card — no per-card
        // container chrome (emphasis via content + spacing, spec §2.1). The whole
        // block selects the account → opens the pane focused on it (WS4 S5).
        // A selected block carries a stable fill; hover adds a subtle fill —
        // colour only, never layout (spec §2.7).
        let col_el = col.finish();
        let handle = self
            .card_states
            .get(&acct.account.key)
            .cloned()
            .unwrap_or_default();
        let key = acct.account.key.clone();
        Hoverable::new(handle, move |mouse| {
            let mut c = Container::new(col_el)
                .with_uniform_padding(CARD_PADDING)
                .with_margin_bottom(CARD_SPACING)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(BLOCK_RADIUS)));
            if is_selected {
                c = c.with_background(internal_colors::fg_overlay_2(theme));
            } else if mouse.is_hovered() {
                c = c.with_background(internal_colors::fg_overlay_1(theme));
            }
            c.finish()
        })
        .with_cursor(warpui::platform::Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CockpitPanelAction::SelectAccount(key.clone()))
        })
        .finish()
    }

    /// The glanceable **Conductor** for the sidebar: the unified cross-host
    /// inventory as a flat Hosts zone, waiting-first, collapsing under scale two
    /// ways — a calm host in a large fleet folds to a one-line [`host_summary`]
    /// (inverse-complexity), and within an open host the calm running/idle rows
    /// fold behind a per-host summary (D1+C2) while waiting rows always show.
    /// Each session is one fixed line whose right metric column never shifts.
    /// Rows are clickable — a session attaches on click via
    /// [`WorkspaceAction::AttachFleetSession`], the same path as the roomy pane
    /// and the `w`-jump. Always `Some`: an empty inventory still renders the
    /// Hosts card (a calm empty-state hint + the "+ Add host" root), so the
    /// surface guides a fresh user instead of vanishing.
    /// The **one** zone header both sidebar zones use (spec v3 §S1), so they can
    /// never drift apart again: a quiet uppercase label, the count as a trailing
    /// muted number, and **at most one** trailing element — the connections
    /// zone's gear, or the accounts zone's fleet total (which doubles as the
    /// fleet-pane entry point). The old Maximize icon is gone; the fleet spend it
    /// used to sit beside is now that single trailing element, not a second one.
    fn render_zone_header(
        label: String,
        count: usize,
        trailing: Option<Box<dyn Element>>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let sub = appearance.ui_font_subheading();
        let faint = theme
            .sub_text_color(theme.background())
            .with_opacity(55)
            .into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            // The label is scaffolding, not content: muted + uppercase, so the eye
            // reads it as structure and skips to the rows (spec v3 §0).
            .with_child(Self::text(label.to_uppercase(), family, sub, muted))
            .with_child(
                Shrinkable::new(1.0, Self::text(count.to_string(), family, sub, faint)).finish(),
            );
        if let Some(trailing) = trailing {
            row = row.with_child(trailing);
        }
        row.with_main_axis_size(MainAxisSize::Max).finish()
    }

    fn render_conductor(
        &self,
        tree: &FleetTree,
        favorites: &[Favorite],
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        // Same faint as the project header's count — the two read as one level.
        let faint = theme
            .sub_text_color(theme.background())
            .with_opacity(55)
            .into_solid();
        let fleet_large = fleet_is_large(tree);

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            // The same calm row rhythm as the roomy pane, scaled down.
            .with_spacing(3.0)
            .with_child(
                Container::new(Self::render_zone_header(
                    crate::t!("cockpit-zone-connections").to_string(),
                    tree.hosts.len(),
                    // The gear is this zone's ONE affordance: it opens the SSH
                    // manager, which owns host add/edit — that is why the spine's
                    // „＋ Host hinzufügen" root is gone (spec v3 §S2).
                    Some(icon_verb_button_tooltip(
                        self.zone_gear_btn.clone(),
                        icons::Icon::Gear,
                        theme.sub_text_color(theme.background()),
                        theme.accent(),
                        crate::t!("cockpit-zone-connections-settings"),
                        appearance,
                        WorkspaceAction::OpenSshManager,
                    )),
                    appearance,
                ))
                .with_margin_bottom(2.0)
                .finish(),
            );

        for host in &tree.hosts {
            // Inverse-complexity: a calm host in a large fleet folds away its
            // children. Only its children — the header row itself stays exactly
            // as interactive as any other host's (spec v3 §2 F8).
            //
            // It used to fold to a bare line of text, which meant the ★, the ⋯,
            // the open-a-terminal click and the status dot all disappeared the
            // moment the fleet grew past two hosts. Interaction died precisely at
            // the scale this tool exists for.
            let collapsed = host_auto_collapsed(host, fleet_large);
            // Locality from the inventory's explicit marker, not a label match.
            let is_local = host.is_local;
            // Stable host identity keys the density-fold state (never the label).
            let ident = host_ident(is_local, host.host_id.as_deref());
            // The row's GLANCE span: worst-child status dot (the host reads
            // amber when any child is waiting, without opening it — spec §3),
            // the name, and — only when folded — how much is behind the fold.
            // The fold count is faint and never amber: a host only auto-folds
            // when nothing on it is waiting (`host_auto_collapsed`), so the
            // fold can only ever hide calm work. NO needs-me badge beside the
            // name: the dot already carries "wartet" (spec v3 §1.3 „Nichts
            // codiert doppelt").
            let mut head = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.0)
                .with_child(Self::host_status_dot(host, appearance))
                .with_child(
                    Shrinkable::new(
                        1.0,
                        Self::identity_text(
                            host_display_label(host, &crate::t!("cockpit-host-removed")),
                            family,
                            body,
                            main,
                        ),
                    )
                    .finish(),
                );
            if collapsed {
                head = head.with_child(Self::text(
                    host_session_count(host).to_string(),
                    family,
                    body,
                    faint,
                ));
            }
            let head_el = head.with_main_axis_size(MainAxisSize::Max).finish();
            // A registered host (no live agent, re-added by the registry merge)
            // is a click target that opens a terminal on it. The hover surface
            // is the WHOLE glance span via the shared row grammar (`hover_row`)
            // — not a text-width sliver around the word (audit P0.2). Live-only
            // hosts get the same geometry, hover-less, so the column aligns.
            let label_el: Box<dyn Element> = match host.availability {
                HostAvailability::Available => match available_registry_node_id(host) {
                    Some(node_id) => {
                        let handle = self
                            .conductor_host_states
                            .get(node_id)
                            .cloned()
                            .unwrap_or_default();
                        registered_host_click_target(node_id, head_el, handle, appearance)
                    }
                    None => hover_row(head_el, false, appearance),
                },
                HostAvailability::Removed => {
                    let handle = self
                        .removed_host_states
                        .get(&ident)
                        .cloned()
                        .unwrap_or_default();
                    removed_host_click_target(&ident, head_el, handle, appearance)
                }
            };
            // Compose the header: identity takes every flexible pixel; repeated
            // secondary actions occupy fixed icon squares. Keeping the actions
            // outside the click target also prevents click collisions.
            // ui-contract: compact-row-actions:start
            let mut header_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.0)
                .with_child(Shrinkable::new(1.0, label_el).finish());
            if let Some(node_id) = available_registry_node_id(host).map(str::to_string) {
                let is_fav = favorites
                    .iter()
                    .any(|f| f.same_target(FavoriteKind::Host, &node_id));
                let favorite_action = if is_fav {
                    self.conductor_host_unfavorite_actions.get(&node_id)
                } else {
                    self.conductor_host_favorite_actions.get(&node_id)
                };
                debug_assert!(favorite_action.is_some());
                if let Some(action) = favorite_action {
                    header_row = header_row.with_child(action.render());
                }
                debug_assert!(self.conductor_host_agent_actions.contains_key(&node_id));
                if let Some(action) = self.conductor_host_agent_actions.get(&node_id) {
                    header_row = header_row.with_child(action.render());
                }
                debug_assert!(self.conductor_host_files_actions.contains_key(&node_id));
                if let Some(action) = self.conductor_host_files_actions.get(&node_id) {
                    header_row = header_row.with_child(action.render());
                }
                debug_assert!(self.conductor_host_manage_actions.contains_key(&node_id));
                if let Some(action) = self.conductor_host_manage_actions.get(&node_id) {
                    header_row = header_row.with_child(action.render());
                }
            }
            // ui-contract: compact-row-actions:end
            col = col.with_child(header_row.with_main_axis_size(MainAxisSize::Max).finish());

            // Sessions grouped by project — the Host → Projekt → Session tree
            // (spec §2.1). Each project is a collapsible group header (its own
            // typographic level, no dot); its sessions are shown waiting-first.
            // Projects default to expanded (absent in `expanded_projects`).
            //
            // A folded host contributes none of this: the fold is what keeps a
            // large fleet calm. Its header above stays whole either way.
            if collapsed {
                continue;
            }
            for project in &host.projects {
                let pkey = project_key(&ident, &project.root);
                let expanded = self.expanded_projects.get(&pkey).copied().unwrap_or(true);
                let has_waiting = project
                    .sessions
                    .iter()
                    .any(|s| s.state == SessionState::Waiting);
                col = col.with_child(
                    Container::new(self.render_project_header(
                        &pkey,
                        &project.name,
                        project.sessions.len(),
                        has_waiting,
                        expanded,
                        appearance,
                    ))
                    // 4 + ROW_H_PADDING: the host rows above sit inside the
                    // shared `hover_row` inset now, so every level below keeps
                    // its old indent relative to them.
                    .with_padding_left(10.0)
                    .finish(),
                );
                if expanded {
                    // Waiting-first within the project (stable sort): the
                    // attention rows lead, the calm ones follow, none hidden.
                    let mut sessions: Vec<&SessionSnapshot> = project.sessions.iter().collect();
                    sessions.sort_by_key(|s| s.state != SessionState::Waiting);
                    for session in sessions {
                        col = col.with_child(
                            Container::new(self.render_conductor_row(
                                &host.host,
                                host.host_id.as_deref(),
                                session,
                                is_local,
                                host.is_available(),
                                appearance,
                            ))
                            .with_padding_left(22.0)
                            .finish(),
                        );
                    }
                }
            }
            if host.projects.is_empty() {
                // A registered host with no live session — shown as a spine root
                // so it stays navigable/launchable, with a calm hint that it is
                // idle (build_fleet_tree drops agentless hosts; the registry merge
                // re-adds them, see CockpitModel).
                col = col.with_child(
                    Container::new(Self::text(
                        crate::t!("cockpit-host-no-agents"),
                        family,
                        body,
                        muted,
                    ))
                    .with_padding_left(22.0)
                    .finish(),
                );
            }
        }
        // Empty state (S6): a fresh sidebar with no live agents and no registered
        // hosts still shows the Hosts card — a calm hint above the always-present
        // "+ Add host" root, so the surface guides the user instead of vanishing.
        if tree.hosts.is_empty() {
            col = col.with_child(
                Container::new(Self::text(
                    crate::t!("cockpit-conductor-empty"),
                    family,
                    body,
                    muted,
                ))
                .with_padding_left(10.0)
                .finish(),
            );
        }
        // NO "＋ Add host" root here (spec v3 §S2): host add/edit belongs to the
        // SSH manager, which the zone-header gear opens. A second entry point in
        // the spine made the zone's last row read as data ("a fourth host") and
        // duplicated a flow the manager already owns.
        Some(col.finish())
    }

    /// One calm Conductor row. Structured task state adds only progress and the
    /// current step below the established session identity; without a plan the
    /// row remains unchanged.
    fn render_conductor_row(
        &self,
        host_label: &str,
        host_id: Option<&str>,
        session: &SessionSnapshot,
        is_local: bool,
        can_attach: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        let task_glance = task_glance(session);
        let label = session_identity_label(session, "");
        let metric_width = session_metric_column_width(&label);

        let glance = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(glyph_cell(
                session_glyph(session.state),
                status_dot_coloru(session.state, appearance),
                appearance,
            ))
            .with_child(Shrinkable::new(1.0, Self::text(label, family, body, main)).finish())
            .with_child(self.metric_column(session, is_local, host_id, metric_width, appearance))
            .with_main_axis_size(MainAxisSize::Max)
            .finish();
        let glance = if let Some(task) = task_glance.as_ref() {
            let progress = task.current.map_or_else(
                || format!("{}/{}", task.completed, task.total),
                |current| format!("{}/{} · {current}", task.completed, task.total),
            );
            Flex::column()
                .with_spacing(2.0)
                .with_child(glance)
                .with_child(
                    Container::new(Self::text(progress, family, body - 1.0, muted))
                        .with_padding_left(GLYPH_COL_WIDTH + 6.0)
                        .finish(),
                )
                .finish()
        } else {
            glance
        };

        // The whole glance line attaches on click — BOTH local and remote (remote
        // in-place adopt is wired via `attach_fleet_session`).
        let key = session_key(is_local, host_id, session);
        let row = if can_attach {
            match self.conductor_row_states.get(&key).cloned() {
                Some(state) => {
                    let action = WorkspaceAction::AttachFleetSession {
                        host: host_label.to_string(),
                        host_id: host_id.map(str::to_string),
                        session_id: session.session_id.clone(),
                        provider: session.provider,
                        config_dir: session.config_dir.clone(),
                        account_email: session.account_email.clone(),
                        is_local,
                    };
                    // Same full-span hover grammar as the host rows (`hover_row`).
                    // The mouse state used to be discarded here — a clickable row
                    // that never says so (audit P0.2).
                    Hoverable::new(state, move |mouse| {
                        hover_row(glance, mouse.is_hovered(), appearance)
                    })
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                    .finish()
                }
                None => hover_row(glance, false, appearance),
            }
        } else {
            hover_row(glance, false, appearance)
        };

        let Some(peek_state) = self.conductor_peek_states.get(&key).cloned() else {
            return row;
        };
        let peek_title = session_identity_label(session, "");
        let peek_account = session.account_email.as_ref().map_or_else(
            || provider_label(session.provider).to_owned(),
            |email| format!("{} · {email}", provider_label(session.provider)),
        );
        let peek_host = host_label.to_owned();
        let peek_cwd = session.cwd.clone();
        let session_state = session.state;
        let task_state = session.task_state.clone();
        let relative = format_relative(session.last_activity, chrono::Utc::now());
        let activity = task_activity_label(task_state.as_ref(), &relative);
        Hoverable::new(peek_state, move |mouse| {
            let mut stack = Stack::new().with_child(row);
            if mouse.is_hovered() {
                stack.add_positioned_overlay_child(
                    Self::render_task_peek(
                        &peek_title,
                        &peek_account,
                        &peek_host,
                        &peek_cwd,
                        session_state,
                        &activity,
                        task_state.as_ref(),
                        appearance,
                    ),
                    OffsetPositioning::offset_from_parent(
                        vec2f(8.0, 0.0),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopRight,
                        ChildAnchor::TopLeft,
                    ),
                );
            }
            stack.finish()
        })
        .with_hover_in_delay(TASK_PEEK_DELAY)
        .finish()
    }

    pub(super) fn render_task_peek(
        title: &str,
        account: &str,
        host: &str,
        cwd: &str,
        state: SessionState,
        activity: &str,
        task_state: Option<&TaskState>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let accent = theme.accent().into_solid();
        let state_label = match state {
            SessionState::Waiting => crate::t!("cockpit-task-peek-state-waiting"),
            SessionState::Active | SessionState::Monitor => {
                crate::t!("cockpit-task-peek-state-working")
            }
            SessionState::Idle => crate::t!("cockpit-task-peek-state-idle"),
        };
        let header = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(
                Shrinkable::new(1.0, Self::text(title.to_owned(), family, body, main)).finish(),
            )
            .with_child(Self::text(state_label, family, body, accent))
            .finish();
        let facts = Flex::column()
            .with_spacing(3.0)
            .with_child(Self::text(
                crate::t!("cockpit-task-peek-account", value = account),
                family,
                body - 1.0,
                muted,
            ))
            .with_child(Self::text(
                crate::t!("cockpit-task-peek-host", value = host),
                family,
                body - 1.0,
                muted,
            ))
            .with_child(Self::text(
                crate::t!("cockpit-task-peek-directory", value = cwd),
                family,
                body - 1.0,
                muted,
            ))
            .finish();
        let mut content = Flex::column()
            .with_spacing(7.0)
            .with_child(header)
            .with_child(facts)
            .with_child(Self::text(
                crate::t!("cockpit-task-peek-activity", value = activity),
                family,
                body,
                main,
            ));
        if let Some(task_state) = task_state {
            let completed = task_state
                .tasks
                .iter()
                .filter(|task| task.status == TaskStatus::Completed)
                .count();
            content = content.with_child(Self::text(
                crate::t!(
                    "cockpit-task-peek-title",
                    completed = completed,
                    total = task_state.tasks.len()
                ),
                family,
                body,
                main,
            ));
            for task in &task_state.tasks {
                let (glyph, color) = match task.status {
                    TaskStatus::Pending => ("○", muted),
                    TaskStatus::InProgress => ("●", accent),
                    TaskStatus::Completed => ("✓", muted),
                };
                content = content.with_child(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_spacing(8.0)
                        .with_child(Self::text(glyph.to_owned(), family, body, color))
                        .with_child(
                            Shrinkable::new(
                                1.0,
                                Self::text(task.title.clone(), family, body, color),
                            )
                            .finish(),
                        )
                        .finish(),
                );
            }
        } else {
            content = content.with_child(Self::text(
                crate::t!("cockpit-task-peek-no-plan"),
                family,
                body,
                muted,
            ));
        }
        ConstrainedBox::new(
            Container::new(content.finish())
                .with_background(theme.tooltip_background())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.0)))
                .with_padding(Padding::uniform(10.0))
                .finish(),
        )
        .with_width(TASK_PEEK_WIDTH)
        .finish()
    }

    /// The fixed-width **right metric column** of a session line:
    /// `[provider-icon] Model·effort ctx%`, right-aligned inside a constant-width
    /// box so the metrics never move horizontally as the branch label changes
    /// (spec §2.3 — the horizontal-jump defect the owner vetoed). The provider
    /// icon leads (spec §2.3), muted; model rests in accent; ctx% is heat-colored.
    fn metric_column(
        &self,
        session: &SessionSnapshot,
        is_local: bool,
        host_id: Option<&str>,
        width: f32,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let effort = crate::cockpit::session_effort(session, is_local, host_id);
        let attrs = zaplex_cockpit::session_attrs(
            &session.model,
            effort.as_deref(),
            session.ctx_tokens,
            session.state,
        );
        // Glance surface: ONLY the context-fill readout, right-aligned in a fixed
        // column. No provider mark in the tree — provider (account) colour lives
        // in the KI-Konten cards + the pane table, never the spine (spec §1:
        // provider colours never appear in the tree). Model·effort is pane detail.
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_spacing(4.0);
        if let Some(pct) = attrs.ctx_pct {
            row = row.with_child(ctx_pct_element(pct, attrs.ctx_fill, false, appearance));
        }
        ConstrainedBox::new(row.with_main_axis_size(MainAxisSize::Max).finish())
            .with_width(width)
            .finish()
    }

    /// The leading worst-child status dot for a host header: waiting if any child
    /// waits (the whole host reads amber), else working if any child works, else
    /// idle — so attention bubbles up without opening the host (spec §3).
    fn host_status_dot(host: &HostNode, appearance: &Appearance) -> Box<dyn Element> {
        let state = if host.needs_me > 0 {
            SessionState::Waiting
        } else if host
            .projects
            .iter()
            .flat_map(|p| &p.sessions)
            .any(|s| matches!(s.state, SessionState::Active | SessionState::Monitor))
        {
            SessionState::Active
        } else {
            SessionState::Idle
        };
        glyph_cell(
            session_glyph(state),
            status_dot_coloru(state, appearance),
            appearance,
        )
    }

    /// A **project group header** — the collapsible middle level of the tree
    /// (Host → Projekt → Session, spec §2.1). It reads as a group label, never a
    /// session: a disclosure chevron (`▾` open / `▸` collapsed) + the project name
    /// (muted, brightening on hover) + the session count, with **no status dot**
    /// (the user vetoed a dot before the project). Attention still reaches the
    /// eye: a *collapsed* project that hides a waiting session tints its **count**
    /// amber — the chevron stays a pure affordance, so nothing is encoded twice
    /// (spec v3 §1.3). Clicking anywhere folds/unfolds.
    fn render_project_header(
        &self,
        pkey: &str,
        name: &str,
        count: usize,
        has_waiting: bool,
        expanded: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let theme = appearance.theme();
        let bg = theme.background();
        let main_c = theme.main_text_color(bg).into_solid();
        let muted_c = theme.sub_text_color(bg).into_solid();
        let faint_c = theme.sub_text_color(bg).with_opacity(55).into_solid();
        // The chevron carries ONLY the collapse state — never attention (spec v3
        // §1.3: nothing is encoded twice, and the chevron is an affordance, not a
        // signal). It stays muted in every case. A real SVG icon, not a text
        // glyph: as an affordance it must be pixel-identical everywhere and can't
        // depend on the UI font happening to carry ▾/▸ (spec v3 §7 / E3).
        let chevron_icon = if expanded {
            icons::Icon::ChevronDown
        } else {
            icons::Icon::ChevronRight
        };
        let chevron_fill = theme.sub_text_color(bg);
        // Attention for a hidden waiting session rides the COUNT instead: when a
        // project is collapsed and hides someone who waits, its session count goes
        // amber. One signal, at the place that does the hiding — expanded projects
        // never need it, because the waiting session's own dot is then visible.
        let count_c = if !expanded && has_waiting {
            attention_coloru(appearance)
        } else {
            faint_c
        };
        let handle = self
            .conductor_project_states
            .get(pkey)
            .cloned()
            .unwrap_or_default();
        let pkey_owned = pkey.to_string();
        let name_s = name.to_string();
        let count_s = count.to_string();
        Hoverable::new(handle, move |mouse| {
            let name_color = if mouse.is_hovered() { main_c } else { muted_c };
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.0)
                .with_child(
                    ConstrainedBox::new(chevron_icon.to_warpui_icon(chevron_fill).finish())
                        .with_width(GLYPH_COL_WIDTH)
                        .with_height(GLYPH_COL_WIDTH)
                        .finish(),
                )
                .with_child(
                    Shrinkable::new(
                        1.0,
                        Text::new_inline(name_s.clone(), family, body)
                            .with_color(name_color)
                            .finish(),
                    )
                    .finish(),
                )
                .with_child(
                    Text::new_inline(count_s.clone(), family, body)
                        .with_color(count_c)
                        .finish(),
                )
                .with_main_axis_size(MainAxisSize::Max)
                .finish()
        })
        .with_cursor(warpui::platform::Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CockpitPanelAction::ToggleProject(pkey_owned.clone()))
        })
        .finish()
    }

    /// The „KI-KONTEN" zone header: label + count like the connections zone, plus
    /// the **fleet total** — the one cross-account number (spec v3 §S1).
    ///
    /// The Maximize icon is gone, but the *fleet view it opened* is not: an
    /// account pane can only ever show its own account, so if this number and its
    /// entry point both vanished, cross-account spend would have no home at all —
    /// a regression, not a decluttering. The total therefore stays visible and
    /// **is itself the affordance**: clicking it opens the fleet pane. One
    /// element, two jobs, no extra chrome.
    fn render_header(
        &self,
        snapshot_len: usize,
        fleet_today: f64,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let total = verb_button_colored(
            self.fleet_total_btn.clone(),
            crate::t!(
                "cockpit-header-today-total",
                today = format_cost(fleet_today)
            ),
            theme.sub_text_color(theme.background()).into_solid(),
            theme.accent().into_solid(),
            appearance,
            CockpitPanelAction::OpenDashboardPane,
        );
        Self::render_zone_header(
            crate::t!("cockpit-zone-accounts").to_string(),
            snapshot_len,
            Some(total),
            appearance,
        )
    }

    // (No standalone Maximize icon any more — spec v3 §S1. The fleet dashboard it
    // opened is still reachable: the KI-KONTEN header's fleet total is now the
    // affordance, dispatching `OpenDashboardPane` → the fleet pane (no account
    // key). A card click opens that account's own pane instead — an *additional*
    // path, never a replacement for the fleet-wide view, which is why the total
    // stays a target of its own.)
}

impl View for CockpitPanel {
    fn ui_name() -> &'static str {
        "CockpitPanel"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        // A disabled cockpit clears its snapshot to empty; the placeholder must say
        // "disabled", not "no accounts" (spec: the empty state is only for the real one).
        let enabled = *crate::cockpit::settings::CockpitSettings::as_ref(app).enabled;

        let snapshot = CockpitModel::as_ref(app).snapshot().clone();
        let inventory = CockpitModel::as_ref(app).inventory().clone();
        // Favorites drive the ★ fill state on host + session rows (design §10).
        let favorites = crate::cockpit::favorites::FavoritesStore::handle(app)
            .as_ref(app)
            .items()
            .to_vec();

        let mut cards = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);

        // ── Hosts zone-card (leads, spec §2.1) — the object tree, rendered
        // whenever the inventory has hosts, crucially even with **no** AI
        // account so registered SSH hosts still appear as roots (#100). One flat
        // `surface_1` card, no heavy chrome.
        if let Some(conductor) = self.render_conductor(&inventory, &favorites, appearance) {
            cards = cards.with_child(
                zone_card(conductor, appearance)
                    .with_uniform_padding(CARD_PADDING)
                    .with_margin_bottom(CARD_SPACING * 2.0)
                    .finish(),
            );
        }

        // A degraded scan with some accounts present: the list may be missing others
        // (e.g. a broken Codex sign-in). Warn above the accounts; the empty case shows
        // this in its own placeholder instead.
        if !snapshot.accounts.is_empty()
            && matches!(snapshot.health, zaplex_cockpit::ScanHealth::Degraded(_))
        {
            cards = cards.with_child(
                Container::new(self.render_scan_placeholder(&snapshot.health, enabled, appearance))
                    .with_uniform_padding(CARD_PADDING)
                    .with_margin_bottom(CARD_SPACING * 2.0)
                    .finish(),
            );
        }

        // ── AI-Accounts zone-card (below the hosts, spec §2.1). One flat card
        // holding the fleet-usage header + one flat block per account. Empty
        // accounts show a calm hint instead (a section under the hosts, not the
        // whole panel — hosts stay visible without an account).
        if snapshot.accounts.is_empty() {
            // Even with no accounts the zone keeps its header („KI-KONTEN 0"), so
            // the section reads as a deliberate empty state, not a missing panel
            // (spec §S1). No fleet total here — there is no fleet to open into.
            let empty = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(
                    Container::new(Self::render_zone_header(
                        crate::t!("cockpit-zone-accounts").to_string(),
                        0,
                        None,
                        appearance,
                    ))
                    .with_margin_bottom(CARD_SPACING * 2.0)
                    .finish(),
                )
                .with_child(self.render_scan_placeholder(&snapshot.health, enabled, appearance));
            cards = cards.with_child(
                Container::new(empty.finish())
                    .with_uniform_padding(CARD_PADDING)
                    .finish(),
            );
        } else {
            let fleet_today: f64 = snapshot.accounts.iter().map(|a| a.today.cost_usd).sum();
            let selected = CockpitModel::as_ref(app)
                .selected_account()
                .map(str::to_string);
            let mut accounts = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(
                    Container::new(self.render_header(
                        snapshot.accounts.len(),
                        fleet_today,
                        appearance,
                    ))
                    .with_margin_bottom(CARD_SPACING * 2.0)
                    .finish(),
                );
            for acct in &snapshot.accounts {
                let is_selected = selected.as_deref() == Some(acct.account.key.as_str());
                accounts = accounts.with_child(self.render_card(acct, is_selected, appearance));
            }
            cards = cards.with_child(
                zone_card(accounts.finish(), appearance)
                    .with_uniform_padding(CARD_PADDING)
                    .finish(),
            );
        }

        let body_el = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            cards.finish(),
            ScrollbarWidth::Auto,
            theme.disabled_text_color(theme.background()).into(),
            theme.main_text_color(theme.background()).into(),
            ElementFill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        Container::new(body_el)
            .with_uniform_padding(CARD_PADDING)
            .finish()
    }
}

impl Entity for CockpitPanel {
    type Event = CockpitPanelEvent;
}

/// Sidebar actions (routed back into the view by the action system).
#[derive(Clone, Debug)]
pub enum CockpitPanelAction {
    OpenDashboardPane,
    ToggleHostFavorite(String),
    OpenHostAgent(String),
    OpenHostFiles(String),
    ManageHost(String),
    /// Collapse/expand a project group in the Host → Projekt → Session tree,
    /// keyed by `project_key`. Toggles between absent/`true` (expanded, the
    /// default) and `false` (collapsed).
    ToggleProject(String),
    /// Select an account (its `account.key`) → open (or focus) that account's
    /// own pane and carry a stable highlight in the sidebar. A second click
    /// focuses the pane; it does not de-select.
    SelectAccount(String),
    /// Re-run the account scan — the retry on the loading/scan-failed/empty
    /// placeholder.
    Rescan,
}

impl TypedActionView for CockpitPanel {
    type Action = CockpitPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CockpitPanelAction::OpenDashboardPane => {
                ctx.emit(CockpitPanelEvent::OpenCockpitPane(None));
            }
            CockpitPanelAction::ToggleHostFavorite(node_id) => {
                let host =
                    available_registered_host(CockpitModel::as_ref(ctx).inventory(), node_id)
                        .map(|host| host.host.clone());
                if let Some(host) = host {
                    ctx.dispatch_typed_action(&toggle_host_favorite_action(node_id, &host));
                } else {
                    log::warn!("favorite action ignored for missing host node {node_id}");
                }
            }
            CockpitPanelAction::OpenHostAgent(node_id) => {
                let host =
                    available_registered_host(CockpitModel::as_ref(ctx).inventory(), node_id)
                        .map(|host| host.host.clone());
                if let Some(host) = host {
                    ctx.dispatch_typed_action(&open_registered_host_agent_action(node_id, &host));
                } else {
                    log::warn!("agent action ignored for missing host node {node_id}");
                }
            }
            CockpitPanelAction::OpenHostFiles(node_id) => {
                if available_registered_host(CockpitModel::as_ref(ctx).inventory(), node_id)
                    .is_some()
                {
                    ctx.dispatch_typed_action(&open_registered_host_files_action(node_id));
                } else {
                    log::warn!("files action ignored for unavailable host node {node_id}");
                }
            }
            CockpitPanelAction::ManageHost(node_id) => {
                if available_registered_host(CockpitModel::as_ref(ctx).inventory(), node_id)
                    .is_some()
                {
                    ctx.dispatch_typed_action(&manage_registered_host_action(node_id));
                } else {
                    log::warn!("manage action ignored for unavailable host node {node_id}");
                }
            }
            CockpitPanelAction::ToggleProject(key) => {
                // Absent = expanded (default); the first toggle collapses to false.
                let cur = self.expanded_projects.get(key).copied().unwrap_or(true);
                self.expanded_projects.insert(key.clone(), !cur);
                ctx.notify();
            }
            CockpitPanelAction::SelectAccount(key) => {
                // Mark it selected (the sidebar highlight follows), then open —
                // or focus — that account's own pane. Clicking the same card
                // again lands here too and simply focuses the pane it already
                // has: the selection no longer toggles off underneath it.
                let key = key.clone();
                CockpitModel::handle(ctx)
                    .update(ctx, |model, ctx| model.select_account(key.clone(), ctx));
                ctx.emit(CockpitPanelEvent::OpenCockpitPane(Some(key)));
            }
            CockpitPanelAction::Rescan => {
                CockpitModel::handle(ctx).update(ctx, |model, ctx| model.rescan(ctx));
            }
        }
    }
}

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
