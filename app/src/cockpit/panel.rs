//! `CockpitPanel` — the „KI-Sessions" sidebar view: the live
//! `Host → Project → PTY Session → Agent` tree with the full sidebar height.
//! The account cards are their own toolbelt view (`CockpitAccountsPanel`,
//! #504); the roomy full dashboard remains the main-area pane.

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
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, Element, Fill as ElementFill, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, Padding, ParentAnchor,
    ParentElement, ParentOffsetBounds, Point, Radius, ScrollbarWidth, Shrinkable, Stack, Text,
};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::windowing::{StateEvent, WindowManager};
use warpui::{
    AfterLayoutContext, AppContext, Entity, EventContext, LayoutContext, PaintContext,
    SingletonEntity, SizeConstraint, TypedActionView, View, ViewContext, WindowId,
};
use zaplex_cockpit::{
    fleet_conductor_session_count, format_relative, group_project_sessions,
    host_conductor_session_count, host_ident, session_glyph, session_key, AgentInventoryStatus,
    ConductorSession, FleetTree, HostAvailability, HostNode, Provider, SessionSnapshot,
    SessionState, TaskState, TaskStatus,
};

use crate::cockpit::fleet_details::ManagedFleetInventory;
use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::style::{
    attention_coloru, glyph_cell, hover_row, provider_label, status_dot_coloru, zone_card,
    GLYPH_COL_WIDTH,
};
use crate::settings::AccessibilitySettings;
use crate::ui_components::icons;
use crate::ui_components::window_focus_dimming::WindowFocusDimming;
use crate::WorkspaceAction;

pub(super) const CARD_PADDING: f32 = 8.0;
pub(super) const CARD_SPACING: f32 = 4.0;
const TREE_DEPTH_INDENT: f32 = 16.0;
const TASK_PEEK_WIDTH: f32 = 390.0;
pub(super) const TASK_PEEK_DELAY: Duration = Duration::from_millis(350);
const WAITING_PULSE_PERIOD: Duration = Duration::from_millis(1600);
const WAITING_GLYPH_FOOTPRINT: f32 = GLYPH_COL_WIDTH;
const WAITING_GLYPH_CORE_DIAMETER: f32 = 6.0;
const WAITING_PULSE_REPAINT: Duration = Duration::from_millis(100);

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

