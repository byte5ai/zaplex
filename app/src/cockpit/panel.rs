//! `CockpitPanel` — the cockpit **sidebar** (left toolbelt tab): a compact, glanceable
//! list of account cards over the `zaplex_cockpit` data spine. Read-only in C2; the
//! live-session quick-list + quick-launch land in later increments (see the cockpit
//! native-integration design doc). The roomy full dashboard is the main-area pane (C2b).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Fill as ElementFill, Flex, Hoverable, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, ParentElement, Radius, Rect, ScrollbarWidth, Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};
use zaplex_cockpit::{
    fleet_is_large, format_cost, format_tokens, heat_fill, heat_pct_label_with_provenance,
    host_auto_collapsed, host_ident, host_key, host_summary, session_glyph, AccountUsage, Favorite,
    FavoriteKind, FleetTree, HeatLevel, HostNode, SessionSnapshot, SessionState, UsageProvenance,
};

use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::style::{
    ctx_pct_element, glyph_cell, heat_coloru, icon_verb_button, provider_icon, provider_label,
    status_dot_coloru, zone_card, GLYPH_COL_WIDTH, METRIC_COL_WIDTH,
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
    expand_btn: MouseStateHandle,
    /// Hover/click state per Conductor session row (key = `host_ident\0id`,
    /// stable host identity — never the display label), synced against the
    /// unified inventory. Clicking a row attaches the agent.
    conductor_row_states: HashMap<String, MouseStateHandle>,
    /// Hover state of each local row's compact "review" verb (step 6, key =
    /// `host_ident\0id`). The sidebar is the glance surface, so it carries only the
    /// review entry point; the full commit/PR cluster lives on the main pane.
    conductor_review_states: HashMap<String, MouseStateHandle>,
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
    /// Hover state for the spine's "＋ Add host" root.
    add_host_btn: MouseStateHandle,
    /// Hover state of each host's "N running · M idle — show" density toggle
    /// (D1+C2, key = stable host identity `host_ident`). Waiting rows are always
    /// shown; the calm running/idle rows fold behind this summary until clicked.
    conductor_host_rest_states: HashMap<String, MouseStateHandle>,
    /// Which hosts have their non-waiting (running/idle) rows expanded, keyed by
    /// stable host identity `host_ident`. Absent / `false` = folded behind the
    /// summary (the calm default); `true` = the user expanded them. Retained
    /// across the 45s reconcile like the hover maps, so expanding doesn't flicker.
    expanded_host_rest: HashMap<String, bool>,
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
            expand_btn: MouseStateHandle::default(),
            conductor_row_states: HashMap::new(),
            conductor_review_states: HashMap::new(),
            card_states: HashMap::new(),
            conductor_host_states: HashMap::new(),
            conductor_host_star_states: HashMap::new(),
            conductor_host_manage_states: HashMap::new(),
            conductor_row_star_states: HashMap::new(),
            add_host_btn: MouseStateHandle::default(),
            conductor_host_rest_states: HashMap::new(),
            expanded_host_rest: HashMap::new(),
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
        self.conductor_review_states.retain(|k, _| live.contains(k));
        self.conductor_row_star_states.retain(|k, _| live.contains(k));
        for key in live {
            self.conductor_row_states.entry(key.clone()).or_default();
            self.conductor_review_states.entry(key.clone()).or_default();
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
        // Per-host density-toggle handles + expand overrides, keyed by the
        // stable host identity (never the label — two remote daemons can share
        // one). Drop hosts that vanished so the maps don't grow unbounded.
        let host_idents: std::collections::HashSet<String> = inv
            .hosts
            .iter()
            .map(|h| host_ident(h.is_local, h.host_id.as_deref()))
            .collect();
        self.conductor_host_rest_states
            .retain(|k, _| host_idents.contains(k));
        self.expanded_host_rest
            .retain(|k, _| host_idents.contains(k));
        for ident in host_idents {
            self.conductor_host_rest_states.entry(ident).or_default();
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
        let level = HeatLevel::from_fraction(fraction);
        let fill_w = (heat_fill(fraction) as f32) * HEAT_BAR_WIDTH;

        let fill = ConstrainedBox::new(
            Rect::new()
                .with_background_color(heat_coloru(level))
                .finish(),
        )
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
                heat_coloru(level),
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

        // Header: the provider icon leads (spec §2.5), then the account label
        // (email/org — which account, not which plan).
        let header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(6.0)
            .with_child(
                ConstrainedBox::new(
                    provider_icon(acct.account.provider)
                        .to_warpui_icon(theme.sub_text_color(theme.background()))
                        .finish(),
                )
                .with_width(GLYPH_COL_WIDTH)
                .with_height(GLYPH_COL_WIDTH)
                .finish(),
            )
            .with_child(
                Shrinkable::new(
                    1.0,
                    Self::text(acct.account.label.clone(), family, sub, main),
                )
                .finish(),
            );

        // Provider · Plan as two fixed slots (spec §2.4) — a dash fills the plan
        // slot when the plan is unknown, so the slot is always present and the
        // plan never silently reads as the provider. The Codex `plan_tier` leak
        // (provider name bleeding into the plan) was fixed in the data model (S2).
        let plan = acct
            .account
            .plan_tier
            .clone()
            .unwrap_or_else(|| crate::t!("cockpit-card-plan-none"));
        let provider_plan = crate::t!(
            "cockpit-card-provider-plan",
            provider = provider_label(acct.account.provider),
            plan = plan
        );

        let cost_line = crate::t!(
            "cockpit-card-cost-line",
            cost = format_cost(acct.today.cost_usd),
            tokens = format_tokens(acct.today.total)
        );

        // Live-session status (C3a): waiting sessions are THE signal — they
        // need the user. Rendered in the heat palette when non-zero.
        let waiting = acct
            .sessions
            .iter()
            .filter(|s| s.state == SessionState::Waiting)
            .count();
        let active = acct
            .sessions
            .iter()
            .filter(|s| s.state == SessionState::Active)
            .count();
        let session_line = (!acct.sessions.is_empty()).then(|| {
            let mut parts = Vec::new();
            if active > 0 {
                parts.push(crate::t!("cockpit-card-sessions-active", count = (active as i64)));
            }
            if waiting > 0 {
                parts.push(crate::t!("cockpit-card-sessions-waiting", count = (waiting as i64)));
            }
            let monitor = acct.sessions.len() - active - waiting;
            if monitor > 0 {
                parts.push(crate::t!("cockpit-card-sessions-running", count = (monitor as i64)));
            }
            parts.join(" · ")
        });

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(CARD_SPACING)
            .with_child(header.finish())
            .with_child(Self::text(provider_plan, family, body, muted))
            // BOTH meters, not just the binding window (spec §2.5): the rolling
            // 5h block and the 7-day week, each heat-colored, so the user sees
            // both limits at a glance instead of one merged headline.
            .with_child(self.heat_bar("5h", acct.heat, acct.provenance, appearance))
            .with_child(self.heat_bar("wk", acct.heat_week, acct.provenance, appearance))
            .with_child(Self::text(cost_line, family, body, muted));
        if let Some(session_line) = session_line {
            let color = if waiting > 0 {
                heat_coloru(HeatLevel::Critical)
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
    /// and the `w`-jump. `None` when the inventory has no hosts.
    fn render_conductor(
        &self,
        tree: &FleetTree,
        favorites: &[Favorite],
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        if tree.hosts.is_empty() {
            return None;
        }
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let sub = appearance.ui_font_subheading();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let fleet_large = fleet_is_large(tree);

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            // The same calm row rhythm as the roomy pane, scaled down.
            .with_spacing(3.0)
            .with_child(
                Container::new(Self::text(
                    crate::t!("cockpit-conductor-title").to_string(),
                    family,
                    sub,
                    main,
                ))
                .with_margin_bottom(2.0)
                .finish(),
            );

        for host in &tree.hosts {
            // Inverse-complexity: a calm host in a large fleet folds to one line.
            if host_auto_collapsed(host, fleet_large) {
                col = col.with_child(Self::text(host_summary(host), family, body, muted));
                continue;
            }
            // Locality from the inventory's explicit marker, not a label match.
            let is_local = host.is_local;
            // Stable host identity keys the density-fold state (never the label).
            let ident = host_ident(is_local, host.host_id.as_deref());
            // Label + needs-me badge = the terminal click target (registered
            // hosts); the ★ favorite toggle sits *beside* it, not inside, so the
            // two clicks never collide.
            let mut label_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.0)
                .with_child(
                    Shrinkable::new(1.0, Self::text(host.host.clone(), family, body, main))
                        .finish(),
                );
            if host.needs_me > 0 {
                label_row = label_row.with_child(Self::text(
                    crate::t!("cockpit-conductor-needs-me-badge", count = (host.needs_me as i64)),
                    family,
                    body,
                    heat_coloru(HeatLevel::Critical),
                ));
            }
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
                    header_row = header_row.with_child(icon_verb_button(
                        manage_state,
                        icons::Icon::DotsHorizontal,
                        theme.sub_text_color(theme.background()),
                        theme.accent(),
                        action,
                    ));
                }
            }
            col = col.with_child(header_row.with_main_axis_size(MainAxisSize::Max).finish());

            // Sessions (D1+C2 density): waiting rows are ALWAYS shown — they are
            // the attention signal — while the calm running/idle rows fold behind
            // a one-line summary until the user opens it. Sessions are already
            // waiting-first within each needs-me-first project, so the waiting
            // rows lead naturally.
            let mut waiting: Vec<(&str, &SessionSnapshot)> = Vec::new();
            let mut rest: Vec<(&str, &SessionSnapshot)> = Vec::new();
            for project in &host.projects {
                for session in &project.sessions {
                    if session.state == SessionState::Waiting {
                        waiting.push((project.name.as_str(), session));
                    } else {
                        rest.push((project.name.as_str(), session));
                    }
                }
            }
            for (pname, session) in &waiting {
                col = col.with_child(
                    Container::new(self.render_conductor_row(
                        &host.host,
                        host.host_id.as_deref(),
                        pname,
                        session,
                        is_local,
                        favorites,
                        appearance,
                    ))
                    .with_padding_left(10.0)
                    .finish(),
                );
            }
            if !rest.is_empty() {
                let expanded = self.expanded_host_rest.get(&ident).copied().unwrap_or(false);
                if expanded {
                    for (pname, session) in &rest {
                        col = col.with_child(
                            Container::new(self.render_conductor_row(
                                &host.host,
                                host.host_id.as_deref(),
                                pname,
                                session,
                                is_local,
                                favorites,
                                appearance,
                            ))
                            .with_padding_left(10.0)
                            .finish(),
                        );
                    }
                }
                let running = rest
                    .iter()
                    .filter(|(_, s)| matches!(s.state, SessionState::Active | SessionState::Monitor))
                    .count();
                let idle = rest.len() - running;
                col = col.with_child(
                    Container::new(self.render_rest_toggle(&ident, running, idle, expanded, appearance))
                        .with_padding_left(10.0)
                        .finish(),
                );
            } else if host.projects.is_empty() {
                // A registered host with no live agent — shown as a spine root so
                // it stays navigable/launchable, with a calm hint that it is idle
                // (build_fleet_tree drops agentless hosts; the registry merge
                // re-adds them, see CockpitModel).
                col = col.with_child(
                    Container::new(Self::text(
                        crate::t!("cockpit-host-no-agents"),
                        family,
                        body,
                        muted,
                    ))
                    .with_padding_left(10.0)
                    .finish(),
                );
            }
        }
        // "Add host" root — folds the SSH-manager add function onto the spine
        // (design §10). A Plus icon + label (#107), muted at rest / accent on
        // hover; creates a blank registered host and opens its editor.
        let add_label = crate::t!("cockpit-conductor-add-host").to_string();
        let add_rest = theme.sub_text_color(theme.background());
        let add_accent = theme.accent();
        let add_host = Hoverable::new(self.add_host_btn.clone(), move |mouse| {
            let color = if mouse.is_hovered() { add_accent } else { add_rest };
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.0)
                .with_child(
                    ConstrainedBox::new(icons::Icon::Plus.to_warpui_icon(color).finish())
                        .with_width(GLYPH_COL_WIDTH)
                        .with_height(GLYPH_COL_WIDTH)
                        .finish(),
                )
                .with_child(
                    Text::new_inline(add_label.clone(), family, body)
                        .with_color(color.into_solid())
                        .finish(),
                )
                .finish()
        })
        .with_cursor(warpui::platform::Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(WorkspaceAction::AddSshHost))
        .finish();
        col = col.with_child(add_host);
        Some(col.finish())
    }

    /// One fixed Conductor line: `Dot · Branch · [Provider Model ctx%]` with the
    /// metric cluster in a **fixed-width right column** so it never shifts as the
    /// branch label grows (spec §2.3). Status is the leading colour dot only.
    /// The whole line attaches on click (local + remote); the ★ favourite and
    /// local review verb trail after the metric column, so they never push the
    /// metrics around. Hover recolors, never re-lays-out (spec §2.7).
    fn render_conductor_row(
        &self,
        host_label: &str,
        host_id: Option<&str>,
        project_name: &str,
        session: &SessionSnapshot,
        is_local: bool,
        favorites: &[Favorite],
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();

        // Identity = branch / worktree, never the model (spec §2.2).
        let label = session_identity_label(session, project_name);
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

        // Trailing curation/review affordances (always present, colour-only
        // hover): the local review verb (Eye) + the ★ favourite (design §10).
        // The favourite target is the host-scoped `host_key` — session ids are
        // unique only within a host, so a bare id could favourite the wrong host.
        let review = is_local
            .then(|| {
                self.conductor_review_states.get(&key).cloned().map(|st| {
                    let action = WorkspaceAction::ReviewSession {
                        project_root: PathBuf::from(&session.project_root),
                        project_name: session.project_name.clone(),
                    };
                    icon_verb_button(
                        st,
                        icons::Icon::Eye,
                        theme.sub_text_color(theme.background()),
                        theme.accent(),
                        action,
                    )
                })
            })
            .flatten();
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

        if review.is_none() && star.is_none() {
            return glance_el;
        }
        let mut trailing = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.0)
            .with_child(Shrinkable::new(1.0, glance_el).finish());
        if let Some(review) = review {
            trailing = trailing.with_child(review);
        }
        if let Some(star) = star {
            trailing = trailing.with_child(star);
        }
        trailing.with_main_axis_size(MainAxisSize::Max).finish()
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
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let accent = theme.accent().into_solid();
        let muted = theme.sub_text_color(theme.background());

        let effort = crate::cockpit::session_effort(session, is_local, host_id);
        let attrs = zaplex_cockpit::session_attrs(
            &session.model,
            effort.as_deref(),
            session.ctx_tokens,
            session.state,
        );
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_spacing(5.0)
            .with_child(
                ConstrainedBox::new(
                    provider_icon(session.provider)
                        .to_warpui_icon(muted)
                        .finish(),
                )
                .with_width(GLYPH_COL_WIDTH)
                .with_height(GLYPH_COL_WIDTH)
                .finish(),
            );
        if !attrs.model_effort.is_empty() {
            row = row.with_child(Self::text(attrs.model_effort, family, body, accent));
        }
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

    /// The per-host density toggle (D1+C2): a calm one-line summary of the folded
    /// running/idle rows that expands/collapses them on click. Muted at rest,
    /// accent on hover (colour only — the list doesn't jump).
    fn render_rest_toggle(
        &self,
        ident: &str,
        running: usize,
        idle: usize,
        expanded: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let theme = appearance.theme();
        let rest = theme.sub_text_color(theme.background());
        let accent = theme.accent();
        // Pick the message by which parts are non-zero so the summary never
        // reads a hollow "· 0 idle" (Idle sessions aren't surfaced yet, so idle
        // is usually 0). `rest` is never empty here, so at least one part is > 0.
        let r = running as i64;
        let i = idle as i64;
        let label = match (running > 0, idle > 0, expanded) {
            (true, true, false) => {
                crate::t!("cockpit-conductor-rest-show-both", running = r, idle = i)
            }
            (true, false, false) => crate::t!("cockpit-conductor-rest-show-running", running = r),
            (false, _, false) => crate::t!("cockpit-conductor-rest-show-idle", idle = i),
            (true, true, true) => {
                crate::t!("cockpit-conductor-rest-hide-both", running = r, idle = i)
            }
            (true, false, true) => crate::t!("cockpit-conductor-rest-hide-running", running = r),
            (false, _, true) => crate::t!("cockpit-conductor-rest-hide-idle", idle = i),
        };
        let handle = self
            .conductor_host_rest_states
            .get(ident)
            .cloned()
            .unwrap_or_default();
        let ident_owned = ident.to_string();
        Hoverable::new(handle, move |mouse| {
            let color = if mouse.is_hovered() { accent } else { rest };
            Text::new_inline(label.clone(), family, body)
                .with_color(color.into_solid())
                .finish()
        })
        .with_cursor(warpui::platform::Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CockpitPanelAction::ToggleHostRest(ident_owned.clone()))
        })
        .finish()
    }

    /// The ★ favorite toggle for a Conductor tree node (design §10), rendered in
    /// the real icon font (#107). Favorited rests in the accent (hover → muted,
    /// hinting un-star); not-favorited rests muted (hover → accent, hinting star).
    fn star_button(
        state: MouseStateHandle,
        is_fav: bool,
        appearance: &Appearance,
        action: WorkspaceAction,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let accent = theme.accent();
        let muted = theme.sub_text_color(theme.background());
        let (rest, hover) = if is_fav {
            (accent, muted)
        } else {
            (muted, accent)
        };
        icon_verb_button(state, icons::Icon::Stars, rest, hover, action)
    }

    fn render_header(
        &self,
        snapshot_len: usize,
        fleet_today: f64,
        fleet_week: f64,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let sub = appearance.ui_font_subheading();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(Self::text(
                crate::t!("cockpit-header-account-count", count = (snapshot_len as i64)),
                family,
                sub,
                main,
            ))
            // Fleet total = today · week (spec §2.5), the sum across accounts.
            .with_child(Self::text(
                crate::t!(
                    "cockpit-header-cost-summary",
                    today = format_cost(fleet_today),
                    week = format_cost(fleet_week)
                ),
                family,
                body,
                muted,
            ))
            .with_child(self.render_expand_button(appearance))
            .finish()
    }

    /// "Open dashboard" affordance: expands the compact sidebar into the roomy
    /// main-area cockpit pane (C2b).
    fn render_expand_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let sub = appearance.ui_font_subheading();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        // Labeled + larger than the old bare "⤢" glyph, which nobody recognized as
        // the entry point to the full dashboard.
        Hoverable::new(self.expand_btn.clone(), move |mouse| {
            let mut c = Container::new(Self::text(crate::t!("cockpit-header-dashboard-button"), family, sub, muted))
                .with_padding_left(8.0)
                .with_padding_right(8.0)
                .with_padding_top(3.0)
                .with_padding_bottom(3.0)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
            if mouse.is_hovered() {
                c = c.with_background(internal_colors::fg_overlay_2(theme));
            }
            c.finish()
        })
        .with_cursor(warpui::platform::Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(CockpitPanelAction::OpenDashboardPane);
        })
        .finish()
    }
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
            cards = cards.with_child(
                Container::new(Self::text(
                    crate::t!("workspace-left-panel-cockpit-empty"),
                    family,
                    body,
                    muted,
                ))
                .with_uniform_padding(CARD_PADDING)
                .finish(),
            );
        } else {
            let fleet_today: f64 = snapshot.accounts.iter().map(|a| a.today.cost_usd).sum();
            let fleet_week: f64 = snapshot.accounts.iter().map(|a| a.week.cost_usd).sum();
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
                        fleet_week,
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
    /// Fold/unfold a host's calm (running/idle) rows in the density view
    /// (D1+C2), keyed by stable host identity `host_ident`. Waiting rows are
    /// unaffected — they always stay visible.
    ToggleHostRest(String),
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
            CockpitPanelAction::ToggleHostRest(ident) => {
                let cur = self.expanded_host_rest.get(ident).copied().unwrap_or(false);
                self.expanded_host_rest.insert(ident.clone(), !cur);
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
