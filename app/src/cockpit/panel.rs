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
    CornerRadius, CrossAxisAlignment, Element, Empty, Fill as ElementFill, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, Padding, ParentAnchor, ParentElement, Point,
    Radius, ScrollbarWidth, Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::windowing::{StateEvent, WindowManager};
use warpui::{
    AfterLayoutContext, AppContext, Entity, EntityId, EventContext, LayoutContext, PaintContext,
    SingletonEntity, SizeConstraint, TypedActionView, View, ViewContext, WindowId,
};
use zaplex_cockpit::{
    fleet_conductor_session_count, group_project_sessions, host_conductor_session_count,
    host_ident, session_glyph, session_key, AgentInventoryStatus, ConductorSession, FleetTree,
    HostAvailability, HostNode, Provider, SessionSnapshot, SessionState, TaskState, TaskStatus,
};

use crate::cockpit::capabilities::terminal_for_inventory_session;
use crate::cockpit::fleet_details::ManagedFleetInventory;
use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::style::{
    attention_coloru, glyph_cell, hover_row, provider_icon, provider_label, status_dot_coloru,
    tree_row, zone_card, GLYPH_COL_WIDTH,
};
use crate::settings::AccessibilitySettings;
use crate::ui_components::icons;
use crate::ui_components::window_focus_dimming::WindowFocusDimming;
use crate::workspace::ActiveSession;
use crate::WorkspaceAction;

pub(super) const CARD_PADDING: f32 = 8.0;
pub(super) const CARD_SPACING: f32 = 4.0;
const TREE_DEPTH_INDENT: f32 = 16.0;
const PROVIDER_ICON_SIZE: f32 = 11.0;
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
    /// Terminal of this window's focused pane; its session gets the stable
    /// highlight (#505).
    focused_terminal: Option<EntityId>,
    session_scroll_state: ClippedScrollStateHandle,
    /// Hover/click state per Conductor session row (complete host + provider +
    /// account + conversation identity), synced against the unified inventory.
    /// Clicking a row attaches the agent.
    conductor_row_states: HashMap<String, MouseStateHandle>,
    /// Tooltip hover state for each agent status glyph. Kept separate from the
    /// clickable row handle so only the glyph owns this tooltip.
    conductor_row_glyph_states: HashMap<String, MouseStateHandle>,
    /// Tooltip hover state for the managed-fleet marker of an agent row.
    conductor_managed_marker_states: HashMap<String, MouseStateHandle>,
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
    /// Tooltip hover state for a row title; keyed by agent key (leaf rows),
    /// session key (multi-agent rows), project key, or host identity.
    conductor_title_states: HashMap<String, MouseStateHandle>,
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

fn directory_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.to_string())
}

fn project_directory_label(project_name: &str, cwd: &str) -> String {
    let dir = directory_name(cwd);
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

/// Below this many characters the distinguishing rest of a shortened sibling
/// title stops being recognisable, so the dimmed shared prefix gets shorter.
const MIN_DISTINCT_TAIL_CHARS: usize = 8;
/// A shared prefix shorter than this is not worth dimming.
const MIN_SHARED_PREFIX_BYTES: usize = 6;
/// The dimmed rendition of a long shared prefix keeps at most this many
/// characters before its ellipsis (`vault-curator-…`).
const MAX_DIMMED_PREFIX_CHARS: usize = 14;
/// Extra air above each further host group; groups read by spacing, not lines.
const HOST_GROUP_GAP: f32 = 6.0;
/// Extra air above each project group inside a host.
const PROJECT_GROUP_GAP: f32 = 3.0;

/// How loudly a session title reads (#505): an idle session is quieter, so
/// running and waiting work stands out; amber remains the glyph's job alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TitleTone {
    Active,
    Quiet,
}

fn title_tone(state: SessionState) -> TitleTone {
    if state == SessionState::Idle {
        TitleTone::Quiet
    } else {
        TitleTone::Active
    }
}

/// A title longer than this keeps a short last segment visible and shortens in
/// the middle instead of at its end (`feat/checkout-re…-step-2`).
const MIDDLE_CLIP_MIN_CHARS: usize = 20;
/// The kept last segment of a middle-shortened title has at least this many
/// characters, so it still identifies the title …
const MIN_KEPT_SEGMENT_CHARS: usize = 6;
/// … and at most this many, so the shortened head keeps some room.
const MAX_KEPT_SEGMENT_CHARS: usize = 16;
/// Flex share of a merged row's project name: it keeps two thirds of a tight
/// row, the session title beside it gives way first.
const PROJECT_TITLE_FLEX: f32 = 2.0;
/// Air between a merged row's project name and its session title; no glyph
/// separates them.
const MERGED_TITLE_GAP: f32 = 6.0;