pub(super) fn account_count_presentation(
    health: &zaplex_cockpit::ScanHealth,
    account_count: usize,
) -> Option<usize> {
    (account_count > 0 || matches!(health, zaplex_cockpit::ScanHealth::Loaded))
        .then_some(account_count)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WaitingPulseFrame {
    core_opacity: u8,
    ring_diameter: f32,
    ring_opacity: u8,
    repaint: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SessionGlyphPresentation {
    visible_label: &'static str,
    semantic_label: String,
}

fn session_glyph_presentation(state: SessionState) -> SessionGlyphPresentation {
    let semantic_label = match state {
        SessionState::Waiting => crate::t!("cockpit-task-peek-state-waiting"),
        SessionState::Active | SessionState::Monitor => {
            crate::t!("cockpit-task-peek-state-working")
        }
        SessionState::Idle => crate::t!("cockpit-task-peek-state-idle"),
    };
    SessionGlyphPresentation {
        visible_label: session_glyph(state),
        semantic_label,
    }
}

fn waiting_pulse_frame(elapsed: Duration, animate: bool) -> WaitingPulseFrame {
    if !animate {
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

fn waiting_pulse_should_animate(reduce_motion: bool, window_focused: bool) -> bool {
    !reduce_motion && window_focused
}

struct WaitingPulseElement {
    color: ColorU,
    animate: bool,
    started_at: Instant,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl WaitingPulseElement {
    fn new(color: ColorU, animate: bool) -> Self {
        Self {
            color,
            animate,
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
        let frame = waiting_pulse_frame(self.started_at.elapsed(), self.animate);
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

fn host_display_label(host: &HostNode, removed_label: &str, unverified_label: &str) -> String {
    match host.availability {
        HostAvailability::Available => host.host.clone(),
        HostAvailability::Unverified => format!("{} — {unverified_label}", host.host),
        HostAvailability::Removed => format!("{} — {removed_label}", host.host),
    }
}

pub struct CockpitPanel {
    window_id: WindowId,
    session_scroll_state: ClippedScrollStateHandle,
    /// Hover/click state per Conductor session row (complete host + provider +
    /// account + conversation identity), synced against the unified inventory.
    /// Clicking a row attaches the agent.
    conductor_row_states: HashMap<String, MouseStateHandle>,
    conductor_peek_states: HashMap<String, MouseStateHandle>,
    /// Tooltip hover state for each agent status glyph. Kept separate from the
    /// clickable row and task-peek handles so only the glyph owns this tooltip.
    conductor_row_glyph_states: HashMap<String, MouseStateHandle>,
    /// Hover/click state per connected host root, keyed by stable host identity.
    conductor_host_states: HashMap<String, MouseStateHandle>,
    /// Tooltip hover state for each host summary glyph.
    conductor_host_glyph_states: HashMap<String, MouseStateHandle>,
    /// Explicit host expansion overrides. Absent means expanded.
    expanded_hosts: HashMap<String, bool>,
    /// Stable hover state for the waiting-summary glyph in the Sessions header.
    conductor_attention_state: MouseStateHandle,
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
    project_directory_label(project_name, &session.cwd)
}

fn project_directory_label(project_name: &str, cwd: &str) -> String {
    let dir = Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.to_string());
    if project_name == dir.as_str() {
        project_name.to_string()
    } else if !project_name.is_empty() {
        format!("{project_name} — {dir}")
    } else {
        dir
    }
}

#[derive(Debug, Eq, PartialEq)]
struct AgentLeafPresentation<'a> {
    provider: &'static str,
    model: Option<&'a str>,
}

fn agent_leaf_presentation(provider: Provider, model: &str) -> AgentLeafPresentation<'_> {
    AgentLeafPresentation {
        provider: provider_label(provider),
        model: (!model.trim().is_empty()).then_some(model),
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
        let window_manager = WindowManager::handle(ctx);
        ctx.subscribe_to_model(&window_manager, |_, _, event, ctx| match event {
            StateEvent::ValueChanged { current, previous } => {
                if WindowManager::did_window_change_focus(ctx.window_id(), current, previous) {
                    ctx.notify();
                }
            }
        });
        let mut me = Self {
            window_id: ctx.window_id(),
            session_scroll_state: ClippedScrollStateHandle::default(),
            conductor_row_states: HashMap::new(),
            conductor_peek_states: HashMap::new(),
            conductor_row_glyph_states: HashMap::new(),
            conductor_host_states: HashMap::new(),
            conductor_host_glyph_states: HashMap::new(),
            expanded_hosts: HashMap::new(),
            conductor_attention_state: MouseStateHandle::default(),
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
        self.conductor_row_glyph_states
            .retain(|k, _| visible.contains(k));
        for key in routable {
            self.conductor_row_states.entry(key).or_default();
        }
        for key in visible {
            self.conductor_peek_states.entry(key.clone()).or_default();
            self.conductor_row_glyph_states.entry(key).or_default();
        }
        // Connected host handles and explicit expansion overrides.
        self.conductor_host_states
            .retain(|key, _| host_keys.contains(key));
        self.conductor_host_glyph_states
            .retain(|key, _| host_keys.contains(key));
        self.expanded_hosts.retain(|key, _| host_keys.contains(key));
        for key in host_keys {
            self.conductor_host_states.entry(key.clone()).or_default();
            self.conductor_host_glyph_states.entry(key).or_default();
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

    pub(super) fn text(
        s: String,
        family: warpui::fonts::FamilyId,
        size: f32,
        color: ColorU,
    ) -> Box<dyn Element> {
        Text::new_inline(s, family, size).with_color(color).finish()
    }

    pub(super) fn identity_text(
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
        animate_waiting: bool,
        tooltip_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let presentation = session_glyph_presentation(state);
        let glyph = if state == SessionState::Waiting && pulse_waiting {
            WaitingPulseElement::new(attention_coloru(appearance), animate_waiting).finish()
        } else {
            glyph_cell(
                presentation.visible_label,
                status_dot_coloru(state, appearance),
                appearance,
            )
        };
        appearance.ui_builder().overlay_tool_tip_on_element(
            presentation.semantic_label,
            tooltip_state,
            glyph,
            ParentAnchor::TopMiddle,
            ChildAnchor::BottomMiddle,
            vec2f(0.0, -4.0),
        )
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
    pub(super) fn render_zone_header(
        label: String,
        count: Option<usize>,
        trailing: Option<Box<dyn Element>>,
        background: Fill,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let section_size = appearance.ui_font_footnote();
        let faint = theme
            .sub_text_color(background)
            .with_opacity(55)
            .into_solid();
        let muted = theme.sub_text_color(background).into_solid();

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            // The label is scaffolding, not content: muted + uppercase, so the eye
            // reads it as structure and skips to the rows (spec v3 §0).
            .with_child(Self::text(
                label.to_uppercase(),
                family,
                section_size,
                muted,
            ));
        if let Some(count) = count {
            row = row.with_child(
                Shrinkable::new(
                    1.0,
                    Self::text(count.to_string(), family, section_size, faint),
                )
                .finish(),
            );
        }
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
        let body = appearance.ui_font_body_large();
        let main = theme.main_text_color(theme.surface_1()).into_solid();
        let faint = theme
            .sub_text_color(theme.surface_1())
            .with_opacity(55)
            .into_solid();
        let chevron = if expanded {
            icons::Icon::ChevronDown
        } else {
            icons::Icon::ChevronRight
        };
        let label = host_display_label(
            host,
            &crate::t!("cockpit-host-removed"),
            &crate::t!("cockpit-host-unverified"),
        );
        let count = host_conductor_session_count(host);
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(
                ConstrainedBox::new(
                    chevron
                        .to_warpui_icon(theme.sub_text_color(theme.surface_1()))
                        .finish(),
                )
                .with_width(GLYPH_COL_WIDTH)
                .with_height(GLYPH_COL_WIDTH)
                .finish(),
            )
            .with_child(Self::host_status_dot(
                host,
                self.conductor_host_glyph_states
                    .get(key)
                    .cloned()
                    .unwrap_or_default(),
                appearance,
            ))
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
        let main = theme.main_text_color(theme.surface_1()).into_solid();
        let faint = theme
            .sub_text_color(theme.surface_1())
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
                        .to_warpui_icon(theme.sub_text_color(theme.surface_1()))
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
        animate_waiting: bool,
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
                    animate_waiting,
                    self.conductor_attention_state.clone(),
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
                    Some(fleet_conductor_session_count(tree)),
                    attention,
                    appearance.theme().surface_1(),
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
                    .with_padding_left(TREE_DEPTH_INDENT)
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
                            .with_padding_left(TREE_DEPTH_INDENT * 2.0)
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
                                            animate_waiting,
                                            appearance,
                                        ),
                                    )
                                    .with_padding_left(TREE_DEPTH_INDENT * 3.0)
                                    .finish(),
                                );
                            }
                        }
                    }
                }
            }
            if host.projects.is_empty() {
                let message = empty_inventory_message(host.inventory_status, host.is_local);
                col = col.with_child(
                    Container::new(hover_row(
                        Self::text(message, family, body, muted),
                        false,
                        appearance,
                    ))
                    .with_padding_left(TREE_DEPTH_INDENT)
                    .finish(),
                );
            }
        }
        if tree.hosts.is_empty() {
            col = col.with_child(
                Container::new(hover_row(
                    Self::text(crate::t!("cockpit-conductor-empty"), family, body, muted),
                    false,
                    appearance,
                ))
                .with_padding_left(TREE_DEPTH_INDENT)
                .finish(),
            );
        }
        Some(col.finish())
    }

    /// One compact agent leaf: state glyph, provider, and optional model only.
    /// The delayed fixed-size peek retains activity/task detail without adding a
    /// task subrow or changing the tree's geometry.
    #[allow(clippy::too_many_arguments)]
    fn render_conductor_row(
        &self,
        host_label: &str,
        host_id: Option<&str>,
        session: &SessionSnapshot,
        is_local: bool,
        can_attach: bool,
        is_managed: bool,
        animate_waiting: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let model_size = appearance.ui_font_footnote();
        let main = theme.main_text_color(theme.surface_1()).into_solid();
        let muted = theme.sub_text_color(theme.surface_1()).into_solid();
        let presentation = agent_leaf_presentation(session.provider, &session.model);
        let key = session_key(is_local, host_id, session);

        let mut identity = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(6.0)
            .with_child(Self::text(
                presentation.provider.to_string(),
                family,
                body,
                main,
            ));
        if let Some(model) = presentation.model {
            identity = identity.with_child(
                Shrinkable::new(
                    1.0,
                    Self::identity_text(model.to_string(), family, model_size, muted),
                )
                .finish(),
            );
        }

        let mut glance = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(Self::state_glyph(
                session.presented_state(),
                true,
                animate_waiting,
                self.conductor_row_glyph_states
                    .get(&key)
                    .cloned()
                    .unwrap_or_default(),
                appearance,
            ))
            .with_child(Shrinkable::new(1.0, identity.finish()).finish());
        if is_managed {
            glance = glance.with_child(Self::text("◆".to_string(), family, body, muted));
        }
        let glance = glance.with_main_axis_size(MainAxisSize::Max).finish();

        // The whole glance line attaches on click — BOTH local and remote (remote
        // in-place adopt is wired via `attach_fleet_session`).
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
        let session_state = session.presented_state();
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

    #[allow(clippy::too_many_arguments)]
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
        let state_label = session_glyph_presentation(state).semantic_label;
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
    fn host_status_dot(
        host: &HostNode,
        tooltip_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
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
        Self::state_glyph(state, false, false, tooltip_state, appearance)
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
        let bg = theme.surface_1();
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
                            .with_clip(ClipConfig::ellipsis())
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
            hover_row(
                row.with_main_axis_size(MainAxisSize::Max).finish(),
                mouse.is_hovered(),
                appearance,
            )
        })
        .with_cursor(warpui::platform::Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CockpitPanelAction::ToggleProject(pkey_owned.clone()))
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
        let reduce_motion = *AccessibilitySettings::as_ref(app).reduce_motion;
        let animate_waiting = waiting_pulse_should_animate(
            reduce_motion,
            WindowFocusDimming::is_window_focused(self.window_id, app),
        );

        let inventory = CockpitModel::as_ref(app).inventory().clone();
        let managed_fleet = CockpitModel::as_ref(app).managed_fleet().clone();
        // The live object tree remains independent of account discovery: local
        // is always supplied by the model, while remote roots exist only for
        // currently open connections. One flat surface, no registry controls.
        let session_content = if let Some(conductor) =
            self.render_conductor(&inventory, &managed_fleet, animate_waiting, appearance)
        {
            zone_card(conductor, appearance)
                .with_uniform_padding(CARD_PADDING)
                .finish()
        } else {
            Flex::column().finish()
        };

        // The session tree owns the full sidebar height with one scroll state;
        // the accounts are a separate toolbelt view and can no longer be pushed
        // out of view by a long inventory (#504).
        let session_scroll = ClippedScrollable::vertical(
            self.session_scroll_state.clone(),
            session_content,
            ScrollbarWidth::Auto,
            theme.disabled_text_color(theme.surface_2()).into(),
            theme.main_text_color(theme.surface_2()).into(),
            ElementFill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        Container::new(session_scroll)
            .with_uniform_padding(CARD_PADDING)
            .with_background(theme.surface_2())
            .finish()
    }
}

