//! `CockpitPanel` — the cockpit **sidebar** (left toolbelt tab): a compact, glanceable
//! list of account cards over the `zaplex_cockpit` data spine. Read-only in C2; the
//! live-session quick-list + quick-launch land in later increments (see the cockpit
//! native-integration design doc). The roomy full dashboard is the main-area pane (C2b).

use std::collections::HashMap;
use std::path::Path;

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Fill as ElementFill, Flex, Hoverable, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, ParentElement, Radius, Rect, ScrollbarWidth, Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};
use zaplex_cockpit::{
    fleet_is_large, format_cost, heat_fill, heat_pct_label_with_provenance,
    host_auto_collapsed, host_ident, host_key, host_summary, session_glyph, AccountUsage, Favorite,
    FavoriteKind, FleetTree, HostNode, SessionSnapshot, SessionState, UsageProvenance,
};

use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::style::{
    attention_coloru, ctx_pct_element, glyph_cell, icon_verb_button_tooltip, provider_color_on,
    provider_label, status_dot_coloru, utilisation_coloru, verb_button_colored, zone_card,
    GLYPH_COL_WIDTH, METRIC_COL_WIDTH,
};
use crate::ui_components::icons;
use crate::WorkspaceAction;

const CARD_PADDING: f32 = 8.0;
const CARD_SPACING: f32 = 4.0;
const HEAT_BAR_WIDTH: f32 = 90.0;
const HEAT_BAR_HEIGHT: f32 = 6.0;

/// Events the sidebar emits toward the workspace (via the left panel).
pub enum CockpitPanelEvent {
    /// Open the full cockpit dashboard pane in the main area.
    OpenDashboardPane,
}