/// How a title part reads: in the row's title tone, or dimmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PartTone {
    Title,
    Dim,
}

/// How a title part behaves when the row is too narrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PartFit {
    /// Never shrinks: the distinguishing end of a title.
    Fixed,
    /// Shrinks with an end ellipsis.
    Shrinks,
    /// Shrinks too, but keeps the larger share (a merged row's project name).
    Holds,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LabelPart {
    text: String,
    tone: PartTone,
    fit: PartFit,
    /// Separated from the previous part by air, not by a glyph.
    gap_before: bool,
}

/// A row title as parts that render side by side; `full` is the whole title
/// for the tooltip.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TreeLabel {
    parts: Vec<LabelPart>,
    full: String,
}

impl TreeLabel {
    fn plain(text: &str) -> Self {
        Self {
            parts: middle_clipped(text, PartTone::Title),
            full: text.to_string(),
        }
    }
}

fn label_part(text: &str, tone: PartTone, fit: PartFit) -> LabelPart {
    LabelPart {
        text: text.to_string(),
        tone,
        fit,
        gap_before: false,
    }
}

/// Where the kept end of a long title starts (a separator), so the title
/// shortens in the middle; `None` keeps the plain end ellipsis.
fn kept_suffix_start(text: &str) -> Option<usize> {
    if text.chars().count() <= MIDDLE_CLIP_MIN_CHARS {
        return None;
    }
    kept_suffix(text, MIN_KEPT_SEGMENT_CHARS)
}

/// The separator where a kept end of at least `min_chars` (and at most
/// [`MAX_KEPT_SEGMENT_CHARS`]) characters starts.
fn kept_suffix(text: &str, min_chars: usize) -> Option<usize> {
    let mut start = text.rfind(is_title_separator)?;
    while text[start..].chars().count() < min_chars {
        start = text[..start].rfind(is_title_separator)?;
    }
    (start > 0 && text[start..].chars().count() <= MAX_KEPT_SEGMENT_CHARS).then_some(start)
}

/// One title text as parts: a long text keeps its last segment and shortens
/// before it; anything else shortens at its end.
fn middle_clipped(text: &str, tone: PartTone) -> Vec<LabelPart> {
    match kept_suffix_start(text) {
        Some(start) => vec![
            label_part(&text[..start], tone, PartFit::Shrinks),
            label_part(&text[start..], tone, PartFit::Fixed),
        ],
        None => vec![label_part(text, tone, PartFit::Shrinks)],
    }
}

/// One displayed row of a project's part of the live tree.
#[derive(Debug)]
enum TreeRowKind<'a> {
    /// Collapsible project group header; `count` is what it hides. A project
    /// with a single multi-agent session carries that session's title too.
    Project {
        key: String,
        label: TreeLabel,
        /// The state of its only session when the project row is that
        /// session (merged); its title then takes the session's tone.
        session_state: Option<SessionState>,
        expanded: bool,
        count: usize,
        has_waiting: bool,
        /// Collapsed and hiding the focused pane's session.
        focused: bool,
    },
    /// A PTY session shown as one row with its single agent on it. Without a
    /// title the agent identity is the row's headline.
    SessionLeaf {
        label: Option<TreeLabel>,
        agent: &'a SessionSnapshot,
        focused: bool,
    },
    /// A PTY session hosting several agents; they follow as `Agent` rows.
    SessionContainer {
        key: String,
        label: Option<TreeLabel>,
        state: SessionState,
        expanded: bool,
        count: usize,
        needs_me: usize,
        /// Collapsed and hiding the focused pane's agent.
        focused: bool,
    },
    /// One agent of a multi-agent session.
    Agent {
        agent: &'a SessionSnapshot,
        focused: bool,
    },
}

#[derive(Debug)]
struct TreeRow<'a> {
    /// Indentation level below the host row.
    depth: usize,
    kind: TreeRowKind<'a>,
}

fn is_title_separator(c: char) -> bool {
    matches!(c, '-' | '_' | '/' | '.' | ' ')
}

/// The branch-first title of a session in the tree (spec §2.2, #505): its own
/// registry name, else its git branch, else its linked-worktree name, else its
/// directory where that differs from the project. The **model is never** the
/// identity — parallel agents on one model differ by worktree/branch. `None`
/// when nothing beyond the project's name identifies the session; the title
/// never repeats the project name.
fn session_title(session: &SessionSnapshot, project_name: &str) -> Option<String> {
    [
        Some(session.name.as_str()),
        session.branch.as_deref(),
        session.worktree.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.trim().is_empty() && *value != project_name)
    .map(str::to_string)
    .or_else(|| {
        (project_directory_label(project_name, &session.cwd) != project_name)
            .then(|| directory_name(&session.cwd))
    })
    .filter(|title| !title.trim().is_empty() && title != project_name)
}

