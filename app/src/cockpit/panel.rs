//! `CockpitPanel` — the compact Cockpit sidebar: a live
//! `Host → Project → PTY Session → Agent` tree followed by provider-explicit
//! account cards. The roomy full dashboard remains the main-area pane.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use instant::Instant;
use pathfinder_color::ColorU;
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, Element, Fill as ElementFill, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, Padding, ParentAnchor,
    ParentElement, ParentOffsetBounds, Point, Radius, Rect, ScrollbarWidth, Shrinkable, Stack,
    Text,
};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::{
    AfterLayoutContext, AppContext, Entity, EventContext, LayoutContext, PaintContext,
    SingletonEntity, SizeConstraint, TypedActionView, View, ViewContext,
};
use zaplex_cockpit::{
    fleet_conductor_session_count, format_cost, format_relative, group_project_sessions, heat_fill,
    heat_pct_label_with_provenance, host_conductor_session_count, host_ident, session_glyph,
    session_key, AccountUsage, AgentInventoryStatus, ConductorSession, FleetTree, HostAvailability,
    HostNode, Provider, SessionSnapshot, SessionState, TaskState, TaskStatus, UsageProvenance,
};

use crate::cockpit::account_identity;
use crate::cockpit::fleet_details::ManagedFleetInventory;
use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::style::{
    attention_coloru, glyph_cell, hover_row, provider_color_on, provider_label, status_dot_coloru,
    utilisation_coloru, verb_button_colored, zone_card, BLOCK_RADIUS, GLYPH_COL_WIDTH,
};
use crate::settings::AccessibilitySettings;
use crate::ui_components::icons;
use crate::WorkspaceAction;

const CARD_PADDING: f32 = 8.0;
const CARD_SPACING: f32 = 4.0;
const HEAT_BAR_WIDTH: f32 = 90.0;
const HEAT_BAR_HEIGHT: f32 = 6.0;
const TASK_PEEK_WIDTH: f32 = 390.0;
pub(super) const TASK_PEEK_DELAY: Duration = Duration::from_millis(350);
const WAITING_PULSE_PERIOD: Duration = Duration::from_millis(1600);
const WAITING_GLYPH_FOOTPRINT: f32 = GLYPH_COL_WIDTH;
const WAITING_GLYPH_CORE_DIAMETER: f32 = 6.0;
const WAITING_PULSE_REPAINT: Duration = Duration::from_millis(32);

#[derive(Clone, Copy, Debug, PartialEq)]
struct ContainerCountPresentation {
    count: usize,
    attention: bool,
}

fn container_count_presentation(
    expanded: bool,
    count: usize,
    hidden_attention: usize,
) -> Option<ContainerCountPresentation> {
    (!expanded).then_some(ContainerCountPresentation {
        count,
        attention: hidden_attention > 0,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WaitingPulseFrame {
    core_opacity: u8,
    ring_diameter: f32,
    ring_opacity: u8,
    repaint: bool,
}

fn waiting_pulse_frame(elapsed: Duration, reduce_motion: bool) -> WaitingPulseFrame {
    if reduce_motion {
        return WaitingPulseFrame {
            core_opacity: 100,
            ring_diameter: WAITING_GLYPH_CORE_DIAMETER * 1.45,
            ring_opacity: 36,
            repaint: false,
        };
    }

    let phase = (elapsed.as_secs_f32() / WAITING_PULSE_PERIOD.as_secs_f32()).fract();
    let emphasis = ((phase * std::f32::consts::TAU).sin() + 1.0) * 0.5;
    WaitingPulseFrame {
        core_opacity: (88.0 + emphasis * 12.0).round() as u8,
        ring_diameter: WAITING_GLYPH_CORE_DIAMETER * (1.0 + phase),
        ring_opacity: ((1.0 - phase) * 58.0).round() as u8,
        repaint: true,
    }
}

struct WaitingPulseElement {
    color: ColorU,
    reduce_motion: bool,
    started_at: Instant,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl WaitingPulseElement {
    fn new(color: ColorU, reduce_motion: bool) -> Self {
        Self {
            color,
            reduce_motion,
            started_at: Instant::now(),
            size: None,
            origin: None,
        }
    }
}

impl Element for WaitingPulseElement {
    fn layout(
        &mut self,
        _constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        let size = vec2f(WAITING_GLYPH_FOOTPRINT, WAITING_GLYPH_FOOTPRINT);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let frame = waiting_pulse_frame(self.started_at.elapsed(), self.reduce_motion);
        if frame.repaint {
            ctx.repaint_after(WAITING_PULSE_REPAINT);
        }

        let ring_offset = (WAITING_GLYPH_FOOTPRINT - frame.ring_diameter) * 0.5;
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(
                origin + vec2f(ring_offset, ring_offset),
                vec2f(frame.ring_diameter, frame.ring_diameter),
            ))
            .with_border(
                Border::all(1.0)
                    .with_border_color(coloru_with_opacity(self.color, frame.ring_opacity)),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.0)));

        let core_offset = (WAITING_GLYPH_FOOTPRINT - WAITING_GLYPH_CORE_DIAMETER) * 0.5;
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(
                origin + vec2f(core_offset, core_offset),
                vec2f(WAITING_GLYPH_CORE_DIAMETER, WAITING_GLYPH_CORE_DIAMETER),
            ))
            .with_background(coloru_with_opacity(self.color, frame.core_opacity))
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.0)));
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        _event: &warpui::event::DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }
}