pub struct CockpitPanel {
    scroll_state: ClippedScrollStateHandle,
    /// Hover/click state per Conductor session row (key = `host_ident\0id`,
    /// stable host identity — never the display label), synced against the
    /// unified inventory. Clicking a row attaches the agent.
    conductor_row_states: HashMap<String, MouseStateHandle>,
    /// Hover state per account card (key = account `key`). The whole card is a
    /// click target that opens the roomy dashboard pane.
    card_states: HashMap<String, MouseStateHandle>,
    /// Hover/click state per registered-host spine row, keyed by its registry
    /// `node_id`. Clicking a registered host row (with no live agent) opens a
    /// terminal on that host.
    conductor_host_states: HashMap<String, MouseStateHandle>,
    /// Hover/click state per host ★ (favorite toggle), keyed by registry
    /// `node_id`. The ★ curates a host favorite (design §10).
    conductor_host_star_states: HashMap<String, MouseStateHandle>,
    /// Hover/click state per host "⋯ manage" affordance, keyed by registry
    /// `node_id`. Opens the SSH-manager editor for the host (design §10 folds
    /// the SSH-manager add/edit function onto the host nodes).
    conductor_host_manage_states: HashMap<String, MouseStateHandle>,
    /// Hover/click state per session ★ (favorite toggle), keyed by `host_key`.
    conductor_row_star_states: HashMap<String, MouseStateHandle>,
    /// Hover state of the „VERBINDUNGEN" zone-header gear (opens the SSH manager,
    /// which owns host add/edit — spec v3 §S1/§S2).
    zone_gear_btn: MouseStateHandle,
    /// Hover state of the „KI-KONTEN" header's fleet total — the cross-account
    /// spend figure doubles as the entry point to the fleet pane (spec v3 §S1).
    fleet_total_btn: MouseStateHandle,
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
            card_states: HashMap::new(),
            conductor_host_states: HashMap::new(),
            conductor_host_star_states: HashMap::new(),
            conductor_host_manage_states: HashMap::new(),
            conductor_row_star_states: HashMap::new(),
            zone_gear_btn: MouseStateHandle::default(),
            fleet_total_btn: MouseStateHandle::default(),
            conductor_project_states: HashMap::new(),
            expanded_projects: HashMap::new(),
        };
        me.sync_conductor_states(ctx);
        me
    }

    /// Keep one stable row handle per live fleet session (hover needs a stable
    /// handle across renders); drop handles of sessions that disappeared.
    fn sync_conductor_states(&mut self, ctx: &mut ViewContext<Self>) {
        let inv = CockpitModel::as_ref(ctx).inventory();
        let live: std::collections::HashSet<String> = inv
            .hosts
            .iter()
            .flat_map(|h| {
                h.projects.iter().flat_map(move |p| {
                    p.sessions
                        .iter()
                        .map(move |s| host_key(h.is_local, h.host_id.as_deref(), &s.session_id))
                })
            })
            .collect();
        self.conductor_row_states.retain(|k, _| live.contains(k));
        self.conductor_row_star_states.retain(|k, _| live.contains(k));
        for key in live {
            self.conductor_row_states.entry(key.clone()).or_default();
            self.conductor_row_star_states.entry(key).or_default();
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
        let host_nodes: std::collections::HashSet<String> = inv
            .hosts
            .iter()
            .filter_map(|h| h.registry_node_id.clone())
            .collect();
        self.conductor_host_states
            .retain(|k, _| host_nodes.contains(k));
        self.conductor_host_star_states
            .retain(|k, _| host_nodes.contains(k));
        self.conductor_host_manage_states
            .retain(|k, _| host_nodes.contains(k));
        for key in host_nodes {
            self.conductor_host_states.entry(key.clone()).or_default();
            self.conductor_host_star_states.entry(key.clone()).or_default();
            self.conductor_host_manage_states.entry(key).or_default();
        }
        // Project-group header handles + collapse overrides, keyed by
        // `project_key` (host identity + project name — never the label alone).
        // Drop projects that vanished so the maps don't grow unbounded; the
        // collapse map keeps only live keys, so absent still means "expanded".
        let project_keys: std::collections::HashSet<String> = inv
            .hosts
            .iter()
            .flat_map(|h| {
                let ident = host_ident(h.is_local, h.host_id.as_deref());
                h.projects
                    .iter()
                    .map(move |p| project_key(&ident, &p.root))
            })
            .collect();
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
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
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
                attention_coloru()
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
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.0)));
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
        let faint = theme.sub_text_color(theme.background()).with_opacity(55).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            // The label is scaffolding, not content: muted + uppercase, so the eye
            // reads it as structure and skips to the rows (spec v3 §0).
            .with_child(Self::text(label.to_uppercase(), family, sub, muted))
            .with_child(Shrinkable::new(
                1.0,
                Self::text(count.to_string(), family, sub, faint),
            )
            .finish());
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
            // Inverse-complexity: a calm host in a large fleet folds to one line.
            if host_auto_collapsed(host, fleet_large) {
                // L2 (supaterm): the loudest colour survives every collapse level
                // — a folded host that needs you still reads amber, so attention
                // is never hidden by the fold.
                let color = if host.needs_me > 0 {
                    status_dot_coloru(SessionState::Waiting, appearance)
                } else {
                    muted
                };
                col = col.with_child(Self::text(host_summary(host), family, body, color));
                continue;
            }
            // Locality from the inventory's explicit marker, not a label match.
            let is_local = host.is_local;
            // Stable host identity keys the density-fold state (never the label).
            let ident = host_ident(is_local, host.host_id.as_deref());
            // Label + needs-me badge = the terminal click target (registered
            // hosts); the ★ favorite toggle sits *beside* it, not inside, so the
            // two clicks never collide.
            let label_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.0)
                .with_child(
                    Shrinkable::new(1.0, Self::text(host.host.clone(), family, body, main))
                        .finish(),
                );
            // NO needs-me badge here: the host's leading status dot already carries
            // "wartet" (worst-child, amber). A second amber count beside the name
            // would encode the same fact twice — spec v3 §1.3 „Nichts codiert
            // doppelt". The attention trail is dot → dot, nothing else.
            let label_el = label_row.with_main_axis_size(MainAxisSize::Max).finish();
            // A registered host (no live agent, re-added by the registry merge)
            // becomes a click target that opens a terminal on it — the spine's
            // host-row action. Live-only hosts keep their plain label.
            let label_el: Box<dyn Element> = match host.registry_node_id.clone() {
                Some(node_id) => {
                    let handle = self
                        .conductor_host_states
                        .get(&node_id)
                        .cloned()
                        .unwrap_or_default();
                    Hoverable::new(handle, move |_mouse| label_el)
                        .with_cursor(warpui::platform::Cursor::PointingHand)
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(WorkspaceAction::OpenSshTerminalByNode {
                                node_id: node_id.clone(),
                            })
                        })
                        .finish()
                }
                None => label_el,
            };
            // Compose the header: a leading worst-child status dot (the host
            // reads amber when any child is waiting, without opening it — spec §3
            // worst-child inheritance), then the clickable label (flex), then a
            // ★ favorite toggle for registered hosts (points at the node_id).
            let mut header_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.0)
                .with_child(Self::host_status_dot(host, appearance))
                .with_child(Shrinkable::new(1.0, label_el).finish());
            if let Some(node_id) = host.registry_node_id.clone() {
                if let Some(star_state) = self.conductor_host_star_states.get(&node_id).cloned() {
                    let is_fav = favorites
                        .iter()
                        .any(|f| f.same_target(FavoriteKind::Host, &node_id));
                    let action = WorkspaceAction::ToggleFavorite {
                        kind: FavoriteKind::Host,
                        target: node_id.clone(),
                        label: host.host.clone(),
                    };
                    header_row = header_row
                        .with_child(Self::star_button(star_state, is_fav, appearance, action));
                }
                // ⋯ manage: open the SSH-manager editor for this host (design §10
                // folds host add/edit onto the spine's host nodes).
                if let Some(manage_state) =
                    self.conductor_host_manage_states.get(&node_id).cloned()
                {
                    let action = WorkspaceAction::ManageSshHost {
                        node_id: node_id.clone(),
                    };
                    header_row = header_row.with_child(icon_verb_button_tooltip(
                        manage_state,
                        icons::Icon::DotsHorizontal,
                        theme.sub_text_color(theme.background()),
                        theme.accent(),
                        crate::t!("cockpit-tt-manage-host"),
                        appearance,
                        action,
                    ));
                }
            }
            col = col.with_child(header_row.with_main_axis_size(MainAxisSize::Max).finish());

            // Sessions grouped by project — the Host → Projekt → Session tree
            // (spec §2.1). Each project is a collapsible group header (its own
            // typographic level, no dot); its sessions are shown waiting-first.
            // Projects default to expanded (absent in `expanded_projects`).
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
                    .with_padding_left(4.0)
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
                                favorites,
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

    /// One fixed Conductor line: `Dot · Branch · ctx%` with the metric cluster in
    /// a **fixed-width right column** so it never shifts as the branch label grows
    /// (spec §2.3). Status is the leading shape-coded dot only; the branch is the
    /// sole identity (the project is carried by the group header above it, so the
    /// row never repeats it). The whole line attaches on click (local + remote);
    /// the ★ favourite trails after the metric column so it never pushes the
    /// metrics around. Hover recolors, never re-lays-out (spec §2.7).
    fn render_conductor_row(
        &self,
        host_label: &str,
        host_id: Option<&str>,
        session: &SessionSnapshot,
        is_local: bool,
        favorites: &[Favorite],
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();

        // Identity = branch / worktree only, never the model and never the
        // project (the group header carries the project) — spec §2.2.
        let label = session_identity_label(session, "");
        let fav_label = label.clone();

        // The glance line: colour dot · branch (flex) · fixed metric column.
        let glance = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(glyph_cell(
                session_glyph(session.state),
                status_dot_coloru(session.state, appearance),
                appearance,
            ))
            .with_child(Shrinkable::new(1.0, Self::text(label, family, body, main)).finish())
            .with_child(self.metric_column(session, is_local, host_id, appearance))
            .with_main_axis_size(MainAxisSize::Max)
            .finish();

        // The whole glance line attaches on click — BOTH local and remote (remote
        // in-place adopt is wired via `attach_fleet_session`).
        let key = host_key(is_local, host_id, &session.session_id);
        let glance_el = match self.conductor_row_states.get(&key).cloned() {
            Some(state) => {
                let action = WorkspaceAction::AttachFleetSession {
                    host: host_label.to_string(),
                    host_id: host_id.map(str::to_string),
                    session_id: session.session_id.clone(),
                    is_local,
                };
                Hoverable::new(state, move |_mouse| glance)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                    .finish()
            }
            None => glance,
        };

        // The one trailing affordance: the ★ favourite (design §10). Review + the
        // model levers are a pane concern — keeping them off the glance rows is
        // what makes the list read as a calm column. The favourite target is the
        // host-scoped `host_key` (session ids are unique only within a host).
        let star = self.conductor_row_star_states.get(&key).cloned().map(|st| {
            let is_fav = favorites
                .iter()
                .any(|f| f.same_target(FavoriteKind::Session, &key));
            let action = WorkspaceAction::ToggleFavorite {
                kind: FavoriteKind::Session,
                target: key.clone(),
                label: fav_label,
            };
            Self::star_button(st, is_fav, appearance, action)
        });

        let Some(star) = star else {
            return glance_el;
        };
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.0)
            .with_child(Shrinkable::new(1.0, glance_el).finish())
            .with_child(star)
            .with_main_axis_size(MainAxisSize::Max)
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
            .with_width(METRIC_COL_WIDTH)
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
            attention_coloru()
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

    /// The favorite toggle for a Conductor tree node (design §10) — the
    /// conventional star: a **filled gold star** when favorited (hover dims to
    /// hint un-star), a **hollow outline star** otherwise (hover → gold to hint
    /// add). A tooltip names the action, since a bare star is otherwise ambiguous.
    fn star_button(
        state: MouseStateHandle,
        is_fav: bool,
        appearance: &Appearance,
        action: WorkspaceAction,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let gold = Fill::Solid(theme.ui_yellow_color());
        let muted = theme.sub_text_color(theme.background());
        // A not-yet-favourited star recedes (very faint) so the rows stay calm;
        // it brightens to gold on hover, hinting the add. A favourited star is
        // always the full gold.
        let faint = theme.sub_text_color(theme.background()).with_opacity(38);
        let (icon, rest, hover, tooltip) = if is_fav {
            (
                icons::Icon::StarFilled,
                gold,
                muted,
                crate::t!("cockpit-tt-favorite-remove"),
            )
        } else {
            (
                icons::Icon::Star,
                faint,
                gold,
                crate::t!("cockpit-tt-favorite-add"),
            )
        };
        icon_verb_button_tooltip(state, icon, rest, hover, tooltip, appearance, action)
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
            crate::t!("cockpit-header-today-total", today = format_cost(fleet_today)),
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
    // affordance, dispatching the same `OpenDashboardPane`. Per-account panes are
    // an *additional* path (P1, not yet built), never a replacement for the
    // fleet-wide view.)
}