/// Where a separator-bounded prefix shared by all sibling titles ends. The cut
/// backs off to an earlier separator until every title keeps at least
/// [`MIN_DISTINCT_TAIL_CHARS`] distinguishing characters; `None` when nothing
/// worth dimming is shared.
fn shared_prefix_cut(titles: &[&str]) -> Option<usize> {
    let (first, rest) = titles.split_first()?;
    if rest.is_empty() {
        return None;
    }
    let mut common = first.len();
    for title in rest {
        let shared = first
            .bytes()
            .zip(title.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        common = common.min(shared);
    }
    while !first.is_char_boundary(common) {
        common -= 1;
    }
    // A title that ends inside the shared part has no distinguishing rest
    // (identical titles included), so nothing may be dimmed away.
    if titles.iter().any(|title| title.len() <= common) {
        return None;
    }
    let mut cut = first[..common].rfind(is_title_separator)? + 1;
    while titles
        .iter()
        .any(|title| title[cut..].chars().count() < MIN_DISTINCT_TAIL_CHARS)
    {
        cut = first[..cut - 1].rfind(is_title_separator)? + 1;
    }
    (cut >= MIN_SHARED_PREFIX_BYTES).then_some(cut)
}

/// Per-title cut for [`split_title`]: each title dims the longest prefix it
/// shares with at least one sibling (by [`shared_prefix_cut`] on that pair),
/// so an unrelated sibling such as `main` never blocks a group of similar
/// names from being shortened.
fn shared_prefix_cuts(titles: &[&str]) -> Vec<Option<usize>> {
    titles
        .iter()
        .enumerate()
        .map(|(index, title)| {
            titles
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .filter_map(|(_, sibling)| shared_prefix_cut(&[*title, *sibling]))
                .max()
        })
        .collect()
}

/// The dimmed form of a shared prefix: a short one stays whole, a long one
/// keeps its leading segments up to [`MAX_DIMMED_PREFIX_CHARS`] plus `…`.
fn abbreviate_shared_prefix(prefix: &str) -> String {
    if prefix.chars().count() <= MAX_DIMMED_PREFIX_CHARS {
        return prefix.to_string();
    }
    let head: String = prefix.chars().take(MAX_DIMMED_PREFIX_CHARS).collect();
    let kept = head
        .rfind(is_title_separator)
        .map_or(head.as_str(), |index| &head[..=index]);
    format!("{kept}…")
}

/// A session title under its project: a prefix shared with a sibling is
/// dimmed and may shrink; the distinguishing rest stays whole when short and
/// shortens in its middle when long.
fn split_title(title: &str, cut: Option<usize>) -> TreeLabel {
    let Some(cut) = cut else {
        return TreeLabel::plain(title);
    };
    let rest = &title[cut..];
    let mut parts = vec![label_part(
        &abbreviate_shared_prefix(&title[..cut]),
        PartTone::Dim,
        PartFit::Shrinks,
    )];
    // The distinguishing rest keeps at least MIN_DISTINCT_TAIL_CHARS fixed:
    // whole when short, otherwise its end, with only the middle shrinking.
    match kept_suffix(rest, MIN_DISTINCT_TAIL_CHARS)
        .filter(|_| rest.chars().count() > MAX_KEPT_SEGMENT_CHARS)
    {
        Some(start) => {
            parts.push(label_part(
                &rest[..start],
                PartTone::Title,
                PartFit::Shrinks,
            ));
            parts.push(label_part(&rest[start..], PartTone::Title, PartFit::Fixed));
        }
        None => parts.push(label_part(rest, PartTone::Title, PartFit::Fixed)),
    }
    TreeLabel {
        parts,
        full: title.to_string(),
    }
}

/// The title of a project shown as its only session (#505): the project name
/// keeps priority, the session's own title follows dimmed after some air and
/// gives way first.
fn merged_label(project_name: &str, session_title: Option<&str>) -> TreeLabel {
    let mut parts = vec![label_part(project_name, PartTone::Title, PartFit::Holds)];
    let Some(title) = session_title else {
        return TreeLabel {
            parts,
            full: project_name.to_string(),
        };
    };
    let mut title_parts = middle_clipped(title, PartTone::Dim);
    if let Some(first) = title_parts.first_mut() {
        first.gap_before = true;
    }
    parts.extend(title_parts);
    TreeLabel {
        parts,
        full: format!("{project_name} / {title}"),
    }
}

/// The B+ display rule (#505) for one project of the live tree. The data
/// model stays `Host → Project → PTY session → Agent`; only the rows differ:
/// - a project with exactly one PTY session is that session: the project name
///   leads, the session's own title follows dimmed; with one agent it is a
///   single leaf row, with several the agents follow the project row;
/// - a PTY session with one agent is one leaf row carrying that agent; an
///   untitled one among several shows the agent as its headline;
/// - only a PTY session with several agents keeps child agent rows;
/// - a title sharing a long prefix with a sibling dims it and keeps the
///   distinct rest; a long title keeps its last segment.
fn project_tree_rows<'a>(
    project_name: &str,
    sessions: Vec<ConductorSession<'a>>,
    project_key: String,
    project_expanded: bool,
    session_expanded: impl Fn(&str) -> bool,
    is_focused: impl Fn(&SessionSnapshot) -> bool,
) -> Vec<TreeRow<'a>> {
    let titles: Vec<Option<String>> = sessions
        .iter()
        .map(|session| session_title(session.representative, project_name))
        .collect();
    let mut rows = Vec::new();
    if let [only] = sessions.as_slice() {
        let label = merged_label(project_name, titles[0].as_deref());
        if let [agent] = only.agents.as_slice() {
            let agent: &'a SessionSnapshot = *agent;
            rows.push(TreeRow {
                depth: 1,
                kind: TreeRowKind::SessionLeaf {
                    label: Some(label),
                    agent,
                    focused: is_focused(agent),
                },
            });
            return rows;
        }
        rows.push(TreeRow {
            depth: 1,
            kind: TreeRowKind::Project {
                key: project_key,
                label,
                session_state: Some(only.state),
                expanded: project_expanded,
                count: only.agents.len(),
                has_waiting: only.needs_me > 0,
                focused: !project_expanded && only.agents.iter().any(|agent| is_focused(*agent)),
            },
        });
        if project_expanded {
            for agent in only.agents.iter().copied() {
                rows.push(TreeRow {
                    depth: 2,
                    kind: TreeRowKind::Agent {
                        agent,
                        focused: is_focused(agent),
                    },
                });
            }
        }
        return rows;
    }

    rows.push(TreeRow {
        depth: 1,
        kind: TreeRowKind::Project {
            key: project_key,
            label: TreeLabel::plain(project_name),
            session_state: None,
            expanded: project_expanded,
            count: sessions.len(),
            has_waiting: sessions.iter().any(|session| session.needs_me > 0),
            focused: !project_expanded
                && sessions
                    .iter()
                    .flat_map(|session| session.agents.iter())
                    .any(|agent| is_focused(*agent)),
        },
    });
    if !project_expanded {
        return rows;
    }
    let titled: Vec<&str> = titles.iter().flatten().map(String::as_str).collect();
    let mut cuts = shared_prefix_cuts(&titled).into_iter();
    for (session, title) in sessions.iter().zip(&titles) {
        let label = title
            .as_deref()
            .map(|title| split_title(title, cuts.next().flatten()));
        if let [agent] = session.agents.as_slice() {
            let agent: &'a SessionSnapshot = *agent;
            rows.push(TreeRow {
                depth: 2,
                kind: TreeRowKind::SessionLeaf {
                    label,
                    agent,
                    focused: is_focused(agent),
                },
            });
            continue;
        }
        let expanded = session_expanded(&session.key);
        rows.push(TreeRow {
            depth: 2,
            kind: TreeRowKind::SessionContainer {
                key: session.key.clone(),
                label,
                state: session.state,
                expanded,
                count: session.agents.len(),
                needs_me: session.needs_me,
                focused: !expanded && session.agents.iter().any(|agent| is_focused(*agent)),
            },
        });
        if expanded {
            for agent in session.agents.iter().copied() {
                rows.push(TreeRow {
                    depth: 3,
                    kind: TreeRowKind::Agent {
                        agent,
                        focused: is_focused(agent),
                    },
                });
            }
        }
    }
    rows
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
        // The focused pane's session carries a stable highlight in the tree.
        // `ActiveSession` also notifies on directory changes in any window, so
        // only a change of this window's focused terminal re-renders.
        ctx.observe(&ActiveSession::handle(ctx), |me, active_session, ctx| {
            let focused = active_session.as_ref(ctx).terminal_view_id(me.window_id);
            if focused != me.focused_terminal {
                me.focused_terminal = focused;
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
            focused_terminal: ActiveSession::as_ref(ctx).terminal_view_id(ctx.window_id()),
            session_scroll_state: ClippedScrollStateHandle::default(),
            conductor_row_states: HashMap::new(),
            conductor_row_glyph_states: HashMap::new(),
            conductor_managed_marker_states: HashMap::new(),
            conductor_host_states: HashMap::new(),
            conductor_host_glyph_states: HashMap::new(),
            expanded_hosts: HashMap::new(),
            conductor_attention_state: MouseStateHandle::default(),
            conductor_project_states: HashMap::new(),
            expanded_projects: HashMap::new(),
            conductor_session_states: HashMap::new(),
            expanded_sessions: HashMap::new(),
            conductor_title_states: HashMap::new(),
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
        self.conductor_row_glyph_states
            .retain(|k, _| visible.contains(k));
        self.conductor_managed_marker_states
            .retain(|k, _| visible.contains(k));
        self.conductor_title_states.retain(|k, _| {
            visible.contains(k)
                || session_keys.contains(k)
                || project_keys.contains(k)
                || host_keys.contains(k)
        });
        for key in routable {
            self.conductor_row_states.entry(key).or_default();
        }
        for key in visible {
            self.conductor_title_states.entry(key.clone()).or_default();
            self.conductor_managed_marker_states
                .entry(key.clone())
                .or_default();
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
            self.conductor_host_glyph_states
                .entry(key.clone())
                .or_default();
            self.conductor_title_states.entry(key).or_default();
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
            self.conductor_project_states
                .entry(key.clone())
                .or_default();
            self.conductor_title_states.entry(key).or_default();
        }
        self.conductor_session_states
            .retain(|key, _| session_keys.contains(key));
        self.expanded_sessions
            .retain(|key, _| session_keys.contains(key));
        for key in session_keys {
            self.conductor_session_states
                .entry(key.clone())
                .or_default();
            self.conductor_title_states.entry(key).or_default();
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
        focused: bool,
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
                Shrinkable::new(
                    1.0,
                    appearance.ui_builder().overlay_tool_tip_on_element(
                        label.clone(),
                        self.conductor_title_states
                            .get(key)
                            .cloned()
                            .unwrap_or_default(),
                        Self::identity_text(label, family, body, main),
                        ParentAnchor::TopMiddle,
                        ChildAnchor::BottomMiddle,
                        vec2f(0.0, -4.0),
                    ),
                )
                .finish(),
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
            tree_row(row, mouse.is_hovered(), focused, appearance)
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CockpitPanelAction::ToggleHost(key.clone()))
        })
        .finish()
    }

    /// An empty cell as wide as a chevron or status glyph, so every row keeps
    /// its text on the shared axis of its depth (#505).
    fn empty_slot() -> Box<dyn Element> {
        ConstrainedBox::new(Empty::new().finish())
            .with_width(GLYPH_COL_WIDTH)
            .with_height(GLYPH_COL_WIDTH)
            .finish()
    }

    fn chevron_slot(expanded: bool, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let chevron = if expanded {
            icons::Icon::ChevronDown
        } else {
            icons::Icon::ChevronRight
        };
        ConstrainedBox::new(
            chevron
                .to_warpui_icon(theme.sub_text_color(theme.surface_1()))
                .finish(),
        )
        .with_width(GLYPH_COL_WIDTH)
        .with_height(GLYPH_COL_WIDTH)
        .finish()
    }

    /// The parts of a title side by side. Fixed parts never clip, shrinking
    /// parts end in an ellipsis; a merged row's project name keeps the larger
    /// share. Parts are separated by air only.
    fn label_parts(
        label: &TreeLabel,
        size: f32,
        color: ColorU,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let family = appearance.ui_font_family();
        let theme = appearance.theme();
        let dim = theme
            .sub_text_color(theme.surface_1())
            .with_opacity(55)
            .into_solid();
        let mut line = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);
        for part in &label.parts {
            let part_color = match part.tone {
                PartTone::Title => color,
                PartTone::Dim => dim,
            };
            let text = match part.fit {
                PartFit::Fixed => Self::text(part.text.clone(), family, size, part_color),
                PartFit::Shrinks | PartFit::Holds => {
                    Self::identity_text(part.text.clone(), family, size, part_color)
                }
            };
            let text = if part.gap_before {
                Container::new(text)
                    .with_padding_left(MERGED_TITLE_GAP)
                    .finish()
            } else {
                text
            };
            line = match part.fit {
                PartFit::Fixed => line.with_child(text),
                PartFit::Shrinks => line.with_child(Shrinkable::new(1.0, text).finish()),
                PartFit::Holds => {
                    line.with_child(Shrinkable::new(PROJECT_TITLE_FLEX, text).finish())
                }
            };
        }
        line.finish()
    }

    /// A row title with the full title as tooltip: any title can end up
    /// shorter than its text in a narrow sidebar.
    fn tree_label(
        &self,
        label: &TreeLabel,
        tooltip_key: &str,
        size: f32,
        color: ColorU,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let line = Self::label_parts(label, size, color, appearance);
        appearance.ui_builder().overlay_tool_tip_on_element(
            label.full.clone(),
            self.conductor_title_states
                .get(tooltip_key)
                .cloned()
                .unwrap_or_default(),
            line,
            ParentAnchor::TopMiddle,
            ChildAnchor::BottomMiddle,
            vec2f(0.0, -4.0),
        )
    }

    /// Provider icon, provider and model of one agent, separated by air, not
    /// by a glyph. As a row's headline (`headline = Some(color)`) the provider
    /// takes the title tone; as the second line under a session title the
    /// provider is muted and the model quieter still. A managed-fleet agent
    /// (#168) ends with the `◆` marker, explained by its tooltip.
    fn agent_identity_line(
        agent: &SessionSnapshot,
        managed_marker: Option<MouseStateHandle>,
        headline: Option<ColorU>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let footnote = appearance.ui_font_footnote();
        let muted = theme.sub_text_color(theme.surface_1()).into_solid();
        let faint = theme
            .sub_text_color(theme.surface_1())
            .with_opacity(55)
            .into_solid();
        let presentation = agent_leaf_presentation(agent.provider, &agent.model);
        let icon = ConstrainedBox::new(
            provider_icon(agent.provider)
                .to_warpui_icon(theme.sub_text_color(theme.surface_1()))
                .finish(),
        )
        .with_width(PROVIDER_ICON_SIZE)
        .with_height(PROVIDER_ICON_SIZE)
        .finish();
        let (provider_size, provider_color, model_color) = match headline {
            Some(color) => (appearance.ui_font_body(), color, muted),
            None => (footnote, muted, faint),
        };
        let line = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(5.0)
            .with_child(icon)
            .with_child(Self::text(
                presentation.provider.to_string(),
                family,
                provider_size,
                provider_color,
            ));
        let line = match presentation.model {
            Some(model) => line.with_child(
                Shrinkable::new(
                    1.0,
                    Self::identity_text(model.to_string(), family, footnote, model_color),
                )
                .finish(),
            ),
            None => line,
        };
        let line = match managed_marker {
            Some(tooltip_state) => {
                line.with_child(appearance.ui_builder().overlay_tool_tip_on_element(
                    crate::t!("cockpit-tree-managed-agent"),
                    tooltip_state,
                    Self::text("◆".to_string(), family, footnote, muted),
                    ParentAnchor::TopMiddle,
                    ChildAnchor::BottomMiddle,
                    vec2f(0.0, -4.0),
                ))
            }
            None => line,
        };
        line.finish()
    }

    /// One agent-carrying row: a PTY session with a single agent (titled or
    /// not) or one agent of a multi-agent session. Clicking attaches the agent
    /// through [`WorkspaceAction::AttachFleetSession`].
    #[allow(clippy::too_many_arguments)]
    fn render_agent_leaf(
        &self,
        host: &HostNode,
        agent: &SessionSnapshot,
        label: Option<&TreeLabel>,
        focused: bool,
        managed_fleet: &ManagedFleetInventory,
        animate_waiting: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let body = appearance.ui_font_body();
        let host_id = host.host_id.as_deref();
        let key = session_key(host.is_local, host_id, agent);
        let state = agent.presented_state();
        let tone_color = match title_tone(state) {
            TitleTone::Active => theme.main_text_color(theme.surface_1()).into_solid(),
            TitleTone::Quiet => theme.sub_text_color(theme.surface_1()).into_solid(),
        };
        let managed_marker = managed_fleet
            .matching_agent_session(host_id, agent)
            .is_some()
            .then(|| {
                self.conductor_managed_marker_states
                    .get(&key)
                    .cloned()
                    .unwrap_or_default()
            });
        let text = match label {
            Some(label) => Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(1.0)
                .with_child(self.tree_label(label, &key, body, tone_color, appearance))
                .with_child(Self::agent_identity_line(
                    agent,
                    managed_marker,
                    None,
                    appearance,
                ))
                .finish(),
            None => Self::agent_identity_line(agent, managed_marker, Some(tone_color), appearance),
        };
        let line = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(6.0)
            .with_child(Self::empty_slot())
            .with_child(Self::state_glyph(
                state,
                true,
                animate_waiting,
                self.conductor_row_glyph_states
                    .get(&key)
                    .cloned()
                    .unwrap_or_default(),
                appearance,
            ))
            .with_child(Shrinkable::new(1.0, text).finish())
            .finish();

        // The whole row attaches on click — BOTH local and remote (remote
        // in-place adopt is wired via `attach_fleet_session`).
        let row_state = host
            .is_available()
            .then(|| self.conductor_row_states.get(&key).cloned())
            .flatten();
        match row_state {
            Some(state) => {
                let action = WorkspaceAction::AttachFleetSession {
                    host: host.host.clone(),
                    host_id: host_id.map(str::to_string),
                    session_id: agent.session_id.clone(),
                    provider: agent.provider,
                    config_dir: agent.config_dir.clone(),
                    account_email: agent.account_email.clone(),
                    account_id: agent.account_id.clone(),
                    is_local: host.is_local,
                };
                Hoverable::new(state, move |mouse| {
                    tree_row(line, mouse.is_hovered(), focused, appearance)
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                .finish()
            }
            None => tree_row(line, false, focused, appearance),
        }
    }

    /// A PTY session that hosts several agents: chevron, aggregate glyph and
    /// title; the count of hidden agents appears only while collapsed.
    #[allow(clippy::too_many_arguments)]
    fn render_session_container(
        &self,
        key: &str,
        label: Option<&TreeLabel>,
        state: SessionState,
        expanded: bool,
        count: usize,
        needs_me: usize,
        focused: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let tone_color = match title_tone(state) {
            TitleTone::Active => theme.main_text_color(theme.surface_1()).into_solid(),
            TitleTone::Quiet => theme.sub_text_color(theme.surface_1()).into_solid(),
        };
        let faint = theme
            .sub_text_color(theme.surface_1())
            .with_opacity(55)
            .into_solid();
        let title = match label {
            Some(label) => self.tree_label(label, key, body, tone_color, appearance),
            None => Self::identity_text(
                crate::t!("cockpit-tree-session-untitled"),
                family,
                body,
                tone_color,
            ),
        };
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(6.0)
            .with_child(Self::chevron_slot(expanded, appearance))
            // No aggregate glyph: each agent row below carries its own state,
            // and one fact has one mark (TECH §2); a collapsed count turns
            // amber instead when it hides a waiting agent.
            .with_child(Self::empty_slot())
            .with_child(Shrinkable::new(1.0, title).finish());
        if let Some(count) = container_count_presentation(expanded, count, needs_me) {
            let color = if count.attention {
                attention_coloru(appearance)
            } else {
                faint
            };
            row = row.with_child(Self::text(count.count.to_string(), family, body, color));
        }
        let row = row.finish();
        let handle = self
            .conductor_session_states
            .get(key)
            .cloned()
            .unwrap_or_default();
        let key = key.to_string();
        Hoverable::new(handle, move |mouse| {
            tree_row(row, mouse.is_hovered(), focused, appearance)
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CockpitPanelAction::ToggleSession(key.clone()))
        })
        .finish()
    }

    fn render_tree_row(
        &self,
        row: &TreeRow<'_>,
        host: &HostNode,
        managed_fleet: &ManagedFleetInventory,
        animate_waiting: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let element = match &row.kind {
            TreeRowKind::Project {
                key,
                label,
                session_state,
                expanded,
                count,
                has_waiting,
                focused,
            } => self.render_project_header(
                key,
                label,
                *session_state,
                *count,
                *has_waiting,
                *expanded,
                *focused,
                appearance,
            ),
            TreeRowKind::SessionLeaf {
                label,
                agent,
                focused,
            } => self.render_agent_leaf(
                host,
                agent,
                label.as_ref(),
                *focused,
                managed_fleet,
                animate_waiting,
                appearance,
            ),
            TreeRowKind::SessionContainer {
                key,
                label,
                state,
                expanded,
                count,
                needs_me,
                focused,
            } => self.render_session_container(
                key,
                label.as_ref(),
                *state,
                *expanded,
                *count,
                *needs_me,
                *focused,
                appearance,
            ),
            TreeRowKind::Agent { agent, focused } => self.render_agent_leaf(
                host,
                agent,
                None,
                *focused,
                managed_fleet,
                animate_waiting,
                appearance,
            ),
        };
        let mut container =
            Container::new(element).with_padding_left(TREE_DEPTH_INDENT * row.depth as f32);
        // Every project group gets the same air, merged single-row ones too.
        if row.depth == 1 {
            container = container.with_margin_top(PROJECT_GROUP_GAP);
        }
        container.finish()
    }

    /// The glanceable **Conductor** for the sidebar: the unified cross-host
    /// inventory as `Host → Project → Session → Agent`, displayed by the B+
    /// rule of #505 ([`project_tree_rows`]). Local is always present; remote
    /// roots are supplied only by live daemon connections. Every level starts
    /// expanded and uses explicit, stable expansion state rather than
    /// scale-dependent auto-collapse. Agent rows attach through
    /// [`WorkspaceAction::AttachFleetSession`], the same route as the roomy
    /// pane and the `w`-jump. The session of the focused pane keeps a stable
    /// highlight.
    #[allow(clippy::too_many_arguments)]
    fn render_conductor(
        &self,
        tree: &FleetTree,
        managed_fleet: &ManagedFleetInventory,
        animate_waiting: bool,
        focused_terminal: Option<EntityId>,
        app: &AppContext,
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

        for (host_index, host) in tree.hosts.iter().enumerate() {
            let is_local = host.is_local;
            let host_id = host.host_id.as_deref();
            let ident = host_ident(is_local, host_id);
            let host_expanded = self.expanded_hosts.get(&ident).copied().unwrap_or(true);
            let is_focused = |agent: &SessionSnapshot| {
                focused_terminal.is_some()
                    && terminal_for_inventory_session(agent, is_local, host_id, app)
                        == focused_terminal
            };
            // A collapsed host that hides the focused pane's agent carries
            // its tint instead.
            let host_focused = !host_expanded
                && host
                    .projects
                    .iter()
                    .flat_map(|project| project.sessions.iter())
                    .any(|agent| is_focused(agent));
            let mut host_row = Container::new(self.render_host_header(
                host,
                &ident,
                host_expanded,
                host_focused,
                appearance,
            ));
            if host_index > 0 {
                host_row = host_row.with_margin_top(HOST_GROUP_GAP);
            }
            col = col.with_child(host_row.finish());
            if !host_expanded {
                continue;
            }
            for project in &host.projects {
                let pkey = project_key(&ident, &project.root);
                let project_expanded = self.expanded_projects.get(&pkey).copied().unwrap_or(true);
                let rows = project_tree_rows(
                    &project.name,
                    group_project_sessions(is_local, host_id, &project.sessions),
                    pkey,
                    project_expanded,
                    |key| self.expanded_sessions.get(key).copied().unwrap_or(true),
                    &is_focused,
                );
                for row in &rows {
                    col = col.with_child(self.render_tree_row(
                        row,
                        host,
                        managed_fleet,
                        animate_waiting,
                        appearance,
                    ));
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
    #[allow(clippy::too_many_arguments)]
    fn render_project_header(
        &self,
        pkey: &str,
        label: &TreeLabel,
        session_state: Option<SessionState>,
        count: usize,
        has_waiting: bool,
        expanded: bool,
        focused: bool,
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
        let title_tooltip = self
            .conductor_title_states
            .get(pkey)
            .cloned()
            .unwrap_or_default();
        let pkey_owned = pkey.to_string();
        let label = label.clone();
        Hoverable::new(handle, move |mouse| {
            // A project row reads as a muted group label; a merged row is its
            // only session and takes that session's title tone instead.
            let name_color = match session_state.map(title_tone) {
                _ if mouse.is_hovered() => main_c,
                Some(TitleTone::Active) => main_c,
                Some(TitleTone::Quiet) | None => muted_c,
            };
            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.0)
                .with_child(
                    ConstrainedBox::new(chevron_icon.to_warpui_icon(chevron_fill).finish())
                        .with_width(GLYPH_COL_WIDTH)
                        .with_height(GLYPH_COL_WIDTH)
                        .finish(),
                )
                // The glyph column stays reserved, so the project name sits on
                // the same text axis as the rows of its depth (#505).
                .with_child(Self::empty_slot())
                .with_child(
                    Shrinkable::new(
                        1.0,
                        appearance.ui_builder().overlay_tool_tip_on_element(
                            label.full.clone(),
                            title_tooltip,
                            Self::label_parts(&label, body, name_color, appearance),
                            ParentAnchor::TopMiddle,
                            ChildAnchor::BottomMiddle,
                            vec2f(0.0, -4.0),
                        ),
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
            tree_row(
                row.with_main_axis_size(MainAxisSize::Max).finish(),
                mouse.is_hovered(),
                focused,
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
        let session_content = if let Some(conductor) = self.render_conductor(
            &inventory,
            &managed_fleet,
            animate_waiting,
            self.focused_terminal,
            app,
            appearance,
        ) {
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