fn host_display_label(host: &HostNode, removed_label: &str) -> String {
    match host.availability {
        HostAvailability::Available => host.host.clone(),
        HostAvailability::Removed => format!("{} — {removed_label}", host.host),
    }
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
    /// Hover/click state per connected host root, keyed by stable host identity.
    conductor_host_states: HashMap<String, MouseStateHandle>,
    /// Explicit host expansion overrides. Absent means expanded.
    expanded_hosts: HashMap<String, bool>,
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
    /// Hover/click state and expansion overrides for the PTY Session level.
    conductor_session_states: HashMap<String, MouseStateHandle>,
    expanded_sessions: HashMap<String, bool>,
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

fn agent_leaf_label(provider: Provider, model: &str) -> String {
    let provider = provider_label(provider);
    if model.trim().is_empty() {
        provider.to_string()
    } else {
        format!("{provider} · {model}")
    }
}

fn current_task_title(state: &TaskState) -> Option<&str> {
    state
        .tasks
        .iter()
        .find(|task| task.status == TaskStatus::InProgress)
        .or_else(|| {
            state
                .tasks
                .iter()
                .find(|task| task.status == TaskStatus::Pending)
        })
        .map(|task| task.title.as_str())
}

pub(super) fn task_activity_label(task_state: Option<&TaskState>, relative: &str) -> String {
    task_state.and_then(current_task_title).map_or_else(
        || relative.to_owned(),
        |current| format!("{current} · {relative}"),
    )
}

impl CockpitPanel {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        // Re-render on theme change and whenever the snapshot updates.
        ctx.subscribe_to_model(&Appearance::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&AccessibilitySettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });
        ctx.subscribe_to_model(&CockpitModel::handle(ctx), |me, _, event, ctx| {
            if matches!(event, CockpitEvent::Updated) {
                me.sync_conductor_states(ctx);
                ctx.notify();
            }
        });
        let mut me = Self {
            scroll_state: ClippedScrollStateHandle::default(),
            conductor_row_states: HashMap::new(),
            conductor_peek_states: HashMap::new(),
            card_states: HashMap::new(),
            conductor_host_states: HashMap::new(),
            expanded_hosts: HashMap::new(),
            fleet_total_btn: MouseStateHandle::default(),
            rescan_btn: MouseStateHandle::default(),
            conductor_project_states: HashMap::new(),
            expanded_projects: HashMap::new(),
            conductor_session_states: HashMap::new(),
            expanded_sessions: HashMap::new(),
        };
        me.sync_conductor_states(ctx);
        me
    }

    /// Keep one stable row handle per live fleet session (hover needs a stable
    /// handle across renders); drop handles of sessions that disappeared.
    fn sync_conductor_states(&mut self, ctx: &mut ViewContext<Self>) {
        let (routable, visible, host_keys, project_keys, session_keys) = {
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
            let host_keys: std::collections::HashSet<String> = inv
                .hosts
                .iter()
                .map(|host| host_ident(host.is_local, host.host_id.as_deref()))
                .collect();
            let project_keys: std::collections::HashSet<String> = inv
                .hosts
                .iter()
                .flat_map(|h| {
                    let ident = host_ident(h.is_local, h.host_id.as_deref());
                    h.projects.iter().map(move |p| project_key(&ident, &p.root))
                })
                .collect();
            let session_keys: std::collections::HashSet<String> = inv
                .hosts
                .iter()
                .flat_map(|host| {
                    host.projects.iter().flat_map(move |project| {
                        group_project_sessions(
                            host.is_local,
                            host.host_id.as_deref(),
                            &project.sessions,
                        )
                        .into_iter()
                        .map(|session| session.key)
                    })
                })
                .collect();
            (routable, visible, host_keys, project_keys, session_keys)
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
        // Connected host handles and explicit expansion overrides.
        self.conductor_host_states
            .retain(|key, _| host_keys.contains(key));
        self.expanded_hosts.retain(|key, _| host_keys.contains(key));
        for key in host_keys {
            self.conductor_host_states.entry(key).or_default();
        }
        // Project-group header handles + collapse overrides, keyed by
        // `project_key` (host identity + repository root — never the label alone).
        // Drop projects that vanished so the maps don't grow unbounded; the
        // collapse map keeps only live keys, so absent still means "expanded".
        self.conductor_project_states
            .retain(|k, _| project_keys.contains(k));
        self.expanded_projects
            .retain(|k, _| project_keys.contains(k));
        for key in project_keys {
            self.conductor_project_states.entry(key).or_default();
        }
        self.conductor_session_states
            .retain(|key, _| session_keys.contains(key));
        self.expanded_sessions
            .retain(|key, _| session_keys.contains(key));
        for key in session_keys {
            self.conductor_session_states.entry(key).or_default();
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

    fn state_glyph(
        state: SessionState,
        pulse_waiting: bool,
        reduce_motion: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        if state == SessionState::Waiting && pulse_waiting {
            WaitingPulseElement::new(attention_coloru(appearance), reduce_motion).finish()
        } else {
            glyph_cell(
                session_glyph(state),
                status_dot_coloru(state, appearance),
                appearance,
            )
        }
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
        let identity = account_identity(&acct.account);

        // Provider is the stable headline on every account surface. The colour
        // mark is supplementary; the provider name remains visible in text.
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
                    Self::text(identity.provider.to_string(), family, sub, main),
                )
                .finish(),
            )
            .finish();

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(CARD_SPACING)
            .with_child(header);
        if !identity.subline.is_empty() {
            col = col.with_child(Self::identity_text(identity.subline, family, body, muted));
        }
        col = col
            .with_child(self.heat_bar(
                &crate::t!("cockpit-meter-5h"),
                acct.heat,
                acct.provenance,
                appearance,
            ))
            .with_child(self.heat_bar(
                &crate::t!("cockpit-meter-week"),
                acct.heat_week,
                acct.provenance,
                appearance,
            ));

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
    /// inventory as `Host → Project → Session → Agent`. Local is always present;
    /// remote roots are supplied only by live daemon connections. Every level
    /// starts expanded and uses explicit, stable expansion state rather than
    /// scale-dependent auto-collapse. Agent leaves attach through
    /// [`WorkspaceAction::AttachFleetSession`], the same route as the roomy pane
    /// and the `w`-jump.
    /// Shared quiet zone header: uppercase label, muted total, and at most one
    /// trailing aggregate or affordance. The Sessions header uses that slot for
    /// glyph + needs-attention count, never a repeated status word.
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

    fn render_host_header(
        &self,
        host: &HostNode,
        key: &str,
        expanded: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let faint = theme
            .sub_text_color(theme.background())
            .with_opacity(55)
            .into_solid();
        let chevron = if expanded {
            icons::Icon::ChevronDown
        } else {
            icons::Icon::ChevronRight
        };
        let label = host_display_label(host, &crate::t!("cockpit-host-removed"));
        let count = host_conductor_session_count(host);
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(
                ConstrainedBox::new(
                    chevron
                        .to_warpui_icon(theme.sub_text_color(theme.background()))
                        .finish(),
                )
                .with_width(GLYPH_COL_WIDTH)
                .with_height(GLYPH_COL_WIDTH)
                .finish(),
            )
            .with_child(Self::host_status_dot(host, appearance))
            .with_child(
                Shrinkable::new(1.0, Self::identity_text(label, family, body, main)).finish(),
            );
        if let Some(count) = container_count_presentation(expanded, count, host.needs_me) {
            let color = if count.attention {
                attention_coloru(appearance)
            } else {
                faint
            };
            row = row.with_child(Self::text(count.count.to_string(), family, body, color));
        }
        let row = row.with_main_axis_size(MainAxisSize::Max).finish();
        let handle = self
            .conductor_host_states
            .get(key)
            .cloned()
            .unwrap_or_default();
        let key = key.to_string();
        Hoverable::new(handle, move |mouse| {
            hover_row(row, mouse.is_hovered(), appearance)
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CockpitPanelAction::ToggleHost(key.clone()))
        })
        .finish()
    }

    fn render_session_header(
        &self,
        session: &ConductorSession<'_>,
        project_name: &str,
        expanded: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let faint = theme
            .sub_text_color(theme.background())
            .with_opacity(55)
            .into_solid();
        let chevron = if expanded {
            icons::Icon::ChevronDown
        } else {
            icons::Icon::ChevronRight
        };
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(
                ConstrainedBox::new(
                    chevron
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
                    Self::identity_text(
                        session_identity_label(session.representative, project_name),
                        family,
                        body,
                        main,
                    ),
                )
                .finish(),
            );
        if let Some(count) =
            container_count_presentation(expanded, session.agents.len(), session.needs_me)
        {
            let color = if count.attention {
                attention_coloru(appearance)
            } else {
                faint
            };
            row = row.with_child(Self::text(count.count.to_string(), family, body, color));
        }
        let row = row.with_main_axis_size(MainAxisSize::Max).finish();
        let handle = self
            .conductor_session_states
            .get(&session.key)
            .cloned()
            .unwrap_or_default();
        let key = session.key.clone();
        Hoverable::new(handle, move |mouse| {
            hover_row(row, mouse.is_hovered(), appearance)
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CockpitPanelAction::ToggleSession(key.clone()))
        })
        .finish()
    }

    fn render_conductor(
        &self,
        tree: &FleetTree,
        managed_fleet: &ManagedFleetInventory,
        reduce_motion: bool,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let muted = appearance
            .theme()
            .sub_text_color(appearance.theme().background())
            .into_solid();
        let attention = (tree.needs_me > 0).then(|| {
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(4.0)
                .with_child(Self::state_glyph(
                    SessionState::Waiting,
                    true,
                    reduce_motion,
                    appearance,
                ))
                .with_child(Self::text(
                    tree.needs_me.to_string(),
                    family,
                    body,
                    attention_coloru(appearance),
                ))
                .with_main_axis_size(MainAxisSize::Min)
                .finish()
        });

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            // The same calm row rhythm as the roomy pane, scaled down.
            .with_spacing(3.0)
            .with_child(
                Container::new(Self::render_zone_header(
                    crate::t!("cockpit-zone-sessions").to_string(),
                    fleet_conductor_session_count(tree),
                    attention,
                    appearance,
                ))
                .with_margin_bottom(2.0)
                .finish(),
            );

        for host in &tree.hosts {
            let is_local = host.is_local;
            let ident = host_ident(is_local, host.host_id.as_deref());
            let host_expanded = self.expanded_hosts.get(&ident).copied().unwrap_or(true);
            col = col.with_child(self.render_host_header(host, &ident, host_expanded, appearance));
            if !host_expanded {
                continue;
            }
            for project in &host.projects {
                let pkey = project_key(&ident, &project.root);
                let expanded = self.expanded_projects.get(&pkey).copied().unwrap_or(true);
                let sessions = group_project_sessions(
                    host.is_local,
                    host.host_id.as_deref(),
                    &project.sessions,
                );
                let has_waiting = sessions.iter().any(|session| session.needs_me > 0);
                col = col.with_child(
                    Container::new(self.render_project_header(
                        &pkey,
                        &project.name,
                        sessions.len(),
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
                    for session in sessions {
                        col = col.with_child(
                            Container::new(
                                self.render_session_header(
                                    &session,
                                    &project.name,
                                    self.expanded_sessions
                                        .get(&session.key)
                                        .copied()
                                        .unwrap_or(true),
                                    appearance,
                                ),
                            )
                            .with_padding_left(22.0)
                            .finish(),
                        );
                        if self
                            .expanded_sessions
                            .get(&session.key)
                            .copied()
                            .unwrap_or(true)
                        {
                            for agent in session.agents {
                                col = col.with_child(
                                    Container::new(
                                        self.render_conductor_row(
                                            &host.host,
                                            host.host_id.as_deref(),
                                            agent,
                                            is_local,
                                            host.is_available(),
                                            managed_fleet
                                                .matching_agent_session(
                                                    host.host_id.as_deref(),
                                                    agent,
                                                )
                                                .is_some(),
                                            reduce_motion,
                                            appearance,
                                        ),
                                    )
                                    .with_padding_left(34.0)
                                    .finish(),
                                );
                            }
                        }
                    }
                }
            }
            if host.projects.is_empty() {
                let message = if host.is_local {
                    crate::t!("cockpit-host-no-local-agents")
                } else {
                    match host.inventory_status {
                        AgentInventoryStatus::Ready => crate::t!("cockpit-host-no-agents"),
                        AgentInventoryStatus::Unsupported => {
                            crate::t!("cockpit-host-inventory-unsupported")
                        }
                        AgentInventoryStatus::Unavailable => {
                            crate::t!("cockpit-host-inventory-unavailable")
                        }
                    }
                };
                col = col.with_child(
                    Container::new(Self::text(message, family, body, muted))
                        .with_padding_left(22.0)
                        .finish(),
                );
            }
        }
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
        Some(col.finish())
    }

    /// One compact agent leaf: state glyph, provider, and optional model only.
    /// The delayed fixed-size peek retains activity/task detail without adding a
    /// task subrow or changing the tree's geometry.
    fn render_conductor_row(
        &self,
        host_label: &str,
        host_id: Option<&str>,
        session: &SessionSnapshot,
        is_local: bool,
        can_attach: bool,
        is_managed: bool,
        reduce_motion: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let label = agent_leaf_label(session.provider, &session.model);

        let mut glance = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(Self::state_glyph(
                session.state,
                true,
                reduce_motion,
                appearance,
            ))
            .with_child(
                Shrinkable::new(1.0, Self::identity_text(label, family, body, main)).finish(),
            );
        if is_managed {
            glance = glance.with_child(Self::text("◆".to_string(), family, body, muted));
        }
        let glance = glance.with_main_axis_size(MainAxisSize::Max).finish();

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
                        account_id: session.account_id.clone(),
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
    /// session: a disclosure chevron (`▾` open / `▸` collapsed) + the project name,
    /// with no status dot. The count is absent while expanded and appears only
    /// when collapsed; it turns amber when it hides waiting attention. Clicking
    /// anywhere folds/unfolds.
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
        let count = container_count_presentation(expanded, count, if has_waiting { 1 } else { 0 });
        let handle = self
            .conductor_project_states
            .get(pkey)
            .cloned()
            .unwrap_or_default();
        let pkey_owned = pkey.to_string();
        let name_s = name.to_string();
        Hoverable::new(handle, move |mouse| {
            let name_color = if mouse.is_hovered() { main_c } else { muted_c };
            let mut row = Flex::row()
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
                );
            if let Some(count) = count {
                let count_color = if count.attention {
                    attention_coloru(appearance)
                } else {
                    faint_c
                };
                row = row.with_child(
                    Text::new_inline(count.count.to_string(), family, body)
                        .with_color(count_color)
                        .finish(),
                );
            }
            row.with_main_axis_size(MainAxisSize::Max).finish()
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
        let reduce_motion = *AccessibilitySettings::as_ref(app).reduce_motion;

        let snapshot = CockpitModel::as_ref(app).snapshot().clone();
        let inventory = CockpitModel::as_ref(app).inventory().clone();
        let managed_fleet = CockpitModel::as_ref(app).managed_fleet().clone();
        let mut cards = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);

        // The live object tree remains independent of account discovery: local
        // is always supplied by the model, while remote roots exist only for
        // currently open connections. One flat surface, no registry controls.
        if let Some(conductor) =
            self.render_conductor(&inventory, &managed_fleet, reduce_motion, appearance)
        {
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
    /// Collapse/expand a connected host root. Absent means expanded.
    ToggleHost(String),
    /// Collapse/expand a project group in the Host → Projekt → Session tree,
    /// keyed by `project_key`. Toggles between absent/`true` (expanded, the
    /// default) and `false` (collapsed).
    ToggleProject(String),
    /// Collapse/expand a terminal/PTY Session container. Absent means expanded.
    ToggleSession(String),
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
            CockpitPanelAction::ToggleHost(key) => {
                let current = self.expanded_hosts.get(key).copied().unwrap_or(true);
                self.expanded_hosts.insert(key.clone(), !current);
                ctx.notify();
            }
            CockpitPanelAction::ToggleProject(key) => {
                // Absent = expanded (default); the first toggle collapses to false.
                let cur = self.expanded_projects.get(key).copied().unwrap_or(true);
                self.expanded_projects.insert(key.clone(), !cur);
                ctx.notify();
            }
            CockpitPanelAction::ToggleSession(key) => {
                let current = self.expanded_sessions.get(key).copied().unwrap_or(true);
                self.expanded_sessions.insert(key.clone(), !current);
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