impl View for CockpitPanel {
    fn ui_name() -> &'static str {
        "CockpitPanel"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let muted = theme.sub_text_color(theme.background()).into_solid();

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
                .with_child(Self::text(
                    crate::t!("workspace-left-panel-cockpit-empty"),
                    family,
                    body,
                    muted,
                ));
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
    /// Collapse/expand a project group in the Host → Projekt → Session tree,
    /// keyed by `project_key`. Toggles between absent/`true` (expanded, the
    /// default) and `false` (collapsed).
    ToggleProject(String),
    /// Select an account (its `account.key`) → open the dashboard pane focused
    /// on it and carry a stable highlight in the sidebar (WS4 S5). A second
    /// click on the selected account de-selects it.
    SelectAccount(String),
}

impl TypedActionView for CockpitPanel {
    type Action = CockpitPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CockpitPanelAction::OpenDashboardPane => {
                ctx.emit(CockpitPanelEvent::OpenDashboardPane);
            }
            CockpitPanelAction::ToggleProject(key) => {
                // Absent = expanded (default); the first toggle collapses to false.
                let cur = self.expanded_projects.get(key).copied().unwrap_or(true);
                self.expanded_projects.insert(key.clone(), !cur);
                ctx.notify();
            }
            CockpitPanelAction::SelectAccount(key) => {
                // Store the selection on the shared model (so the pane reacts),
                // then open the dashboard pane. The model emits Updated, which
                // re-renders both the sidebar highlight and the pane focus.
                let key = key.clone();
                CockpitModel::handle(ctx).update(ctx, |model, ctx| model.select_account(key, ctx));
                ctx.emit(CockpitPanelEvent::OpenDashboardPane);
            }
        }
    }
}