impl Entity for CockpitPanel {
    type Event = ();
}

/// Sidebar actions (routed back into the view by the action system).
#[derive(Clone, Debug)]
pub enum CockpitPanelAction {
    /// Collapse/expand a connected host root. Absent means expanded.
    ToggleHost(String),
    /// Collapse/expand a project group in the Host → Projekt → Session tree,
    /// keyed by `project_key`. Toggles between absent/`true` (expanded, the
    /// default) and `false` (collapsed).
    ToggleProject(String),
    /// Collapse/expand a terminal/PTY Session container. Absent means expanded.
    ToggleSession(String),
}

impl TypedActionView for CockpitPanel {
    type Action = CockpitPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
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
        }
    }
}

fn empty_inventory_message(status: AgentInventoryStatus, is_local: bool) -> String {
    match status {
        AgentInventoryStatus::Pending => crate::t!("cockpit-host-inventory-pending"),
        AgentInventoryStatus::Unavailable => crate::t!("cockpit-host-inventory-unavailable"),
        AgentInventoryStatus::Unsupported => crate::t!("cockpit-host-inventory-unsupported"),
        AgentInventoryStatus::Ready if is_local => crate::t!("cockpit-host-no-local-agents"),
        AgentInventoryStatus::Ready => crate::t!("cockpit-host-no-agents"),
    }
}

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
