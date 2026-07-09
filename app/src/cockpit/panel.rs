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
    fleet_is_large, format_cost, format_reset, format_tokens, heat_fill,
    heat_pct_label_with_provenance, host_auto_collapsed, host_key, host_summary, session_glyph,
    AccountUsage, FleetTree, HeatLevel, SessionSnapshot, SessionState, UsageProvenance,
};

use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::style::{ctx_pct_element, glyph_cell, heat_coloru, verb_button, VerbKind};
use crate::WorkspaceAction;

/// Max session rows shown per host in the compact sidebar before an overflow
/// line — the sidebar is glanceable, the roomy pane is the full view.
const SIDEBAR_MAX_ROWS_PER_HOST: usize = 6;

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
    /// Hover state of each local row's compact "◈ review" verb (step 6, key =
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
        let mut me = Self {
            scroll_state: ClippedScrollStateHandle::default(),
            expand_btn: MouseStateHandle::default(),
            conductor_row_states: HashMap::new(),
            conductor_review_states: HashMap::new(),
            card_states: HashMap::new(),
            conductor_host_states: HashMap::new(),
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
        for key in live {
            self.conductor_row_states.entry(key.clone()).or_default();
            self.conductor_review_states.entry(key).or_default();
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
        for key in host_nodes {
            self.conductor_host_states.entry(key).or_default();
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

    fn render_card(&self, acct: &AccountUsage, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let sub = appearance.ui_font_subheading();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let accent = theme.accent().into_solid();
        let now = chrono::Utc::now();

        // Header: label (bold-ish subheading) + optional plan badge.
        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(6.0)
            .with_child(
                Shrinkable::new(
                    1.0,
                    Self::text(acct.account.label.clone(), family, sub, main),
                )
                .finish(),
            );
        if let Some(plan) = &acct.account.plan_tier {
            header = header.with_child(
                Container::new(Self::text(plan.clone(), family, body, accent))
                    .with_padding_left(6.0)
                    .with_padding_right(6.0)
                    .with_padding_top(1.0)
                    .with_padding_bottom(1.0)
                    .with_background(internal_colors::fg_overlay_1(theme))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                    .finish(),
            );
        }

        let cost_line = format!(
            "today {} · {}",
            format_cost(acct.today.cost_usd),
            format_tokens(acct.today.total)
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
                parts.push(format!("● {active} active"));
            }
            if waiting > 0 {
                parts.push(format!("● {waiting} waiting"));
            }
            let monitor = acct.sessions.len() - active - waiting;
            if monitor > 0 {
                parts.push(format!("◌ {monitor} running"));
            }
            parts.join(" · ")
        });

        let reset_5h = format_reset(acct.reset5h, now);
        let reset_wk = format_reset(acct.reset_week, now);
        let reset_line = match (reset_5h.is_empty(), reset_wk.is_empty()) {
            (true, true) => None,
            (false, true) => Some(format!("5h ↻ {reset_5h}")),
            (true, false) => Some(format!("wk ↻ {reset_wk}")),
            (false, false) => Some(format!("5h ↻ {reset_5h} · wk ↻ {reset_wk}")),
        };

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(CARD_SPACING)
            .with_child(header.finish())
            // Headline = the *binding* window (fullest of 5h / week / Opus /
            // Sonnet sublimits), not always 5h — otherwise a busy weekly/Opus
            // limit reads as a calm 5h and the card under-reports (Codex #6).
            .with_child({
                let (frac, label) = zaplex_cockpit::binding_window(acct);
                self.heat_bar(label, frac, acct.provenance, appearance)
            })
            .with_child(Self::text(cost_line, family, body, muted));
        if let Some(session_line) = session_line {
            let color = if waiting > 0 {
                heat_coloru(HeatLevel::Critical)
            } else {
                muted
            };
            col = col.with_child(Self::text(session_line, family, body, color));
        }
        if let Some(reset_line) = reset_line {
            col = col.with_child(Self::text(reset_line, family, body, muted));
        }

        let card = Container::new(col.finish())
            .with_uniform_padding(CARD_PADDING)
            .with_margin_bottom(CARD_SPACING)
            .with_background(internal_colors::fg_overlay_1(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.0)))
            .finish();
        // The whole card is a click target → opens the roomy dashboard pane (same
        // action as the ⤢ expand button). Previously the card looked interactive but
        // did nothing on click.
        let handle = self
            .card_states
            .get(&acct.account.key)
            .cloned()
            .unwrap_or_default();
        Hoverable::new(handle, move |_mouse| card)
            .with_cursor(warpui::platform::Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(CockpitPanelAction::OpenDashboardPane))
            .finish()
    }

    /// The glanceable **Conductor** for the sidebar: the unified cross-host
    /// inventory in condensed form, waiting-first, collapsing under scale (calm
    /// hosts fold to a one-line [`host_summary`] when the fleet is large). Rows
    /// are clickable — a local session attaches on click via
    /// [`WorkspaceAction::AttachFleetSession`], the same path as the roomy pane
    /// and the `w`-jump. `None` when the fleet has no live sessions.
    fn render_conductor(
        &self,
        tree: &FleetTree,
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
            let mut host_header = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.0)
                .with_child(
                    Shrinkable::new(1.0, Self::text(host.host.clone(), family, body, main))
                        .finish(),
                );
            if host.needs_me > 0 {
                host_header = host_header.with_child(Self::text(
                    format!("● {}", host.needs_me),
                    family,
                    body,
                    heat_coloru(HeatLevel::Critical),
                ));
            }
            let header_el = host_header.with_main_axis_size(MainAxisSize::Max).finish();
            // A registered host (no live agent, re-added by the registry merge)
            // becomes a click target that opens a terminal on it — the spine's
            // host-row action. Live-only hosts keep their plain header.
            let header_el: Box<dyn Element> = match host.registry_node_id.clone() {
                Some(node_id) => {
                    let handle = self
                        .conductor_host_states
                        .get(&node_id)
                        .cloned()
                        .unwrap_or_default();
                    Hoverable::new(handle, move |_mouse| header_el)
                        .with_cursor(warpui::platform::Cursor::PointingHand)
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(WorkspaceAction::OpenSshTerminalByNode {
                                node_id: node_id.clone(),
                            })
                        })
                        .finish()
                }
                None => header_el,
            };
            col = col.with_child(header_el);

            // Sessions across the host's projects, already waiting-first per
            // project (projects are needs-me-first). Capped for glanceability.
            let mut shown = 0usize;
            let total: usize = host.projects.iter().map(|p| p.sessions.len()).sum();
            'host: for project in &host.projects {
                for session in &project.sessions {
                    if shown >= SIDEBAR_MAX_ROWS_PER_HOST {
                        break 'host;
                    }
                    shown += 1;
                    col = col.with_child(
                        Container::new(self.render_conductor_row(
                            &host.host,
                            host.host_id.as_deref(),
                            &project.name,
                            session,
                            is_local,
                            appearance,
                        ))
                        .with_padding_left(10.0)
                        .finish(),
                    );
                }
            }
            if total > shown {
                col = col.with_child(
                    Container::new(Self::text(
                        format!("… {} more", total - shown),
                        family,
                        body,
                        muted,
                    ))
                    .with_padding_left(10.0)
                    .finish(),
                );
            } else if total == 0 {
                // A registered host with no live agent — shown as a spine root so
                // it stays navigable/launchable, with a calm hint that it is idle
                // (build_fleet_tree drops agentless hosts; the registry merge
                // re-adds them, see CockpitModel).
                col = col.with_child(
                    Container::new(Self::text(
                        "no agents".to_string(),
                        family,
                        body,
                        muted,
                    ))
                    .with_padding_left(10.0)
                    .finish(),
                );
            }
        }
        Some(col.finish())
    }

    /// One compact Conductor row: `<glyph> <project/name> · <ctx%>`. Local
    /// sessions attach on click; remote sessions are informational (their agent
    /// lives on the host — honest, matching the pane and the `w`-jump).
    fn render_conductor_row(
        &self,
        host_label: &str,
        host_id: Option<&str>,
        project_name: &str,
        session: &SessionSnapshot,
        is_local: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        let glyph_color = match session.state {
            SessionState::Waiting => heat_coloru(HeatLevel::Critical),
            SessionState::Active | SessionState::Monitor => heat_coloru(HeatLevel::Ok),
            SessionState::Idle => muted,
        };
        let dir = Path::new(&session.cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| session.cwd.clone());
        let label = if !session.name.is_empty() {
            session.name.clone()
        } else if !project_name.is_empty() {
            format!("{project_name} — {dir}")
        } else {
            dir
        };

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(glyph_cell(
                session_glyph(session.state),
                glyph_color,
                appearance,
            ))
            .with_child(Shrinkable::new(1.0, Self::text(label, family, body, main)).finish());
        // Always-visible model·effort·context (step 8), compact for the sidebar:
        // the "Opus·High" label in accent, then the colored context-fill %.
        let effort = crate::cockpit::session_effort(session, is_local, host_id);
        let attrs = zaplex_cockpit::session_attrs(
            &session.model,
            effort.as_deref(),
            session.ctx_tokens,
            session.state,
        );
        if !attrs.model_effort.is_empty() {
            let accent = theme.accent().into_solid();
            row = row.with_child(Self::text(attrs.model_effort, family, body, accent));
        }
        if let Some(pct) = attrs.ctx_pct {
            row = row.with_child(ctx_pct_element(pct, attrs.ctx_fill, false, appearance));
        }
        let info = row.with_main_axis_size(MainAxisSize::Max).finish();

        // Attach on click of the info span — for BOTH local and remote sessions
        // now that remote in-place adopt is wired (`attach_fleet_session` resumes
        // a remote session on its host). The compact "◈ review" verb (step 6)
        // stays local-only (remote review isn't wired) and sits alongside.
        let key = host_key(is_local, host_id, &session.session_id);
        let (info_el, review) = match self.conductor_row_states.get(&key).cloned() {
            Some(state) => {
                let action = WorkspaceAction::AttachFleetSession {
                    host: host_label.to_string(),
                    host_id: host_id.map(str::to_string),
                    session_id: session.session_id.clone(),
                    is_local,
                };
                let attach = Hoverable::new(state, move |_mouse| info)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                    .finish();
                let review = if is_local {
                    self.conductor_review_states.get(&key).cloned().map(|st| {
                        let action = WorkspaceAction::ReviewSession {
                            project_root: PathBuf::from(&session.project_root),
                            project_name: session.project_name.clone(),
                        };
                        verb_button(st, "◈", VerbKind::Constructive, appearance, action)
                    })
                } else {
                    None
                };
                (attach, review)
            }
            None => (info, None),
        };

        match review {
            Some(review) => Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(8.0)
                .with_child(Shrinkable::new(1.0, info_el).finish())
                .with_child(review)
                .with_main_axis_size(MainAxisSize::Max)
                .finish(),
            None => info_el,
        }
    }

    fn render_header(
        &self,
        snapshot_len: usize,
        cost5h: f64,
        cost_wk: f64,
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
                format!(
                    "{} account{}",
                    snapshot_len,
                    if snapshot_len == 1 { "" } else { "s" }
                ),
                family,
                sub,
                main,
            ))
            .with_child(Self::text(
                format!("{} 5h · {} wk", format_cost(cost5h), format_cost(cost_wk)),
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
            let mut c = Container::new(Self::text("⤢  Dashboard".to_string(), family, sub, muted))
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

        let body_el: Box<dyn Element> = if snapshot.accounts.is_empty() {
            Container::new(Self::text(
                crate::t!("workspace-left-panel-cockpit-empty"),
                family,
                body,
                muted,
            ))
            .with_uniform_padding(CARD_PADDING)
            .finish()
        } else {
            let cost5h: f64 = snapshot.accounts.iter().map(|a| a.block5h.cost_usd).sum();
            let cost_wk: f64 = snapshot.accounts.iter().map(|a| a.week.cost_usd).sum();

            let mut cards = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(
                    Container::new(self.render_header(
                        snapshot.accounts.len(),
                        cost5h,
                        cost_wk,
                        appearance,
                    ))
                    .with_margin_bottom(CARD_SPACING * 2.0)
                    .finish(),
                );
            // Glanceable Conductor: the unified cross-host inventory, waiting-first.
            let model = CockpitModel::as_ref(app);
            let inventory = model.inventory().clone();
            if let Some(conductor) = self.render_conductor(&inventory, appearance) {
                cards = cards.with_child(
                    Container::new(conductor)
                        .with_margin_bottom(CARD_SPACING * 2.0)
                        .finish(),
                );
            }
            for acct in &snapshot.accounts {
                cards = cards.with_child(self.render_card(acct, appearance));
            }

            ClippedScrollable::vertical(
                self.scroll_state.clone(),
                cards.finish(),
                ScrollbarWidth::Auto,
                theme.disabled_text_color(theme.background()).into(),
                theme.main_text_color(theme.background()).into(),
                ElementFill::None,
            )
            .with_overlayed_scrollbar()
            .finish()
        };

        Container::new(body_el)
            .with_uniform_padding(CARD_PADDING)
            .finish()
    }
}

impl Entity for CockpitPanel {
    type Event = CockpitPanelEvent;
}

/// Sidebar actions (routed back into the view by the action system).
#[derive(Clone, Copy, Debug)]
pub enum CockpitPanelAction {
    OpenDashboardPane,
}

impl TypedActionView for CockpitPanel {
    type Action = CockpitPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CockpitPanelAction::OpenDashboardPane => {
                ctx.emit(CockpitPanelEvent::OpenDashboardPane);
            }
        }
    }
}
