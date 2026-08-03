//! Cockpit **main-area pane** (C2b) — the roomy dashboard over the
//! `zaplex_cockpit` data spine: aggregate header, per-account cards with the
//! full cost/token matrix (today / 5h block / week), both heat bars and reset
//! timers. The compact glanceable variant is the sidebar (`CockpitPanel`);
//! this pane is a first-class zaplex pane (tab/split/promotable,
//! multi-instance), opened from the sidebar's expand action. See
//! docs/superpowers/specs/2026-07-01-cockpit-native-integration-design.md §3.3.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ChildAnchor, ChildView, Clipped, ClippedScrollStateHandle, ClippedScrollable,
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss, Element, Empty,
    Fill as ElementFill, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Rect,
    RowBackground, ScrollbarWidth, Shrinkable, Stack, Table, TableColumnWidth, TableConfig,
    TableHeader, TableStateHandle, TableVerticalSizing, Text,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};
use zaplex_cockpit::{
    fleet_is_large, format_cost, format_relative, format_reset, format_tokens, heat_fill,
    heat_pct_label_with_provenance, host_auto_collapsed, host_ident, host_key, session_glyph,
    session_key, AccountStatus, AccountUsage, FleetTree, HeatLevel, HostNode, Provider,
    SessionSnapshot, SessionState, UsageProvenance, WindowTotals,
};

use crate::cockpit::capabilities::SessionCapabilities;
use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::style::{
    attention_coloru, cluster_divider, ctx_pct_element, glyph_cell, heat_coloru_on,
    icon_verb_button_tooltip, icon_word_verb, provider_color_on, provider_label, status_dot_coloru,
    utilisation_coloru, verb_button, verb_button_colored, zone_card, VerbKind, BLOCK_RADIUS,
    CONTROL_RADIUS,
};
use crate::editor::{
    EditorView, Event as EditorEvent, SingleLineEditorOptions, TextColors, TextOptions,
};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::search_bar::SearchBar;
use crate::ui_components::icons;
use crate::WorkspaceAction;

const PANE_PADDING: f32 = 16.0;
/// Columns in the session table (spec v3 §4.3). A group header fills the first
/// and leaves the rest empty — the table is flat, so a group is a row.
const TABLE_COLUMNS: usize = 9;
const SESSION_TABLE_HEADER_HEIGHT: f32 = 32.0;
const SESSION_TABLE_ROW_HEIGHT: f32 = 30.0;
const SESSION_TABLE_MAX_VISIBLE_ROWS: usize = 8;

fn session_table_viewport_height(row_count: usize) -> f32 {
    let visible_rows = row_count.min(SESSION_TABLE_MAX_VISIBLE_ROWS);
    SESSION_TABLE_HEADER_HEIGHT + visible_rows as f32 * SESSION_TABLE_ROW_HEIGHT
}

fn session_today_cost(
    is_local: bool,
    session_id: &str,
    local_totals: &BTreeMap<String, WindowTotals>,
) -> Option<f64> {
    is_local
        .then(|| local_totals.get(session_id).map(|totals| totals.cost_usd))
        .flatten()
}

/// The search box's width. Fixed on purpose: an `EditorView` panics when it is
/// measured against an infinite width constraint, which is what a flexible child
/// of a row gets during the intrinsic pass.
const SEARCH_WIDTH: f32 = 220.0;
/// The ⋯ drive's width.
const ROW_MENU_WIDTH: f32 = 216.0;
/// The alias editor's width — fixed for the same reason the search box is.
const ALIAS_EDITOR_WIDTH: f32 = 220.0;
const CARD_PADDING: f32 = 12.0;
const CARD_SPACING: f32 = 8.0;
const HEAT_BAR_WIDTH: f32 = 160.0;
const HEAT_BAR_HEIGHT: f32 = 8.0;
/// Fixed column width for the cost/token matrix cells.
const MATRIX_COL_WIDTH: f32 = 110.0;

/// Parse a `#RRGGBB` or `#RGB` hex string into an opaque color. Returns `None`
/// for anything malformed, so an invalid instances.json override color simply
/// yields no tint (never a panic, never a wrong color).
///
/// The ASCII check is what makes that promise true. `len()` counts **bytes**
/// while the slices below index **char boundaries**, so `#éa` measured 3 and
/// took the shorthand branch, where `&hex[0..1]` cut the `é` in half and
/// panicked — taking the app down while rendering an account card, from a value
/// a user typed into a file by hand. A hex colour is ASCII by definition, so
/// rejecting the rest up front makes every byte a boundary.
fn parse_hex_color(s: &str) -> Option<ColorU> {
    let hex = s.strip_prefix('#')?;
    if !hex.is_ascii() {
        return None;
    }
    let (r, g, b) = match hex.len() {
        6 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        ),
        // Shorthand #RGB → each nibble doubled (f → ff).
        3 => {
            let dup = |c: &str| u8::from_str_radix(c, 16).ok().map(|v| v * 17);
            (dup(&hex[0..1])?, dup(&hex[1..2])?, dup(&hex[2..3])?)
        }
        _ => return None,
    };
    Some(ColorU::new(r, g, b, 255))
}

/// The dashboard view backing the cockpit pane.
/// Which sessions the table lists (P3 filter chips).
///
/// `Waiting`/`Active` read the state; `Idle` is the dormant half F3 discovers —
/// conversations with no process left, still resumable. They are a different
/// list on the account (`idle_sessions`), so the filter is what brings them
/// together with the live ones rather than a `state ==` test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFilter {
    All,
    Waiting,
    Active,
    Idle,
}

impl SessionFilter {
    fn matches(self, state: SessionState) -> bool {
        match self {
            SessionFilter::All => true,
            SessionFilter::Waiting => state == SessionState::Waiting,
            // "running" is every non-idle state — the eye separates working from
            // waiting from resting, not busy-substates (spec v3 §1.1).
            SessionFilter::Active => matches!(state, SessionState::Active | SessionState::Monitor),
            SessionFilter::Idle => state == SessionState::Idle,
        }
    }
}

/// Attention order: waiting, then working, then resting. The same rank every
/// other surface sorts by — a table that ordered its states differently would
/// teach the eye a second vocabulary.
fn state_rank(state: SessionState) -> u8 {
    match state {
        SessionState::Waiting => 0,
        SessionState::Active => 1,
        SessionState::Monitor => 2,
        SessionState::Idle => 3,
    }
}

/// A table group's key: **host + repo root** (F9 groups by repo, but a repo on
/// another machine is another working tree). Falls back to the session's own cwd
/// when it is not in a repo at all.
///
/// The same `host_key` shape every other surface uses, so a display label can
/// never merge two hosts.
fn group_key(row: &(SessionSnapshot, Option<String>, Option<String>, bool)) -> String {
    let (session, _host, host_id, is_local) = row;
    let root = if session.repo_root.is_empty() {
        session.project_root.as_str()
    } else {
        session.repo_root.as_str()
    };
    host_key(*is_local, host_id.as_deref(), root)
}

/// The open ⋯ drive: which row, and where it was clicked (P5).
pub struct RowMenu {
    /// `session_key(is_local, host_id, session)` — the row's complete host,
    /// provider, account, and conversation identity.
    pub row_key: String,
    pub position: Vector2F,
}

/// A sortable column of the session table (P4).
///
/// Sorting happens app-side: `warpui::Table` is layout only — it windows rows,
/// it does not know what they mean. Which is the right split; a table element
/// that sorted would have to understand sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Session,
    Worktree,
    Host,
    Model,
    Context,
    Today,
    Status,
    /// Most-recent activity — the default, because "what changed" is what a
    /// glance is usually after.
    Last,
}

/// The table's sort: a column and a direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub column: SortColumn,
    /// `true` = ascending. Defaults differ per column: text reads A→Z, but a
    /// clock or a cost reads biggest-first, because that is the end you look at.
    pub ascending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            column: SortColumn::Last,
            ascending: false,
        }
    }
}

impl SortColumn {
    /// Which way this column reads when first clicked. Text A→Z; a number or a
    /// timestamp starts at the end that matters — newest, priciest, fullest.
    fn default_ascending(self) -> bool {
        match self {
            SortColumn::Session | SortColumn::Worktree | SortColumn::Host | SortColumn::Model => {
                true
            }
            SortColumn::Context | SortColumn::Today | SortColumn::Last | SortColumn::Status => {
                false
            }
        }
    }
}

/// One line of the flattened table: the sum-tree wants a flat row list, so a
/// group header is a row of its own rather than a nesting level.
enum TableRow {
    /// A project group header (F9: the repo, not one of its worktrees).
    ///
    /// Keyed by **host + repo**, not repo alone: the same path on two machines is
    /// two working trees, and a group that merged them could not say which one
    /// its "+" should launch on — it would silently pick this machine.
    Group {
        /// `host_key(is_local, host_id, repo_root)` — the group key, and what the
        /// collapse state is keyed by.
        key: String,
        name: String,
        /// The host it lives on, `None` for this machine — scopes the group's "+".
        host: Option<String>,
        host_id: Option<String>,
        count: usize,
        collapsed: bool,
    },
    Session {
        session: SessionSnapshot,
        /// The host it runs on (F5) — `None` when it is this machine.
        host: Option<String>,
        host_id: Option<String>,
        is_local: bool,
        /// What it spent today (F4), or `None` when it spent nothing.
        today_cost: Option<f64>,
    },
}

/// The session row selected by a complete [`session_key`]. Kept as a pure
/// lookup so the menu cannot silently fall back to the first account carrying a
/// copied conversation id.
struct MatchedSessionRow<'a> {
    session: &'a SessionSnapshot,
    host: &'a Option<String>,
    host_id: &'a Option<String>,
    is_local: bool,
}

fn matching_session_row<'a>(rows: &'a [TableRow], row_key: &str) -> Option<MatchedSessionRow<'a>> {
    let mut matches = rows.iter().filter_map(|row| match row {
        TableRow::Session {
            session,
            host,
            host_id,
            is_local,
            ..
        } if session_key(*is_local, host_id.as_deref(), session) == row_key => {
            Some(MatchedSessionRow {
                session,
                host,
                host_id,
                is_local: *is_local,
            })
        }
        TableRow::Group { .. } | TableRow::Session { .. } => None,
    });
    let matched = matches.next()?;
    matches.next().is_none().then_some(matched)
}

pub struct CockpitPaneView {
    scroll_state: ClippedScrollStateHandle,
    pane_configuration: ModelHandle<PaneConfiguration>,
    /// Virtualised session table (P3). Its row count and renderer are rebuilt on
    /// every render from the current filter/grouping — the sum-tree does the
    /// windowing, so a hundred sessions cost what ten do.
    table_state: TableStateHandle,
    /// Which sessions the table shows (P3 filter chips).
    session_filter: SessionFilter,
    /// Collapsed project groups in the table, keyed by repo root. Absent =
    /// expanded (groups default open; folding is opt-in, like the sidebar's).
    collapsed_table_groups: HashMap<String, bool>,
    /// Hover/click state per table group header, keyed by repo root.
    table_group_states: HashMap<String, MouseStateHandle>,
    /// Hover state per group's "+" (a new agent scoped to that repo), keyed by
    /// repo root.
    table_plus_states: HashMap<String, MouseStateHandle>,
    /// Hover/click state per table row, keyed by complete [`session_key`] so
    /// copied ids in two accounts never share a click/menu target.
    table_row_states: HashMap<String, MouseStateHandle>,
    /// Hover state per filter chip.
    filter_chip_states: HashMap<&'static str, MouseStateHandle>,
    /// How the table is sorted (P4).
    sort: Sort,
    /// The row whose ⋯ drive is open (P5), if any: its row key and where to
    /// anchor the menu.
    row_menu: Option<RowMenu>,
    /// Hover state per row's ⋯ button, keyed like the row.
    row_dots_states: HashMap<String, MouseStateHandle>,
    /// Hover state per sortable column header.
    sort_header_states: HashMap<&'static str, MouseStateHandle>,
    /// The alias editor (A1), present only while renaming this pane's account.
    /// `Some` is the whole "am I editing" state — a separate bool could disagree
    /// with it.
    alias_editor: Option<ViewHandle<EditorView>>,
    /// Visible write failure for the inline alias editor. The editor remains
    /// open and the last good instances.json stays intact.
    alias_persistence_error: Option<String>,
    /// Hover state for the detail card's ⋯.
    alias_dots_state: MouseStateHandle,
    /// Hover/click state for the account placeholder's "try again" retry. A **stable**
    /// handle — a fresh one each render would drop the click (`Hoverable` tracks
    /// mouse-down in it).
    rescan_btn: MouseStateHandle,
    /// The live search box (P4) — project, branch or worktree.
    search: ViewHandle<EditorView>,
    /// The app's [`SearchBar`] chrome around it (magnifier icon, border) — a
    /// bare rectangle is not a search field (audit P0.1).
    search_bar: ViewHandle<SearchBar>,
    /// Its current text, kept here so the render path doesn't reach into the
    /// editor on every frame.
    search_text: String,
    /// The account this pane belongs to (`Account::key`), or `None` for the
    /// fleet dashboard.
    ///
    /// The pane's identity, not a filter: one pane per account, deduped on this,
    /// titled after it. A single global `selected_account` could only ever show
    /// one account at a time — you could not put two side by side, which is the
    /// whole point of running several subscriptions (spec v3 §4.1 P1).
    account_key: Option<String>,
    focus_handle: Option<PaneFocusHandle>,
    /// Hover state of each session row's "fork" action (key = [`session_key`]).
    /// Synced against the snapshot on every cockpit update so handles persist
    /// across renders (hover needs a stable handle).
    session_fork_states: HashMap<String, MouseStateHandle>,
    /// Hover state of each session row's "fork in worktree" action.
    session_forkwt_states: HashMap<String, MouseStateHandle>,
    /// Hover state of each session row's "▸ adopt" action (resume-in-place):
    /// pull an idle CLI session discovered by the cockpit into a live pane.
    session_adopt_states: HashMap<String, MouseStateHandle>,
    /// Hover state of each session row's "log" action (open transcript).
    session_transcript_states: HashMap<String, MouseStateHandle>,
    /// Whether a session's cwd sits inside a git repo (key = [`session_key`]) —
    /// precomputed on cockpit updates so render never touches the filesystem.
    /// Non-repo cwds simply don't get the worktree action (design §3: toggle
    /// disabled, never a broken session).
    session_in_repo: HashMap<String, bool>,
    /// Hover/click state of each Conductor session row (complete host, provider,
    /// account, and conversation identity). Clicking the row attaches the agent.
    /// Synced against the unified inventory on every update.
    conductor_row_states: HashMap<String, MouseStateHandle>,
    /// Delayed hover state for the read-only session peek. Separate from the
    /// click state so dismissing the overlay cannot disturb row activation.
    conductor_peek_states: HashMap<String, MouseStateHandle>,
    /// Hover state of each Conductor host collapse toggle (key = stable host
    /// identity `host_ident`, not the display label).
    conductor_host_toggle_states: HashMap<String, MouseStateHandle>,
    /// Hover state of each Conductor project collapse toggle (key =
    /// `host_ident\0root`).
    conductor_project_toggle_states: HashMap<String, MouseStateHandle>,
    /// Hover state of the contextual "+" (open the Spawn-Karte pre-scoped to
    /// this host/project) on each Conductor host/project header. Keyed with a
    /// `host:`/`proj:` prefix over the stable host identity (`host:<host_ident>`,
    /// `proj:<host_ident>\0root`) so host and project pluses never collide and a
    /// shared display label never crosses two hosts.
    conductor_plus_states: HashMap<String, MouseStateHandle>,
    /// Explicit user collapse override per host (key = stable host identity
    /// `host_ident`, not the display label). Absent = use the inverse-complexity
    /// auto decision ([`host_auto_collapsed`]); present = the user has toggled it
    /// and their choice wins until the fleet changes.
    collapsed_hosts: HashMap<String, bool>,
    /// Explicit user collapse override per project (key = `host_ident\0root`).
    /// Absent = expanded (projects default open; collapse is opt-in per project).
    collapsed_projects: HashMap<String, bool>,
    /// Hover state of each Conductor session row's review-loop verbs (step 6),
    /// keyed `"{verb}\0{session_key}"` (verb ∈
    /// review/mark/redirect/commit/pr). One combined map (rather than five)
    /// keeps the sync/retain cheap; hover still needs a stable handle per (verb,
    /// session) across renders.
    conductor_review_states: HashMap<String, MouseStateHandle>,
    /// Hover state of each Conductor session row's guardrail verbs (step 7):
    /// `⏸ stop` / `⨯ kill`, keyed `"{verb}\0{host_ident}\0{id}"` like the review-loop
    /// map. Unlike the review cluster, guardrails render on **every** live row
    /// — local and remote — since interrupt/kill are cross-host operations.
    conductor_guardrail_states: HashMap<String, MouseStateHandle>,
    /// Hover state of each Conductor session row's model-lever verbs (step 8):
    /// `⚙ /compact` · `⌫ /clear` · `fork` · `+worktree`, keyed
    /// `"{verb}\0{host_ident}\0{id}"` like the review-loop / guardrail maps. Levers are
    /// local-only (they resume/fork into a local PTY); the compact/clear pair is
    /// Claude-only (Claude Code slash commands).
    conductor_lever_states: HashMap<String, MouseStateHandle>,
    /// Hover state of the Conductor pane's single fleet-wide "Stop all"
    /// control (step 7). Not keyed — one control for the whole pane.
    conductor_stop_all_state: MouseStateHandle,
}

/// The review-loop verbs (step 6) that hang on a local Conductor session row, in
/// render order. Used both to seed their hover-state handles and to render them.
/// Hover-state keys for the review cluster. `"mark"` is the read-marker verb —
/// named for what it does, not for an approval it never performs.
const REVIEW_VERB_KEYS: [&str; 5] = ["review", "mark", "redirect", "commit", "pr"];

/// The guardrail verbs (step 7) that hang on **every** live Conductor session
/// row (local and remote alike — unlike the review cluster, which is
/// local-only): `⏸ stop` (SIGINT, no confirm) and `⨯ kill` (SIGKILL, always
/// confirmed). Used both to seed their hover-state handles and to render them.
const GUARDRAIL_VERB_KEYS: [&str; 2] = ["pause", "kill"];

/// The model-lever verbs (step 8) on a **local** Conductor session row, in
/// render order: `/compact` and `/clear` (Claude Code slash commands, Claude
/// rows only) plus `fork` / `+worktree` (branch the conversation). Used both to
/// seed their hover-state handles and to render them.
const LEVER_VERB_KEYS: [&str; 4] = ["compact", "clear", "fork", "forkwt"];

/// The **redirect** verb's seed prompt: opens a routed agent tab with a
/// follow-up instruction prefilled (not auto-sent) for the user to complete, so
/// they steer the work without leaving zaplex (reuses the `AskAgentRouted`
/// path). Kept trailing-colon so the user types the actual redirect inline.
fn review_redirect_prompt(project_name: &str) -> String {
    let target = if project_name.trim().is_empty() {
        "this project".to_string()
    } else {
        format!("\u{201c}{}\u{201d}", project_name.trim())
    };
    format!("I reviewed the working changes in {target}. Please adjust the approach: ")
}

/// Actions the Conductor rows dispatch back into this pane view (collapse
/// toggles). Attach/jump go to the workspace via [`WorkspaceAction`] instead —
/// they open panes, which is the workspace's job.
#[derive(Clone, Debug)]
pub enum CockpitPaneAction {
    /// Fold/unfold a project group in the session table, keyed by **repo root**
    /// (F9: the repo is the group). Absent = expanded; groups default open.
    ToggleTableGroup(String),
    /// Show only these sessions in the table (P3 filter chips).
    SetSessionFilter(SessionFilter),
    /// Sort the table by this column. Clicking the active column flips the
    /// direction; a new column starts at whichever end it reads from.
    SortBy(SortColumn),
    /// Start renaming this pane's account (A1) — the ⋯ on the detail card.
    StartAliasEdit,
    /// Open the ⋯ drive for a table row (P5), anchored where it was clicked.
    OpenRowMenu { row_key: String, position: Vector2F },
    /// Close it.
    CloseRowMenu,
    /// Fold/unfold a host node (key = stable host identity `host_ident`, not the
    /// display label — two remote daemons can share a label).
    ToggleHost(String),
    /// Fold/unfold a project node (key = `host_ident\0root`).
    ToggleProject(String),
    /// Toggle the user's "I have read this" mark for a session, keyed by the
    /// session's own id (never the row's `host_key` — a daemon's host_id is
    /// regenerated on every start). Persisted by `ReviewedStore`; it tells the
    /// agent nothing and approves nothing.
    /// A local, non-mutating marker — dims the row's review affordance so the
    /// user's eye moves on; toggles off if marked twice.
    MarkReviewed(String),
    /// Re-run the account scan — the retry on the loading/scan-failed/empty
    /// placeholder.
    Rescan,
}

impl CockpitPaneView {
    /// The fleet dashboard — every account at once.
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        Self::for_account(None, ctx)
    }

    /// A pane for one account (`None` = the fleet dashboard).
    pub fn for_account(account_key: Option<String>, ctx: &mut ViewContext<Self>) -> Self {
        // The search box. Filtering happens as you type — a search you have to
        // submit is a search you check, and this one is meant to be glanced at.
        let search = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let theme = appearance.theme();
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    is_password: false,
                    text: TextOptions {
                        font_size_override: Some(appearance.ui_font_body()),
                        font_family_override: Some(appearance.ui_font_family()),
                        text_colors_override: Some(TextColors {
                            default_color: theme.active_ui_text_color(),
                            disabled_color: theme.disabled_ui_text_color(),
                            hint_color: theme.disabled_ui_text_color(),
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ctx,
            );
            // An empty input must say what it is for (audit P0.1).
            editor.set_placeholder_text(crate::t!("cockpit-table-search-placeholder"), ctx);
            editor
        });
        // The shared SearchBar chrome (magnifier + border) around that editor —
        // sized to sit in one line with the filter chips.
        let search_bar = {
            let editor = search.clone();
            ctx.add_typed_action_view(move |ctx| {
                let appearance = Appearance::as_ref(ctx);
                let theme = appearance.theme();
                let mut bar = SearchBar::new(editor);
                bar.with_style(UiComponentStyles {
                    padding: Some(Coords {
                        left: 6.0,
                        right: 6.0,
                        top: 3.0,
                        bottom: 3.0,
                    }),
                    background: Some(theme.surface_2().into()),
                    border_color: Some(theme.split_pane_border_color().into()),
                    border_width: Some(1.0),
                    border_radius: Some(CornerRadius::with_all(Radius::Pixels(CONTROL_RADIUS))),
                    font_size: Some(appearance.ui_font_body()),
                    ..Default::default()
                });
                bar
            })
        };
        ctx.subscribe_to_view(&search, |me: &mut Self, editor, event, ctx| {
            if matches!(event, EditorEvent::Edited(_)) {
                me.search_text = editor.as_ref(ctx).buffer_text(ctx);
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&Appearance::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&CockpitModel::handle(ctx), |me, _, event, ctx| {
            if matches!(event, CockpitEvent::Updated) {
                me.sync_session_action_states(ctx);
                me.sync_table_states(ctx);
                ctx.notify();
            }
        });
        // Titled after the account it shows — no host prefix: an account is a
        // subscription, not something that lives on a machine.
        let title = Self::pane_title(account_key.as_deref(), ctx);
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(title));
        let mut me = Self {
            scroll_state: ClippedScrollStateHandle::default(),
            // Rebuilt with real rows on every render; this is just a valid empty
            // state for the frames before one exists.
            table_state: TableStateHandle::new(0, |_, _| Vec::new()),
            session_filter: SessionFilter::All,
            collapsed_table_groups: HashMap::new(),
            table_group_states: HashMap::new(),
            table_plus_states: HashMap::new(),
            table_row_states: HashMap::new(),
            filter_chip_states: HashMap::new(),
            sort: Sort::default(),
            row_menu: None,
            row_dots_states: HashMap::new(),
            sort_header_states: HashMap::new(),
            alias_editor: None,
            alias_persistence_error: None,
            alias_dots_state: MouseStateHandle::default(),
            rescan_btn: MouseStateHandle::default(),
            search,
            search_bar,
            search_text: String::new(),
            pane_configuration,
            account_key,
            focus_handle: None,
            session_fork_states: HashMap::new(),
            session_forkwt_states: HashMap::new(),
            session_adopt_states: HashMap::new(),
            session_transcript_states: HashMap::new(),
            session_in_repo: HashMap::new(),
            conductor_row_states: HashMap::new(),
            conductor_peek_states: HashMap::new(),
            conductor_host_toggle_states: HashMap::new(),
            conductor_project_toggle_states: HashMap::new(),
            conductor_plus_states: HashMap::new(),
            collapsed_hosts: HashMap::new(),
            collapsed_projects: HashMap::new(),
            conductor_review_states: HashMap::new(),
            conductor_guardrail_states: HashMap::new(),
            conductor_lever_states: HashMap::new(),
            conductor_stop_all_state: MouseStateHandle::default(),
        };
        me.sync_session_action_states(ctx);
        // Seed on construction too: without handles the first frame's rows would
        // render as text, and the pane would look inert until the next update.
        me.sync_table_states(ctx);
        me
    }

    /// Effective collapse state of a host: the user's explicit toggle if set,
    /// otherwise the inverse-complexity auto decision (calm hosts fold when the
    /// fleet is large; a host that needs you never auto-folds).
    fn host_collapsed(&self, host: &HostNode, fleet_large: bool) -> bool {
        self.collapsed_hosts
            .get(&host_ident(host.is_local, host.host_id.as_deref()))
            .copied()
            .unwrap_or_else(|| host_auto_collapsed(host, fleet_large))
    }

    /// Effective collapse state of a project: expanded unless the user folded it.
    /// Keyed by the project's stable host identity, not the display label.
    fn project_collapsed(&self, is_local: bool, host_id: Option<&str>, root: &str) -> bool {
        self.collapsed_projects
            .get(&host_key(is_local, host_id, root))
            .copied()
            .unwrap_or(false)
    }

    /// Keep one stable `MouseStateHandle` per live session for each row action
    /// (hover needs a stable handle across renders) and precompute the
    /// is-inside-a-git-repo bit per session; drop state of sessions that
    /// disappeared.
    /// Seed the table's hover handles against the current inventory.
    ///
    /// A row without a handle renders as plain text: no cursor, no click. That is
    /// how a fold once ate a host's whole header (F8) — so the seed walks the
    /// same set the table draws from, live and dormant, local and remote.
    fn sync_table_states(&mut self, ctx: &mut ViewContext<Self>) {
        // The chips are fixed, so their handles are seeded once and never pruned.
        for key in ["all", "waiting", "active", "idle"] {
            self.filter_chip_states.entry(key).or_default();
        }
        for key in [
            "session", "worktree", "host", "model", "context", "today", "status", "last",
        ] {
            self.sort_header_states.entry(key).or_default();
        }
        let Some(account_key) = self.account_key.clone() else {
            return;
        };
        let model = CockpitModel::as_ref(ctx);
        let Some(acct) = model
            .snapshot()
            .accounts
            .iter()
            .find(|a| a.account.key == account_key)
        else {
            return;
        };
        let tree = model.inventory().clone();

        let mut row_keys: std::collections::HashSet<String> = acct
            .sessions
            .iter()
            .chain(acct.idle_sessions.iter())
            .map(|s| session_key(true, None, s))
            .collect();
        // Group handles key exactly like the rows do (`group_key`), or a group
        // would render without its chevron and its "+".
        let mut group_keys: std::collections::HashSet<String> = acct
            .sessions
            .iter()
            .chain(acct.idle_sessions.iter())
            .map(|s| group_key(&(s.clone(), None, None, true)))
            .collect();
        for row in zaplex_cockpit::sessions_of_account(&tree, &acct.account) {
            row_keys.insert(session_key(row.is_local, row.host_id, row.session));
            group_keys.insert(group_key(&(
                row.session.clone(),
                (!row.is_local).then(|| row.host.to_string()),
                row.host_id.map(str::to_string),
                row.is_local,
            )));
        }

        // Drop what vanished so the maps don't grow with every session ever seen;
        // the collapse map keeps only live keys, so absent still means "expanded".
        self.table_row_states.retain(|k, _| row_keys.contains(k));
        self.table_group_states
            .retain(|k, _| group_keys.contains(k));
        self.table_plus_states.retain(|k, _| group_keys.contains(k));
        self.collapsed_table_groups
            .retain(|k, _| group_keys.contains(k));
        for k in row_keys {
            // Two handles per row: the row itself, and its ⋯ — separate click
            // targets, so neither can fire the other.
            self.row_dots_states.entry(k.clone()).or_default();
            // …plus one per item of whichever drive is open. Seeding them all
            // for every row would be thousands of handles for a menu that shows
            // one row's worth at a time.
            if self.row_menu.as_ref().is_some_and(|m| m.row_key == k) {
                for item in [
                    "adopt",
                    "fork",
                    "forkwt",
                    "compact",
                    "clear",
                    "transcript",
                    "review",
                    "reviewed",
                    "redirect",
                    "commit",
                    "pr",
                    "stop",
                    "kill",
                ] {
                    self.row_dots_states
                        .entry(format!("{k}\u{0}{item}"))
                        .or_default();
                }
            }
            self.table_row_states.entry(k).or_default();
        }
        for k in group_keys {
            self.table_plus_states.entry(k.clone()).or_default();
            self.table_group_states.entry(k).or_default();
        }
    }

    fn sync_session_action_states(&mut self, ctx: &mut ViewContext<Self>) {
        let sessions: Vec<(String, String)> = CockpitModel::as_ref(ctx)
            .snapshot()
            .accounts
            .iter()
            .flat_map(|a| a.sessions.iter())
            .map(|s| (session_key(true, None, s), s.cwd.clone()))
            .collect();
        let live: std::collections::HashSet<&String> = sessions.iter().map(|(id, _)| id).collect();
        self.session_fork_states.retain(|id, _| live.contains(id));
        self.session_forkwt_states.retain(|id, _| live.contains(id));
        self.session_adopt_states.retain(|id, _| live.contains(id));
        self.session_transcript_states
            .retain(|id, _| live.contains(id));
        self.session_in_repo.retain(|id, _| live.contains(id));
        for (key, cwd) in sessions {
            // `.git` may be a dir (repo root) or a file (linked worktree) —
            // `exists()` covers both, so forking from inside a worktree chains.
            let in_repo = Path::new(&cwd).ancestors().any(|p| p.join(".git").exists());
            self.session_in_repo.insert(key.clone(), in_repo);
            self.session_fork_states.entry(key.clone()).or_default();
            self.session_forkwt_states.entry(key.clone()).or_default();
            self.session_adopt_states.entry(key.clone()).or_default();
            self.session_transcript_states.entry(key).or_default();
        }

        // Conductor maps: keyed off the unified cross-host inventory (which
        // includes remote sessions the local `accounts` list never sees), by the
        // complete `session_key` for rows, `host_key` for projects, and bare
        // stable `host_ident` for hosts — never the display label or raw session
        // id. Retain live keys, drop the rest, and prune stale collapse overrides
        // so a disconnected host/account doesn't leak UI state.
        let inv = CockpitModel::as_ref(ctx).inventory();
        let live_rows: std::collections::HashSet<String> = inv
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
        let live_hosts: std::collections::HashSet<String> = inv
            .hosts
            .iter()
            .map(|h| host_ident(h.is_local, h.host_id.as_deref()))
            .collect();
        let live_projects: std::collections::HashSet<String> = inv
            .hosts
            .iter()
            .flat_map(|h| {
                h.projects
                    .iter()
                    .map(move |p| host_key(h.is_local, h.host_id.as_deref(), &p.root))
            })
            .collect();
        self.conductor_row_states
            .retain(|k, _| live_rows.contains(k));
        self.conductor_peek_states
            .retain(|k, _| live_rows.contains(k));
        // Review-loop maps: keyed `"{verb}\0{session_key}"`; the tail after the
        // first `\0` is the complete row key. Retain live rows, drop the rest.
        self.conductor_review_states.retain(|k, _| {
            k.split_once('\u{0}')
                .map(|(_, rest)| live_rows.contains(rest))
                .unwrap_or(false)
        });
        // Guardrail verb map: same key shape as the review-loop map, retained
        // the same way.
        self.conductor_guardrail_states.retain(|k, _| {
            k.split_once('\u{0}')
                .map(|(_, rest)| live_rows.contains(rest))
                .unwrap_or(false)
        });
        // Model-lever verb map: same key shape, retained the same way.
        self.conductor_lever_states.retain(|k, _| {
            k.split_once('\u{0}')
                .map(|(_, rest)| live_rows.contains(rest))
                .unwrap_or(false)
        });
        self.conductor_host_toggle_states
            .retain(|k, _| live_hosts.contains(k));
        self.conductor_project_toggle_states
            .retain(|k, _| live_projects.contains(k));
        self.conductor_plus_states.retain(|k, _| {
            k.strip_prefix("host:")
                .map(|h| live_hosts.contains(h))
                .or_else(|| k.strip_prefix("proj:").map(|p| live_projects.contains(p)))
                .unwrap_or(false)
        });
        self.collapsed_hosts.retain(|k, _| live_hosts.contains(k));
        self.collapsed_projects
            .retain(|k, _| live_projects.contains(k));
        for host in &inv.hosts {
            let hident = host_ident(host.is_local, host.host_id.as_deref());
            self.conductor_host_toggle_states
                .entry(hident.clone())
                .or_default();
            self.conductor_plus_states
                .entry(format!("host:{hident}"))
                .or_default();
            for project in &host.projects {
                let pkey = host_key(host.is_local, host.host_id.as_deref(), &project.root);
                self.conductor_project_toggle_states
                    .entry(pkey.clone())
                    .or_default();
                self.conductor_plus_states
                    .entry(format!("proj:{pkey}"))
                    .or_default();
                for session in &project.sessions {
                    let rk = session_key(host.is_local, host.host_id.as_deref(), session);
                    self.conductor_row_states.entry(rk.clone()).or_default();
                    self.conductor_peek_states.entry(rk.clone()).or_default();
                    for verb in REVIEW_VERB_KEYS {
                        self.conductor_review_states
                            .entry(format!("{verb}\u{0}{rk}"))
                            .or_default();
                    }
                    for verb in GUARDRAIL_VERB_KEYS {
                        self.conductor_guardrail_states
                            .entry(format!("{verb}\u{0}{rk}"))
                            .or_default();
                    }
                    for verb in LEVER_VERB_KEYS {
                        self.conductor_lever_states
                            .entry(format!("{verb}\u{0}{rk}"))
                            .or_default();
                    }
                }
            }
        }
    }

    /// One session-row fork action ("fork" / "+worktree"): muted, accent
    /// on hover, dispatches [`WorkspaceAction::ForkAgentSession`]. `None` when
    /// the provider has no fork mechanism, or — for the worktree variant —
    /// when the session's cwd is not inside a git repo: disabled-by-absence,
    /// never a broken session (fork/worktree design §2/§3).
    fn session_fork_action(
        &self,
        acct: &AccountUsage,
        session: &zaplex_cockpit::SessionSnapshot,
        into_worktree: bool,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let agent = crate::cockpit::agent_of(acct.account.provider);
        let key = session_key(true, None, session);
        // Disabled-by-absence: an agent with no fork mechanism gets no surface.
        // `acct.sessions` is local by contract (see `AccountUsage::sessions`),
        // which is what `is_local` states here; when the table starts showing
        // other hosts' rows it will pass the row's real flag.
        if !SessionCapabilities::of(session, true).can_fork {
            return None;
        }
        if into_worktree && !self.session_in_repo.get(&key).copied().unwrap_or(false) {
            return None;
        }
        let states = if into_worktree {
            &self.session_forkwt_states
        } else {
            &self.session_fork_states
        };
        let state = states.get(&key).cloned()?;

        let label = if into_worktree {
            crate::t!("cockpit-session-fork-worktree")
        } else {
            crate::t!("cockpit-session-fork")
        };

        let action = WorkspaceAction::ForkAgentSession {
            agent,
            session_id: session.session_id.clone(),
            cwd: PathBuf::from(&session.cwd),
            // Non-default accounts pin the fork to the same subscription.
            config_dir: (!acct.account.is_default).then(|| acct.account.config_dir.clone()),
            account_email: acct.account.email.clone(),
            into_worktree,
            // The account pane's fork surface is local by contract (`acct.sessions`
            // are this machine's), matching the `is_local = true` passed to
            // `SessionCapabilities::of` above.
            host: String::new(),
            host_id: None,
            is_local: true,
        };
        Some(icon_word_verb(
            state,
            icons::Icon::GitBranch,
            label.to_string(),
            VerbKind::Constructive,
            appearance,
            action,
        ))
    }

    /// One session-row "▸ adopt" action: resume an idle CLI session in place
    /// (same session, no fork) into a live local pane — the cockpit's
    /// "open = focus/adopt" verb (audit (b)#13, (c)#4). `None` when the provider
    /// has no resume mechanism (disabled-by-absence, never a broken session).
    fn session_adopt_action(
        &self,
        acct: &AccountUsage,
        session: &zaplex_cockpit::SessionSnapshot,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let agent = crate::cockpit::agent_of(acct.account.provider);
        let key = session_key(true, None, session);
        // Disabled-by-absence: an agent with no resume mechanism gets no
        // surface. `acct.sessions` is local by contract — see the fork verb.
        if !SessionCapabilities::of(session, true).can_resume {
            return None;
        }
        let state = self.session_adopt_states.get(&key).cloned()?;

        let action = WorkspaceAction::AdoptAgentSession {
            agent,
            session_id: session.session_id.clone(),
            cwd: PathBuf::from(&session.cwd),
            // Non-default accounts resume on the same subscription.
            config_dir: (!acct.account.is_default).then(|| acct.account.config_dir.clone()),
            account_email: acct.account.email.clone(),
        };
        Some(verb_button(
            state,
            crate::t!("cockpit-session-adopt").to_string(),
            VerbKind::Constructive,
            appearance,
            action,
        ))
    }

    /// One session-row "log" action: open the session's conversation
    /// transcript in a code/text pane (no regression vs claudeplex's transcript
    /// view). Claude-only — transcripts live under a Claude account's
    /// `projects/…/<id>.jsonl`; Codex has no equivalent here, so it gets no
    /// surface (disabled-by-absence).
    fn session_transcript_action(
        &self,
        acct: &AccountUsage,
        session: &zaplex_cockpit::SessionSnapshot,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        if acct.account.provider != Provider::Claude {
            return None;
        }
        let key = session_key(true, None, session);
        let state = self.session_transcript_states.get(&key).cloned()?;

        let action = WorkspaceAction::ViewTranscript {
            session_id: session.session_id.clone(),
            config_dir: acct.account.config_dir.clone(),
            cwd: PathBuf::from(&session.cwd),
            // Follow live: the opened transcript refreshes on each cockpit
            // reconcile (claudeplex-desktop watch parity).
            watch: true,
        };
        Some(icon_word_verb(
            state,
            icons::Icon::History,
            crate::t!("cockpit-session-transcript").to_string(),
            VerbKind::Constructive,
            appearance,
            action,
        ))
    }

    /// The pane's tab title: the account's display name, or the fleet
    /// dashboard's when this pane shows every account.
    ///
    /// The display name is simply `account.label` — the overrides layer has
    /// already replaced it with the user's alias by the time the snapshot exists
    /// (`overrides.apply`), so reading the alias again here would be a second
    /// source for one fact, free to disagree with the sidebar's.
    ///
    /// No host prefix: an account is a subscription. It is reachable from every
    /// machine, so naming it after one would be wrong, not merely noisy.
    fn pane_title(account_key: Option<&str>, ctx: &AppContext) -> String {
        let Some(key) = account_key else {
            return crate::t!("cockpit-pane-title").to_string();
        };
        CockpitModel::as_ref(ctx)
            .snapshot()
            .accounts
            .iter()
            .find(|a| a.account.key == key)
            .map(|a| a.account.label.clone())
            // An account that vanished (signed out, config dir gone) keeps its
            // pane titled by key rather than going blank — honest about what it
            // is showing.
            .unwrap_or_else(|| key.to_string())
    }

    /// The account this pane belongs to, or `None` for the fleet dashboard.
    pub fn account_key(&self) -> Option<&str> {
        self.account_key.as_deref()
    }

    /// The ONE reset-countdown line, shared by the fleet card and the account
    /// detail: `5h ↻ <t> · Wo ↻ <t>` — absent windows drop out, `None` when
    /// neither is known. Labels are the same short meter vocabulary as
    /// [`Self::heat_bar`]'s, so the line reads against the meters above it
    /// (audit P0.4: bare times with no label read as debug output).
    fn reset_line(acct: &AccountUsage, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
        let label_5h = crate::t!("cockpit-meter-5h");
        let label_week = crate::t!("cockpit-meter-week");
        let reset_5h = format_reset(acct.reset5h, now);
        let reset_wk = format_reset(acct.reset_week, now);
        match (reset_5h.is_empty(), reset_wk.is_empty()) {
            (true, true) => None,
            (false, true) => Some(format!("{label_5h} ↻ {reset_5h}")),
            (true, false) => Some(format!("{label_week} ↻ {reset_wk}")),
            (false, false) => Some(format!(
                "{label_5h} ↻ {reset_5h} · {label_week} ↻ {reset_wk}"
            )),
        }
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn text(
        s: String,
        family: warpui::fonts::FamilyId,
        size: f32,
        color: ColorU,
    ) -> Box<dyn Element> {
        Text::new_inline(s, family, size).with_color(color).finish()
    }

    /// The dashboard placeholder, disambiguated by scan health so an empty account
    /// list no longer reads the same whether the first scan is still running, a
    /// config/dir failed to load, or there genuinely are no accounts. The failed
    /// and genuine-empty cases offer a retry (re-run the scan).
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
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(CockpitPaneAction::Rescan))
        .finish();
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.0)
            .with_child(msg_el)
            .with_child(retry_el)
            .finish()
    }

    /// A labelled heat bar (roomier than the sidebar variant). Estimate-driven
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
        // Utilisation, not attention: one shared rule (spec v3 §1.2) — calm grey,
        // true red only at/above the single "fast voll" threshold, contrast-adapted.
        // Same helper the sidebar meters use, so both surfaces read identically.
        let bar_color = utilisation_coloru(fraction, appearance);
        let fill_w = (heat_fill(fraction) as f32) * HEAT_BAR_WIDTH;

        let fill = ConstrainedBox::new(Rect::new().with_background_color(bar_color).finish())
            .with_width(fill_w)
            .with_height(HEAT_BAR_HEIGHT)
            .finish();

        let track = ConstrainedBox::new(
            Container::new(fill)
                .with_background(internal_colors::fg_overlay_1(theme))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CONTROL_RADIUS)))
                .finish(),
        )
        .with_width(HEAT_BAR_WIDTH)
        .with_height(HEAT_BAR_HEIGHT)
        .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.0)
            .with_child(
                ConstrainedBox::new(Self::text(label.to_string(), family, size, muted))
                    .with_width(24.0)
                    .finish(),
            )
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

    /// One matrix cell: a muted label over a value line ("cost · tokens").
    fn matrix_cell(
        &self,
        label: &str,
        totals: &WindowTotals,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        ConstrainedBox::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(2.0)
                .with_child(Self::text(label.to_string(), family, body, muted))
                .with_child(Self::text(
                    format_cost(totals.cost_usd),
                    family,
                    appearance.ui_font_subheading(),
                    main,
                ))
                .with_child(Self::text(format_tokens(totals.total), family, body, muted))
                .finish(),
        )
        .with_width(MATRIX_COL_WIDTH)
        .finish()
    }

    /// A full account card: header (label + plan), both heat bars, the
    /// today/5h/week cost+token matrix, and the reset line.
    /// Flatten this account's sessions — from **every** host — into the table's
    /// row list: a group header per repo, then its sessions.
    ///
    /// Live and dormant sessions are two lists on the account, kept apart on
    /// purpose (an idle conversation is not running work, and the Conductor must
    /// not count it as such). The table is the one surface that wants both, so
    /// this is where they meet — which is also why the Idle chip can exist at all.
    ///
    /// Remote sessions come from the fleet tree via `sessions_of_account`, joined
    /// on `(provider, account_email)`: the only identity a subscription has that
    /// means the same on another machine (F5).
    fn build_table_rows(
        &self,
        acct: &AccountUsage,
        tree: &FleetTree,
        app: &AppContext,
    ) -> Vec<TableRow> {
        let _ = app;
        // (session, host, host_id, is_local)
        let mut all: Vec<(SessionSnapshot, Option<String>, Option<String>, bool)> = Vec::new();
        // This machine's, live and dormant alike. The account's own lists are the
        // only place the dormant ones exist — the fleet tree holds live work.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for s in acct.sessions.iter().chain(acct.idle_sessions.iter()) {
            if seen.insert(session_key(true, None, s)) {
                all.push((s.clone(), None, None, true));
            }
        }
        // …and the same account's sessions on every host, this one included.
        //
        // Deduped by identity rather than by skipping whatever claims to be
        // local: the tree's local rows come from these same account lists today,
        // but relying on that would mean a session appears twice the day it stops
        // being true — and a set costs nothing to be sure.
        for row in zaplex_cockpit::sessions_of_account(tree, &acct.account) {
            let key = session_key(row.is_local, row.host_id, row.session);
            if !seen.insert(key) {
                continue;
            }
            all.push((
                row.session.clone(),
                (!row.is_local).then(|| row.host.to_string()),
                row.host_id.map(str::to_string),
                row.is_local,
            ));
        }

        all.retain(|(s, ..)| self.session_filter.matches(s.state));

        // Live search over the coordinates that identify a session to a human:
        // its project, its branch, its worktree — plus its own name and host.
        // Not the model or the state: those have a chip and a column, and a
        // search that also matched them would make "opus" select half the table.
        let needle = self.search_text.trim().to_lowercase();
        if !needle.is_empty() {
            all.retain(|(s, host, ..)| {
                let hay = [
                    s.project_name.as_str(),
                    s.name.as_str(),
                    s.branch.as_deref().unwrap_or(""),
                    s.worktree.as_deref().unwrap_or(""),
                    host.as_deref().unwrap_or(""),
                ];
                hay.iter().any(|h| h.to_lowercase().contains(&needle))
            });
        }

        // Group by host + repo (F9): three worktrees of one repo are one group
        // with three sessions, each keeping its own worktree in its row — but the
        // same repo on another machine is its own group. Merging those would give
        // the group's "+" no host to launch on, and mix two working trees under
        // one count.
        let mut by_repo: BTreeMap<
            String,
            Vec<(SessionSnapshot, Option<String>, Option<String>, bool)>,
        > = BTreeMap::new();
        for row in all {
            by_repo.entry(group_key(&row)).or_default().push(row);
        }

        let mut rows = Vec::new();
        for (key, mut group) in by_repo {
            // Sort inside the group — the grouping is the outer order, and a
            // sort that broke it apart would just be an ungrouped table.
            let acct_ref = acct;
            group.sort_by(|a, b| {
                let ord = match self.sort.column {
                    SortColumn::Session => zaplex_cockpit::session_label(&a.0)
                        .to_lowercase()
                        .cmp(&zaplex_cockpit::session_label(&b.0).to_lowercase()),
                    SortColumn::Worktree => {
                        a.0.worktree
                            .as_deref()
                            .unwrap_or("")
                            .cmp(b.0.worktree.as_deref().unwrap_or(""))
                    }
                    SortColumn::Host => {
                        a.1.as_deref()
                            .unwrap_or("")
                            .cmp(b.1.as_deref().unwrap_or(""))
                    }
                    SortColumn::Model => a.0.model.cmp(&b.0.model),
                    SortColumn::Context => zaplex_cockpit::context_fill(&a.0.model, a.0.ctx_tokens)
                        .partial_cmp(&zaplex_cockpit::context_fill(&b.0.model, b.0.ctx_tokens))
                        .unwrap_or(std::cmp::Ordering::Equal),
                    SortColumn::Today => {
                        let cost = |id: &str| {
                            acct_ref
                                .today_by_session
                                .get(id)
                                .map(|t| t.cost_usd)
                                .unwrap_or(0.0)
                        };
                        cost(&a.0.session_id)
                            .partial_cmp(&cost(&b.0.session_id))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    }
                    // Waiting first — the whole point of the cockpit's order.
                    SortColumn::Status => state_rank(a.0.state).cmp(&state_rank(b.0.state)),
                    SortColumn::Last => a.0.last_activity.cmp(&b.0.last_activity),
                };
                let ord = if self.sort.ascending {
                    ord
                } else {
                    ord.reverse()
                };
                // A stable tie-break, so two equal rows don't swap places
                // between frames for no reason the user can see.
                ord.then_with(|| a.0.session_id.cmp(&b.0.session_id))
            });
            let (name, host, host_id) = group
                .first()
                .map(|(s, h, hid, _)| (s.project_name.clone(), h.clone(), hid.clone()))
                .unwrap_or_else(|| (key.clone(), None, None));
            let collapsed = self
                .collapsed_table_groups
                .get(&key)
                .copied()
                .unwrap_or(false);
            rows.push(TableRow::Group {
                key: key.clone(),
                name,
                host,
                host_id,
                count: group.len(),
                collapsed,
            });
            if collapsed {
                continue;
            }
            for (session, host, host_id, is_local) in group {
                // What this session spent today (F4). Absent rather than $0.00:
                // a session that has not spent today has no figure, and a zero
                // would read as one.
                let today_cost =
                    session_today_cost(is_local, &session.session_id, &acct.today_by_session);
                rows.push(TableRow::Session {
                    session,
                    host,
                    host_id,
                    is_local,
                    today_cost,
                });
            }
        }
        rows
    }

    /// One table cell: text at the shared body size, optionally right-aligned for
    /// a number column (spec v3 §4.3 — figures line up, tabular).
    fn cell(text: String, color: ColorU, right: bool, appearance: &Appearance) -> Box<dyn Element> {
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let el = Self::text(text, family, body, color);
        if right {
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_child(el)
                .finish()
        } else {
            el
        }
    }

    /// One ⋯ item: a label and what it does. Destructive ones rest muted and
    /// turn amber under the cursor — a danger affordance, transient by
    /// construction, which is why it may borrow the attention colour (§1.3).
    fn menu_item<A: warpui::Action + Clone>(
        &self,
        key: &str,
        label: String,
        kind: VerbKind,
        action: A,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let state = self.row_dots_states.get(key).cloned()?;
        Some(
            Container::new(verb_button(state, &label, kind, appearance, action))
                .with_padding_left(8.0)
                .with_padding_right(8.0)
                .with_padding_top(3.0)
                .with_padding_bottom(3.0)
                .finish(),
        )
    }

    /// **P5** — the ⋯ drive: everything you can do to a session, in one place,
    /// **capability-gated** (F6).
    ///
    /// An item appears only when it can actually work. A Codex session gets no
    /// Stop/Kill — it carries no pid, so the signal path would refuse and answer
    /// with an error toast; offering it would be the row lying. A remote session
    /// gets no Review: `project_root` is a path over there. Disabled-by-absence,
    /// not greyed-out — a menu that lists what it cannot do makes the user read
    /// it twice.
    fn render_row_menu(
        &self,
        acct: &AccountUsage,
        tree: &FleetTree,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let menu = self.row_menu.as_ref()?;
        // Find the row again — the inventory may have moved on since the click.
        let rows = self.build_table_rows(acct, tree, app);
        let MatchedSessionRow {
            session,
            host,
            host_id,
            is_local,
        } = matching_session_row(&rows, &menu.row_key)?;

        let caps = SessionCapabilities::of(session, is_local);
        let agent = crate::cockpit::agent_of(session.provider);
        // The stamped account route pins fork/resume/slash to the subscription
        // that owns this exact session. For a remote session it is a host-side
        // path and is replayed verbatim there; for local it is the local path.
        let config_dir = session.config_dir.clone().map(PathBuf::from);
        let label = zaplex_cockpit::session_label(session);
        let rk = &menu.row_key;
        let k = |suffix: &str| format!("{rk}\u{0}{suffix}");

        let theme = appearance.theme();
        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);
        let push = |el: Option<Box<dyn Element>>, col: &mut Flex| {
            if let Some(el) = el {
                col.add_child(el);
            }
        };

        // Adopt — the row click does this too, but a menu that omitted the
        // primary action would read as if it were missing.
        if caps.can_resume {
            push(
                self.menu_item(
                    &k("adopt"),
                    crate::t!("cockpit-menu-adopt").to_string(),
                    VerbKind::Constructive,
                    WorkspaceAction::AttachFleetSession {
                        host: host.clone().unwrap_or_default(),
                        host_id: host_id.clone(),
                        session_id: session.session_id.clone(),
                        provider: session.provider,
                        config_dir: session.config_dir.clone(),
                        account_email: session.account_email.clone(),
                        is_local,
                    },
                    appearance,
                ),
                &mut col,
            );
        }
        if caps.can_fork {
            push(
                self.menu_item(
                    &k("fork"),
                    crate::t!("cockpit-menu-fork").to_string(),
                    VerbKind::Constructive,
                    WorkspaceAction::ForkAgentSession {
                        agent,
                        session_id: session.session_id.clone(),
                        cwd: PathBuf::from(&session.cwd),
                        config_dir: config_dir.clone(),
                        account_email: session.account_email.clone(),
                        into_worktree: false,
                        host: host.clone().unwrap_or_default(),
                        host_id: host_id.clone(),
                        is_local,
                    },
                    appearance,
                ),
                &mut col,
            );
            // Into a worktree only for a *local* session that is in a repo.
            // Worktree isolation is a local-git feature (the fork opens a fresh
            // sibling worktree on this machine); `session_in_repo` is also probed
            // against the local filesystem, so a remote session could only match
            // via a session-id collision with a local repo session — gating on
            // `is_local` closes that, so a remote fork is always the correct
            // in-place run on its host.
            if is_local
                && self
                    .session_in_repo
                    .get(&session_key(is_local, host_id.as_deref(), session))
                    .copied()
                    .unwrap_or(false)
            {
                push(
                    self.menu_item(
                        &k("forkwt"),
                        crate::t!("cockpit-menu-fork-worktree").to_string(),
                        VerbKind::Constructive,
                        WorkspaceAction::ForkAgentSession {
                            agent,
                            session_id: session.session_id.clone(),
                            cwd: PathBuf::from(&session.cwd),
                            config_dir: config_dir.clone(),
                            account_email: session.account_email.clone(),
                            into_worktree: true,
                            host: host.clone().unwrap_or_default(),
                            host_id: host_id.clone(),
                            is_local,
                        },
                        appearance,
                    ),
                    &mut col,
                );
            }
        }
        if caps.can_slash {
            // `t!` wants a literal, so these are spelled out rather than looped.
            let slash = |command: &str| WorkspaceAction::SlashCommandSession {
                provider: session.provider,
                session_id: session.session_id.clone(),
                cwd: PathBuf::from(&session.cwd),
                config_dir: config_dir.clone(),
                account_email: session.account_email.clone(),
                command: command.to_string(),
                host: host.clone().unwrap_or_default(),
                host_id: host_id.clone(),
                is_local,
            };
            push(
                self.menu_item(
                    &k("compact"),
                    crate::t!("cockpit-session-compact").to_string(),
                    VerbKind::Constructive,
                    slash("/compact"),
                    appearance,
                ),
                &mut col,
            );
            push(
                self.menu_item(
                    &k("clear"),
                    crate::t!("cockpit-session-clear").to_string(),
                    VerbKind::Constructive,
                    slash("/clear"),
                    appearance,
                ),
                &mut col,
            );
        }
        push(
            self.menu_item(
                &k("transcript"),
                crate::t!("cockpit-menu-transcript").to_string(),
                VerbKind::Constructive,
                WorkspaceAction::ViewTranscript {
                    session_id: session.session_id.clone(),
                    config_dir: config_dir.clone().unwrap_or_default(),
                    cwd: PathBuf::from(&session.cwd),
                    // Follow a live conversation; a dormant one has nothing left
                    // to follow, and re-reading it on every reconcile would be
                    // work done to watch a file that cannot change.
                    watch: session.state != SessionState::Idle,
                },
                appearance,
            ),
            &mut col,
        );
        if caps.can_review {
            push(
                self.menu_item(
                    &k("review"),
                    crate::t!("cockpit-menu-review").to_string(),
                    VerbKind::Constructive,
                    WorkspaceAction::ReviewSession {
                        project_root: PathBuf::from(&session.project_root),
                        project_name: session.project_name.clone(),
                    },
                    appearance,
                ),
                &mut col,
            );
            push(
                self.menu_item(
                    &k("reviewed"),
                    crate::t!("cockpit-session-mark-reviewed").to_string(),
                    VerbKind::Constructive,
                    CockpitPaneAction::MarkReviewed(session.session_id.clone()),
                    appearance,
                ),
                &mut col,
            );
            // What you do *after* reading the changes: steer, land, or open a
            // PR. They ride with review because they act on the same working
            // tree — and so on the same "only from this machine" condition.
            push(
                self.menu_item(
                    &k("redirect"),
                    crate::t!("cockpit-session-redirect").to_string(),
                    VerbKind::Constructive,
                    WorkspaceAction::AskAgentRouted {
                        prompt: review_redirect_prompt(&session.project_name),
                        config_dir: config_dir.clone(),
                    },
                    appearance,
                ),
                &mut col,
            );
            push(
                self.menu_item(
                    &k("commit"),
                    crate::t!("cockpit-menu-commit").to_string(),
                    VerbKind::Constructive,
                    WorkspaceAction::CommitReviewChanges {
                        project_root: PathBuf::from(&session.project_root),
                    },
                    appearance,
                ),
                &mut col,
            );
            push(
                self.menu_item(
                    &k("pr"),
                    crate::t!("cockpit-menu-pr").to_string(),
                    VerbKind::Constructive,
                    WorkspaceAction::CreateReviewPr {
                        project_root: PathBuf::from(&session.project_root),
                    },
                    appearance,
                ),
                &mut col,
            );
        }
        if caps.can_signal {
            col.add_child(cluster_divider(appearance));
            push(
                self.menu_item(
                    &k("stop"),
                    crate::t!("cockpit-menu-stop").to_string(),
                    VerbKind::Destructive,
                    WorkspaceAction::StopAgent {
                        host: host.clone().unwrap_or_default(),
                        host_id: host_id.clone(),
                        session_id: session.session_id.clone(),
                        pid: session.pid,
                        process_fingerprint: session.process_fingerprint.clone(),
                        is_local,
                        agent_label: label.clone(),
                    },
                    appearance,
                ),
                &mut col,
            );
            push(
                self.menu_item(
                    &k("kill"),
                    crate::t!("cockpit-menu-kill").to_string(),
                    VerbKind::Destructive,
                    WorkspaceAction::KillAgentRequest {
                        host: host.clone().unwrap_or_default(),
                        host_id: host_id.clone(),
                        session_id: session.session_id.clone(),
                        pid: session.pid,
                        process_fingerprint: session.process_fingerprint.clone(),
                        is_local,
                        agent_label: label,
                        project_name: session.project_name.clone(),
                    },
                    appearance,
                ),
                &mut col,
            );
        }

        let inner = ConstrainedBox::new(
            Container::new(col.finish())
                .with_background(theme.surface_2())
                .with_border(Border::all(1.0).with_border_color(theme.surface_3().into()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(BLOCK_RADIUS)))
                .with_uniform_padding(4.0)
                .finish(),
        )
        .with_width(ROW_MENU_WIDTH)
        .finish();

        Some(
            Dismiss::new(inner)
                .prevent_interaction_with_other_elements()
                .on_dismiss(|ctx, _| {
                    ctx.dispatch_typed_action(CockpitPaneAction::CloseRowMenu);
                })
                .finish(),
        )
    }

    /// **P4** — the table's toolbar: live search, then the filter chips.
    ///
    /// Search and chips narrow the same list and compose: searching inside
    /// "Wartet" is a question people actually have ("is anything waiting on
    /// zaplex?"), and making them exclusive would answer a different one.
    fn render_table_toolbar(
        &self,
        acct: &AccountUsage,
        tree: &FleetTree,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        // The app-wide SearchBar (magnifier + placeholder + border), not a bare
        // rectangle (audit P0.1). Fixed width: an EditorView panics on an
        // infinite width constraint, and a Flex child is measured against one
        // during the intrinsic pass. (`ssh_manager/panel.rs:2144` learned this
        // the same way.)
        let search = ConstrainedBox::new(ChildView::new(&self.search_bar).finish())
            .with_width(SEARCH_WIDTH)
            .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(10.0)
            .with_child(search)
            .with_child(Shrinkable::new(1.0, Empty::new().finish()).finish())
            .with_child(self.render_filter_chips(acct, tree, appearance))
            .finish()
    }

    /// **P3** — the filter chips: which sessions the table lists.
    ///
    /// Each carries its own count, so the chip answers before it is clicked —
    /// "is anything waiting?" is the question the cockpit exists for, and making
    /// the user click to find out would be the wrong trade.
    fn render_filter_chips(
        &self,
        acct: &AccountUsage,
        tree: &FleetTree,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg = theme.background();
        let muted = theme.sub_text_color(bg).into_solid();
        let accent = theme.accent().into_solid();

        // Count against the same set the table draws from — live + dormant, every
        // host — or a chip would promise rows the table then does not show.
        let mut states: Vec<SessionState> = acct
            .sessions
            .iter()
            .chain(acct.idle_sessions.iter())
            .map(|s| s.state)
            .collect();
        for row in zaplex_cockpit::sessions_of_account(tree, &acct.account) {
            if !row.is_local {
                states.push(row.session.state);
            }
        }
        let count = |f: SessionFilter| states.iter().filter(|s| f.matches(**s)).count();

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0);
        for (key, label, filter) in [
            (
                "all",
                crate::t!("cockpit-table-filter-all"),
                SessionFilter::All,
            ),
            (
                "waiting",
                crate::t!("cockpit-table-filter-waiting"),
                SessionFilter::Waiting,
            ),
            (
                "active",
                crate::t!("cockpit-table-filter-active"),
                SessionFilter::Active,
            ),
            (
                "idle",
                crate::t!("cockpit-table-filter-idle"),
                SessionFilter::Idle,
            ),
        ] {
            let Some(state) = self.filter_chip_states.get(key).cloned() else {
                continue;
            };
            let selected = self.session_filter == filter;
            let text = format!("{label} {}", count(filter));
            let color = if selected { accent } else { muted };
            row = row.with_child(verb_button_colored(
                state,
                &text,
                color,
                accent,
                appearance,
                CockpitPaneAction::SetSessionFilter(filter),
            ));
        }
        row.with_main_axis_size(MainAxisSize::Min).finish()
    }

    /// **P3/P4** — the account's sessions, every host, on the virtualised table.
    ///
    /// `warpui::Table` rather than a hand-rolled flex grid provides the shared
    /// column model, sum-tree row store, and viewported row windowing. The card
    /// gives the table a finite eight-row viewport so large session histories
    /// stay clipped and scroll inside the account zone instead of growing over
    /// adjacent cockpit content.
    fn render_sessions_table(
        &self,
        acct: &AccountUsage,
        tree: &FleetTree,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let bg = theme.background();
        // Only the headers and the empty state are drawn here; the row closure
        // takes its own colours from the context it is handed.
        let muted = theme.sub_text_color(bg).into_solid();

        let rows = self.build_table_rows(acct, tree, app);
        if rows.is_empty() {
            // An empty state is a designed state, not a debug printout: the
            // line sits centered with room around it (audit P0.7).
            return Container::new(
                Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(Self::text(
                        crate::t!("cockpit-table-empty"),
                        family,
                        body,
                        muted,
                    ))
                    .finish(),
            )
            .with_padding_top(24.0)
            .with_padding_bottom(24.0)
            .finish();
        }

        let main = theme.main_text_color(bg).into_solid();
        // A header says what it sorts by and which way — a caret only on the
        // active one, so the row of headers stays quiet until it means something.
        let header =
            |key: &'static str, label: String, col: SortColumn, right: bool| -> TableHeader {
                let active = self.sort.column == col;
                let text = if active {
                    format!("{label} {}", if self.sort.ascending { "▲" } else { "▼" })
                } else {
                    label
                };
                let color = if active { main } else { muted };
                let el = Self::cell(text, color, right, appearance);
                let el: Box<dyn Element> = match self.sort_header_states.get(key).cloned() {
                    Some(state) => Hoverable::new(state, move |_m| el)
                        .with_cursor(Cursor::PointingHand)
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(CockpitPaneAction::SortBy(col))
                        })
                        .finish(),
                    None => el,
                };
                TableHeader::new(el)
            };

        // Everything the render closure needs, owned: it outlives this frame.
        let rows_for_render = std::sync::Arc::new(rows);
        let rows_len = rows_for_render.len();
        let group_states = self.table_group_states.clone();
        let row_states = self.table_row_states.clone();
        let peek_states = self.conductor_peek_states.clone();
        let dots_states = self.row_dots_states.clone();
        let plus_states = self.table_plus_states.clone();

        self.table_state.set_row_count(rows_len);
        // The closure outlives this frame, so it reads the appearance from the
        // context it is handed rather than capturing this frame's reference —
        // which also means a theme switch repaints the rows without rebuilding
        // the table.
        self.table_state.set_row_render_fn(move |i, app| {
            let appearance = Appearance::as_ref(app);
            let theme = appearance.theme();
            let bg = theme.background();
            let main = theme.main_text_color(bg).into_solid();
            let muted = theme.sub_text_color(bg).into_solid();
            let faint = theme.sub_text_color(bg).with_opacity(55).into_solid();
            let family = appearance.ui_font_family();
            let body = appearance.ui_font_body();

            match rows_for_render.get(i) {
                None => vec![],
                Some(TableRow::Group {
                    key,
                    name,
                    host,
                    host_id,
                    count,
                    collapsed,
                }) => {
                    // The group header spans the row by putting everything in the
                    // first cell: the table is flat, so a group is a row that
                    // simply fills one column and leaves the rest empty.
                    let chev = if *collapsed { "▸" } else { "▾" };
                    let inner = Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(6.0)
                        .with_child(Self::text(chev.to_string(), family, body, faint))
                        .with_child(Self::text(name.clone(), family, body, main))
                        .with_child(Self::text(count.to_string(), family, body, faint))
                        .with_main_axis_size(MainAxisSize::Min)
                        .finish();
                    // The tree's project "+" lands here: a new agent pre-bound to
                    // the repo you are looking at. Without a successor, folding
                    // the tree away would take the scoped launch with it.
                    let plus: Option<Box<dyn Element>> = plus_states.get(key).cloned().map(|st| {
                        // Scoped to the group's OWN host, not this machine.
                        // The tree's plus carried the host identity; passing
                        // None here would launch a remote project's agent
                        // locally — the exact regression P6 must not ship.
                        let (host_owned, host_id_owned) = (host.clone(), host_id.clone());
                        let root_owned = zaplex_cockpit::split_host_key(key)
                            .map(|(_, root)| root.to_string())
                            .unwrap_or_else(|| key.clone());
                        verb_button(
                            st,
                            "+",
                            VerbKind::Constructive,
                            appearance,
                            WorkspaceAction::OpenSpawnCard {
                                registry_node_id: None,
                                host_id: host_id_owned,
                                host: host_owned,
                                project: Some(PathBuf::from(root_owned)),
                            },
                        )
                    });
                    let first: Box<dyn Element> = match group_states.get(key).cloned() {
                        Some(state) => {
                            let key = key.clone();
                            Hoverable::new(state, move |_m| inner)
                                .with_cursor(Cursor::PointingHand)
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(CockpitPaneAction::ToggleTableGroup(
                                        key.clone(),
                                    ))
                                })
                                .finish()
                        }
                        None => inner,
                    };
                    let first = match plus {
                        Some(plus) => Flex::row()
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_spacing(8.0)
                            .with_child(first)
                            .with_child(plus)
                            .with_main_axis_size(MainAxisSize::Min)
                            .finish(),
                        None => first,
                    };
                    let mut cells: Vec<Box<dyn Element>> = vec![first];
                    for _ in 1..TABLE_COLUMNS {
                        cells.push(Empty::new().finish());
                    }
                    cells
                }
                Some(TableRow::Session {
                    session,
                    host,
                    host_id,
                    is_local,
                    today_cost,
                }) => {
                    // Session: provider dot + label. The dot is the account's
                    // colour, contrast-picked; the row's *state* is the status
                    // column's job, and saying it twice is what §1.3 forbids.
                    let name = zaplex_cockpit::session_label(session);
                    let sess = Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(6.0)
                        .with_child(
                            ConstrainedBox::new(
                                Rect::new()
                                    .with_background_color(provider_color_on(
                                        session.provider,
                                        bg.into_solid(),
                                    ))
                                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.0)))
                                    .finish(),
                            )
                            .with_width(5.0)
                            .with_height(5.0)
                            .finish(),
                        )
                        .with_child(
                            Shrinkable::new(1.0, Self::text(name, family, body, main)).finish(),
                        )
                        .with_main_axis_size(MainAxisSize::Max)
                        .finish();

                    let rk = session_key(*is_local, host_id.as_deref(), session);
                    let sess_cell: Box<dyn Element> = match row_states.get(&rk).cloned() {
                        Some(state) => {
                            let action = WorkspaceAction::AttachFleetSession {
                                host: host.clone().unwrap_or_default(),
                                host_id: host_id.clone(),
                                session_id: session.session_id.clone(),
                                provider: session.provider,
                                config_dir: session.config_dir.clone(),
                                account_email: session.account_email.clone(),
                                is_local: *is_local,
                            };
                            Hoverable::new(state, move |_m| sess)
                                .with_cursor(Cursor::PointingHand)
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(action.clone())
                                })
                                .finish()
                        }
                        None => sess,
                    };
                    let sess_cell = match peek_states.get(&rk).cloned() {
                        Some(peek_state) => {
                            let peek_title = zaplex_cockpit::session_label(session);
                            let peek_account = session.account_email.as_ref().map_or_else(
                                || provider_label(session.provider).to_owned(),
                                |email| format!("{} · {email}", provider_label(session.provider)),
                            );
                            let peek_host = host.clone().unwrap_or_else(|| {
                                crate::t!("cockpit-table-host-local").to_string()
                            });
                            let peek_cwd = session.cwd.clone();
                            let session_state = session.state;
                            let task_state = session.task_state.clone();
                            let relative =
                                format_relative(session.last_activity, chrono::Utc::now());
                            let activity =
                                super::panel::task_activity_label(task_state.as_ref(), &relative);
                            Hoverable::new(peek_state, move |mouse| {
                                let mut stack = Stack::new().with_child(sess_cell);
                                if mouse.is_hovered() {
                                    stack.add_positioned_overlay_child(
                                        super::panel::CockpitPanel::render_task_peek(
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
                            .with_hover_in_delay(super::panel::TASK_PEEK_DELAY)
                            .finish()
                        }
                        None => sess_cell,
                    };

                    // Worktree is an attribute of the session (F9) — the group
                    // above is the repo. Absent when this is the main checkout.
                    let wt = session.worktree.clone().unwrap_or_else(|| "—".to_string());
                    let wt_color = if session.worktree.is_some() {
                        muted
                    } else {
                        faint
                    };

                    // Host (F5): this machine says so by staying quiet.
                    let host_label = host
                        .clone()
                        .unwrap_or_else(|| crate::t!("cockpit-table-host-local").to_string());

                    let model = zaplex_cockpit::model_effort_label(
                        &session.model,
                        session.effort.as_deref(),
                    );

                    let today = match today_cost {
                        Some(c) => Self::cell(format_cost(*c), main, true, appearance),
                        // No figure rather than $0.00 — a zero would read as one.
                        None => Self::cell("—".to_string(), faint, true, appearance),
                    };

                    let state_cell = Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(6.0)
                        .with_child(glyph_cell(
                            session_glyph(session.state),
                            status_dot_coloru(session.state, appearance),
                            appearance,
                        ))
                        .with_child(Self::text(
                            zaplex_cockpit::state_word(session.state).to_string(),
                            family,
                            body,
                            if session.state == SessionState::Waiting {
                                main
                            } else {
                                muted
                            },
                        ))
                        .with_main_axis_size(MainAxisSize::Min)
                        .finish();

                    vec![
                        sess_cell,
                        Self::cell(wt, wt_color, false, appearance),
                        Self::cell(host_label, muted, false, appearance),
                        Self::cell(model, muted, false, appearance),
                        {
                            let attrs = zaplex_cockpit::session_attrs(
                                &session.model,
                                session.effort.as_deref(),
                                session.ctx_tokens,
                                session.state,
                            );
                            match attrs.ctx_pct {
                                Some(pct) => {
                                    ctx_pct_element(pct, attrs.ctx_fill, false, appearance)
                                }
                                None => Self::cell("—".to_string(), faint, true, appearance),
                            }
                        },
                        today,
                        state_cell,
                        Self::cell(
                            format_relative(session.last_activity, chrono::Utc::now()),
                            faint,
                            true,
                            appearance,
                        ),
                        // The ⋯ is its own cell, outside the row's click target:
                        // opening the drive and opening the session are different
                        // intents, and one must never fire the other.
                        match dots_states.get(&rk).cloned() {
                            Some(state) => {
                                let key = rk.clone();
                                Hoverable::new(state, move |mouse| {
                                    let c = if mouse.is_hovered() { main } else { faint };
                                    Self::text("⋯".to_string(), family, body, c)
                                })
                                .with_cursor(Cursor::PointingHand)
                                .on_click(move |ctx, _, position| {
                                    ctx.dispatch_typed_action(CockpitPaneAction::OpenRowMenu {
                                        row_key: key.clone(),
                                        position,
                                    })
                                })
                                .finish()
                            }
                            None => Empty::new().finish(),
                        },
                    ]
                }
            }
        });

        let table = Table::new(self.table_state.clone(), 0.0, 0.0)
            .with_headers(vec![
                header(
                    "session",
                    crate::t!("cockpit-table-col-session").to_string(),
                    SortColumn::Session,
                    false,
                )
                .with_width(TableColumnWidth::Flex(2.2)),
                header(
                    "worktree",
                    crate::t!("cockpit-table-col-worktree").to_string(),
                    SortColumn::Worktree,
                    false,
                )
                .with_width(TableColumnWidth::Flex(1.2)),
                header(
                    "host",
                    crate::t!("cockpit-table-col-host").to_string(),
                    SortColumn::Host,
                    false,
                )
                .with_width(TableColumnWidth::Flex(0.9)),
                // 1.4: the model cell carries "family·effort" strings that a
                // 1.0 share truncated mid-word ("codex-auto-revie…") with no
                // way to read the rest. The existing tooltip helpers are
                // button-bound (`icon_verb_button_tooltip`), there is no plain
                // cell-tooltip wrapper — so the column gets the room instead
                // (audit P1.3).
                header(
                    "model",
                    crate::t!("cockpit-table-col-model").to_string(),
                    SortColumn::Model,
                    false,
                )
                .with_width(TableColumnWidth::Flex(1.4)),
                header(
                    "context",
                    crate::t!("cockpit-table-col-context").to_string(),
                    SortColumn::Context,
                    true,
                )
                .with_width(TableColumnWidth::Flex(1.0)),
                header(
                    "today",
                    crate::t!("cockpit-pane-col-today").to_string(),
                    SortColumn::Today,
                    true,
                )
                .with_width(TableColumnWidth::Flex(0.8)),
                header(
                    "status",
                    crate::t!("cockpit-table-col-status").to_string(),
                    SortColumn::Status,
                    false,
                )
                .with_width(TableColumnWidth::Flex(1.0)),
                header(
                    "last",
                    crate::t!("cockpit-table-col-last").to_string(),
                    SortColumn::Last,
                    true,
                )
                .with_width(TableColumnWidth::Flex(0.8)),
                // The ⋯ column: no label — a header over a menu affordance would
                // be naming the furniture.
                TableHeader::new(Empty::new().finish()).with_width(TableColumnWidth::Fixed(24.0)),
            ])
            .with_row_count(rows_len)
            .with_config(TableConfig {
                border_width: 1.0,
                border_color: theme.split_pane_border_color().into_solid(),
                outer_border: false,
                column_dividers: false,
                row_dividers: true,
                cell_padding: 6.0,
                // No banding, no header wash: the row's own state carries the
                // eye (spec §0), and a striped table would add a rhythm that
                // means nothing.
                header_background: ColorU::transparent_black(),
                row_background: RowBackground {
                    primary: ColorU::transparent_black(),
                    alternating: None,
                },
                fixed_header: true,
                vertical_sizing: TableVerticalSizing::Viewported,
                measure_body_cells_for_intrinsic_widths: false,
            })
            .finish();

        ConstrainedBox::new(Clipped::new(table).finish())
            .with_height(session_table_viewport_height(rows_len))
            .finish()
    }

    /// Persist a typed alias and close the editor (A1).
    ///
    /// What you typed is what you get; blank clears the alias, since an account
    /// is never named "".
    ///
    /// There was a cleverness here — "an alias equal to the discovered label is
    /// stored as no alias" — and it was worse than useless. The snapshot's label
    /// has already had the override applied by the time anyone can read it
    /// (`overrides.apply` runs before the snapshot exists), so the comparison was
    /// against the *current alias*, not the discovered name. Opening the editor
    /// on an existing alias and pressing Enter without touching it silently
    /// deleted that alias. Deriving the discovered label would need the account
    /// to carry it separately — a data change to buy a nicety nobody asked for.
    /// Storing what the user typed is simpler and cannot be wrong.
    fn commit_alias(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        let Some(key) = self.account_key.clone() else {
            self.alias_editor = None;
            ctx.notify();
            return;
        };
        let text = text.trim().to_string();
        let alias = (!text.is_empty()).then_some(text);

        match CockpitModel::as_ref(ctx).set_alias(&key, alias.as_deref()) {
            Ok(()) => {
                // The file is watched: the snapshot reloads and the new name
                // reaches the title, the card and the sidebar on its own.
                self.alias_editor = None;
                self.alias_persistence_error = None;
            }
            Err(e) => {
                log::warn!("cockpit: could not write alias for {key}: {e}");
                self.alias_persistence_error = Some(crate::t!(
                    "cockpit-account-alias-write-error",
                    error = e.to_string()
                ));
            }
        }
        ctx.notify();
    }

    /// **P2** — the account's own detail card: who it is, how close to full, and
    /// what it has spent.
    ///
    /// Distinct from `render_card`, which is one row in the fleet list: this is
    /// the head of *that account's* pane, so it can afford the identity in full
    /// (mail/org belong here — the sidebar deliberately drops them) and the three
    /// windows side by side instead of stacked.
    fn render_account_detail(
        &self,
        acct: &AccountUsage,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let heading = appearance.ui_font_heading_3();
        let bg = theme.background();
        let main = theme.main_text_color(bg).into_solid();
        let muted = theme.sub_text_color(bg).into_solid();
        let faint = theme.sub_text_color(bg).with_opacity(55).into_solid();
        let now = chrono::Utc::now();

        // Identity: provider tile, display name, then the quieter facts.
        // The tile is a glyph-scale colour swatch: its radius follows its own
        // size (10 px → 3), not the control scale — CONTROL_RADIUS on a mark
        // this small would read as a circle.
        let tile = ConstrainedBox::new(
            Rect::new()
                .with_background_color(provider_color_on(acct.account.provider, bg.into_solid()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.0)))
                .finish(),
        )
        .with_width(10.0)
        .with_height(10.0)
        .finish();

        // `label` already carries the user's alias — the overrides layer replaced
        // it before this snapshot existed. Reading the alias again here would be
        // a second source for one fact.
        // The name, or the box you rename it in. One `Option`, so there is no
        // second flag to disagree with it about whether we are editing.
        let name_el: Box<dyn Element> = match &self.alias_editor {
            Some(editor) => ConstrainedBox::new(
                appearance
                    .ui_builder()
                    .text_input(editor.clone())
                    .with_style(UiComponentStyles {
                        padding: Some(Coords {
                            left: 5.0,
                            right: 5.0,
                            top: 1.0,
                            bottom: 1.0,
                        }),
                        background: Some(theme.surface_2().into()),
                        border_color: Some(theme.accent().into()),
                        border_width: Some(1.0),
                        border_radius: Some(CornerRadius::with_all(Radius::Pixels(CONTROL_RADIUS))),
                        font_size: Some(heading),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            // Fixed: an EditorView panics when measured against an infinite
            // width, which a flexible row child gets during the intrinsic pass.
            .with_width(ALIAS_EDITOR_WIDTH)
            .finish(),
            None => Self::text(acct.account.label.clone(), family, heading, main),
        };
        let mut ident = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.0)
            .with_child(tile)
            .with_child(name_el);
        if let Some(plan) = &acct.account.plan_tier {
            ident = ident.with_child(Self::text(plan.clone(), family, body, faint));
        }
        // ⋯ — rename this account (A1). The alias is a label override in
        // instances.json, the one place overrides live; there is no second store.
        if self.alias_editor.is_none() {
            ident = ident.with_child(Shrinkable::new(1.0, Empty::new().finish()).finish());
            ident = ident.with_child(icon_verb_button_tooltip(
                self.alias_dots_state.clone(),
                icons::Icon::DotsHorizontal,
                theme.sub_text_color(bg),
                theme.accent(),
                crate::t!("cockpit-account-rename"),
                appearance,
                CockpitPaneAction::StartAliasEdit,
            ));
        }

        // Mail / org: allowed here, unlike the sidebar (spec v3 §4.2). Absent
        // rather than empty when the provider never told us.
        let mut sub_parts: Vec<String> = Vec::new();
        if let Some(email) = &acct.account.email {
            sub_parts.push(email.clone());
        }
        if let Some(org) = &acct.account.org {
            sub_parts.push(org.clone());
        }

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(10.0)
            .with_child(ident.with_main_axis_size(MainAxisSize::Max).finish());
        if !sub_parts.is_empty() {
            col = col.with_child(Self::text(sub_parts.join(" · "), family, body, muted));
        }
        if let Some(error) = self.alias_persistence_error.as_ref() {
            col = col.with_child(Self::text(
                error.clone(),
                family,
                body,
                theme.ui_error_color(),
            ));
        }

        // The two meters. `heat_bar` carries the one utilisation rule (grey below
        // the threshold, true red at or above it) — asked for rather than redone.
        // Meter labels are the short vocabulary („5h"/„Wo"), same as the fleet
        // card — the long column titles clipped inside the label cell (audit
        // P0.3); they belong to the figures matrix below.
        col = col.with_child(self.heat_bar(
            &crate::t!("cockpit-meter-5h"),
            acct.heat,
            acct.provenance,
            appearance,
        ));
        col = col.with_child(self.heat_bar(
            &crate::t!("cockpit-meter-week"),
            acct.heat_week,
            acct.provenance,
            appearance,
        ));
        // ONE reset line under both meters, in the fleet card's format
        // (`5h ↻ … · Wo ↻ …`) — bare times with no label read as debug output
        // (audit P0.4).
        if let Some(reset_line) = Self::reset_line(acct, now) {
            col = col.with_child(Self::text(reset_line, family, body, faint));
        }

        // Three windows × ($ / tokens). "Today" is the LOCAL day (F2).
        let figure = |label: String, totals: &WindowTotals| -> Box<dyn Element> {
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(2.0)
                .with_child(Self::text(label, family, body, muted))
                .with_child(Self::text(
                    format_cost(totals.cost_usd),
                    family,
                    heading,
                    main,
                ))
                .with_child(Self::text(format_tokens(totals.total), family, body, faint))
                .finish()
        };
        let figures = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(24.0)
            .with_child(figure(
                crate::t!("cockpit-pane-col-today").to_string(),
                &acct.today,
            ))
            .with_child(figure(
                crate::t!("cockpit-pane-col-5h").to_string(),
                &acct.block5h,
            ))
            .with_child(figure(
                crate::t!("cockpit-pane-col-week").to_string(),
                &acct.week,
            ))
            .finish();
        col = col.with_child(figures);

        // A bounded card (hairline zone-card, §2.1), not print on the pane
        // background: the edge is what keeps the meters from visually running
        // off into empty pane space (RC acceptance, 2026-07-17).
        zone_card(col.finish(), appearance)
            .with_uniform_padding(CARD_PADDING)
            .with_margin_bottom(CARD_SPACING * 2.0)
            .finish()
    }

    fn render_card(
        &self,
        acct: &AccountUsage,
        override_color: Option<ColorU>,
        is_selected: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let heading = appearance.ui_font_heading_3();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let accent = theme.accent().into_solid();
        let now = chrono::Utc::now();

        // Account activity glyph (WORKING/LIVE/OFFLINE), derived by the spine.
        // The working hue is contrast-picked like every other themed mark; the
        // dark-palette constant used here would sink on a light theme.
        let (status_glyph, status_color) = match acct.status {
            AccountStatus::Working => (
                "●",
                heat_coloru_on(HeatLevel::Ok, theme.background().into_solid()),
            ),
            AccountStatus::Live => ("◐", muted),
            AccountStatus::Offline => ("○", muted),
        };
        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.0);
        // User override color (instances.json): a small leading swatch so a
        // recolored account is recognizable at a glance.
        if let Some(color) = override_color {
            header = header.with_child(Self::text("▉".to_string(), family, body, color));
        }
        header = header
            .with_child(Self::text(
                status_glyph.to_string(),
                family,
                body,
                status_color,
            ))
            .with_child(
                Shrinkable::new(
                    1.0,
                    Self::text(acct.account.label.clone(), family, heading, main),
                )
                .finish(),
            );
        if let Some(plan) = &acct.account.plan_tier {
            header = header.with_child(
                Container::new(Self::text(plan.clone(), family, body, accent))
                    .with_padding_left(8.0)
                    .with_padding_right(8.0)
                    .with_padding_top(2.0)
                    .with_padding_bottom(2.0)
                    .with_background(internal_colors::fg_overlay_1(theme))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CONTROL_RADIUS)))
                    .finish(),
            );
        }

        // The 5h-block heat drives account routing later; the week heat shows
        // the slower budget. Week heat = week.work / budget via AccountUsage —
        // the spine exposes `heat` (5h) and `heat_week`.
        let matrix = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(16.0)
            .with_child(self.matrix_cell(
                &crate::t!("cockpit-pane-col-today"),
                &acct.today,
                appearance,
            ))
            .with_child(self.matrix_cell(
                &crate::t!("cockpit-pane-col-5h"),
                &acct.block5h,
                appearance,
            ))
            .with_child(self.matrix_cell(
                &crate::t!("cockpit-pane-col-week"),
                &acct.week,
                appearance,
            ))
            .finish();

        let reset_line = Self::reset_line(acct, now);

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(CARD_SPACING)
            .with_child(header.finish())
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
        // 7-day per-model sublimits (Max plans) when the endpoint reports them —
        // often the binding constraint, so the dashboard shows them explicitly
        // next to 5h/wk (Codex #6).
        if let Some(opus) = acct.heat_opus {
            col = col.with_child(self.heat_bar("opus", opus, acct.provenance, appearance));
        }
        if let Some(sonnet) = acct.heat_sonnet {
            col = col.with_child(self.heat_bar("sonnet", sonnet, acct.provenance, appearance));
        }
        col = col.with_child(matrix);
        // Live sessions (C3a), waiting-first (the spine pre-sorts): the
        // dashboard's job is surfacing what needs YOU.
        for session in acct.sessions.iter().take(4) {
            // Status dots: the SHAPE carries the state, colour only reinforces it
            // (spec v3 §1.1). This used to hard-code its own glyphs and rendered
            // Waiting and Active as the SAME "●", telling them apart by amber vs
            // green alone — indistinguishable with red-green colour blindness, and
            // it silently drifted from the shared vocabulary while its comment
            // claimed to follow it. Both now come from the single source of truth.
            let glyph = session_glyph(session.state);
            let color = status_dot_coloru(session.state, appearance);
            let dir = std::path::Path::new(&session.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| session.cwd.clone());
            let label = if session.name.is_empty() {
                dir
            } else {
                format!("{} — {dir}", session.name)
            };
            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.0)
                .with_child(glyph_cell(glyph, color, appearance))
                .with_child(Shrinkable::new(1.0, Self::text(label, family, body, main)).finish());
            // Model family + context-window fill of the latest turn (claudeplex
            // parity): family in accent, context as a percent of the model's
            // real window, colored by how full it is.
            let fam = zaplex_cockpit::model_family(&session.model);
            if !fam.is_empty() {
                row = row.with_child(Self::text(fam.to_string(), family, body, accent));
            }
            if session.ctx_tokens > 0 {
                let frac = zaplex_cockpit::context_fill(&session.model, session.ctx_tokens);
                let pct = (frac * 100.0).round() as u32;
                row = row.with_child(ctx_pct_element(pct, frac, false, appearance));
            }
            // Adopt verb: "open = focus" — resume this idle session in place.
            if let Some(action) = self.session_adopt_action(acct, session, appearance) {
                row = row.with_child(action);
            }
            // Open the conversation transcript (claudeplex-parity read view).
            if let Some(action) = self.session_transcript_action(acct, session, appearance) {
                row = row.with_child(action);
            }
            // Fork verbs (fork/worktree design §2): branch a copy of the
            // conversation — plain, or into an isolated sibling worktree.
            for into_worktree in [false, true] {
                if let Some(action) =
                    self.session_fork_action(acct, session, into_worktree, appearance)
                {
                    row = row.with_child(action);
                }
            }
            col = col.with_child(row.with_main_axis_size(MainAxisSize::Max).finish());
        }
        if acct.sessions.len() > 4 {
            col = col.with_child(Self::text(
                format!("… {} more", acct.sessions.len() - 4),
                family,
                body,
                muted,
            ));
        }
        if let Some(reset_line) = reset_line {
            col = col.with_child(Self::text(reset_line, family, body, muted));
        }

        // A selected account (chosen in the sidebar, WS4 S5) is the detail focus:
        // a stronger surface marks it so the user's eye lands on the card they
        // clicked. Background only — no border, so nothing reflows.
        let card_bg = if is_selected {
            internal_colors::fg_overlay_2(theme)
        } else {
            internal_colors::fg_overlay_1(theme)
        };
        Container::new(col.finish())
            .with_uniform_padding(CARD_PADDING)
            .with_margin_bottom(CARD_SPACING)
            .with_background(card_bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(BLOCK_RADIUS)))
            .finish()
    }

    fn render_aggregate(
        &self,
        accounts: &[AccountUsage],
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let heading = appearance.ui_font_heading_3();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        let cost_today: f64 = accounts.iter().map(|a| a.today.cost_usd).sum();
        let cost_5h: f64 = accounts.iter().map(|a| a.block5h.cost_usd).sum();
        let cost_wk: f64 = accounts.iter().map(|a| a.week.cost_usd).sum();
        let waiting: usize = accounts
            .iter()
            .flat_map(|a| &a.sessions)
            .filter(|s| s.state == SessionState::Waiting)
            .count();
        // Working = the agent is busy (Active) or mid tool-run / live job
        // (Monitor) — hands off. Surfaced next to the waiting count so the
        // header answers "how much is running vs waiting on me" at a glance.
        let working: usize = accounts
            .iter()
            .flat_map(|a| &a.sessions)
            .filter(|s| matches!(s.state, SessionState::Active | SessionState::Monitor))
            .count();

        let mut summary = format!(
            "{} account{}",
            accounts.len(),
            if accounts.len() == 1 { "" } else { "s" }
        );
        if working > 0 {
            summary.push_str(&format!(" · ▶ {working} working"));
        }
        if waiting > 0 {
            summary.push_str(&format!(" · ✋ {waiting} waiting on you"));
        }

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(Self::text(
                summary,
                family,
                heading,
                if waiting > 0 {
                    attention_coloru(appearance)
                } else {
                    main
                },
            ))
            .with_child(Self::text(
                format!(
                    "today {} · 5h {} · wk {}",
                    format_cost(cost_today),
                    format_cost(cost_5h),
                    format_cost(cost_wk)
                ),
                family,
                body,
                muted,
            ))
            .finish()
    }
}

impl View for CockpitPaneView {
    fn ui_name() -> &'static str {
        "CockpitPaneView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        // A disabled cockpit clears its snapshot to empty; the placeholder must say
        // "disabled", not "no accounts".
        let enabled = *crate::cockpit::settings::CockpitSettings::as_ref(app).enabled;

        let snapshot = CockpitModel::as_ref(app).snapshot().clone();

        // An account pane shows THAT account: its detail card, then its sessions
        // from every host. The fleet dashboard (no key) keeps the overview it
        // always had — the two are different questions, and one view answering
        // both is what P1 took apart.
        if let Some(key) = self.account_key.as_deref() {
            // A disabled cockpit cleared its snapshot, so this account is not found —
            // say "turned off", not "account gone" (it was not removed, the cockpit is
            // just off).
            if !enabled {
                return Container::new(self.render_scan_placeholder(
                    &snapshot.health,
                    enabled,
                    appearance,
                ))
                .with_background(theme.background())
                .with_uniform_padding(PANE_PADDING)
                .finish();
            }
            let model = CockpitModel::as_ref(app);
            let tree = model.inventory().clone();
            let content: Box<dyn Element> =
                match snapshot.accounts.iter().find(|a| a.account.key == key) {
                    Some(acct) => {
                        let scroll = {
                            // Two zone-cards (the sidebar's §2.1 vocabulary): the
                            // account's identity + meters, then its sessions (toolbar +
                            // table). Bounded surfaces with a hairline edge instead of
                            // print on the raw pane background — without them, stacked
                            // account panes read as one continuous debug dump (RC
                            // acceptance, 2026-07-17).
                            let sessions_zone = zone_card(
                                Flex::column()
                                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                                    .with_main_axis_size(MainAxisSize::Min)
                                    .with_child(
                                        Container::new(
                                            self.render_table_toolbar(acct, &tree, appearance),
                                        )
                                        .with_margin_bottom(CARD_SPACING)
                                        .finish(),
                                    )
                                    .with_child(
                                        self.render_sessions_table(acct, &tree, app, appearance),
                                    )
                                    .finish(),
                                appearance,
                            )
                            .with_uniform_padding(CARD_PADDING)
                            .finish();
                            let mut col = Flex::column()
                                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                                .with_main_axis_size(MainAxisSize::Min)
                                .with_child(self.render_account_detail(acct, appearance))
                                .with_child(sessions_zone);
                            // C3b: the ~ marker needs its explanation wherever it is
                            // visible — this pane showed estimates unexplained while
                            // only the fleet dashboard carried the legend (audit P0.6).
                            if acct.provenance == UsageProvenance::Estimate {
                                col = col.with_child(
                                    Container::new(Self::text(
                                        crate::t!("cockpit-pane-provenance-legend"),
                                        family,
                                        body,
                                        muted,
                                    ))
                                    .with_margin_top(CARD_SPACING)
                                    .finish(),
                                );
                            }
                            // The pane padding sits INSIDE the scrollable, so the ⋯
                            // drive's overlay Stack keeps its geometry (its anchor maths
                            // broke once already; don't move its parent).
                            ClippedScrollable::vertical(
                                self.scroll_state.clone(),
                                Container::new(col.finish())
                                    .with_uniform_padding(PANE_PADDING)
                                    .finish(),
                                ScrollbarWidth::Auto,
                                theme.disabled_text_color(theme.background()).into(),
                                theme.main_text_color(theme.background()).into(),
                                ElementFill::None,
                            )
                            .with_overlayed_scrollbar()
                            .finish()
                        };
                        // The ⋯ drive floats over the pane, anchored where it was
                        // clicked, and dismisses on any click outside itself.
                        match self.render_row_menu(acct, &tree, app, appearance) {
                            Some(menu) => {
                                let position = self
                                    .row_menu
                                    .as_ref()
                                    .map(|m| m.position)
                                    .unwrap_or_default();
                                let mut stack = Stack::new();
                                stack.add_child(scroll);
                                stack.add_positioned_overlay_child(
                                    menu,
                                    OffsetPositioning::offset_from_parent(
                                        position,
                                        ParentOffsetBounds::ParentByPosition,
                                        ParentAnchor::TopLeft,
                                        ChildAnchor::TopLeft,
                                    ),
                                );
                                stack.finish()
                            }
                            None => scroll,
                        }
                    }
                    // Signed out, config dir gone — say so instead of an empty pane
                    // that looks like a load that never finished.
                    None => Container::new(Self::text(
                        crate::t!("cockpit-account-gone"),
                        family,
                        body,
                        muted,
                    ))
                    .with_uniform_padding(PANE_PADDING)
                    .finish(),
                };
            // Same opaque pane background as the fleet branch below — an
            // account pane left it unset, so its content sat directly on
            // whatever was behind the pane (part of the "debug view" look).
            return Container::new(content)
                .with_background(theme.background())
                .finish();
        }

        let content: Box<dyn Element> = if snapshot.accounts.is_empty() {
            self.render_scan_placeholder(&snapshot.health, enabled, appearance)
        } else {
            let mut col = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min);
            // A degraded scan with accounts present: the list may be missing others
            // (e.g. a broken Codex sign-in). Warn above the numbers; the empty case
            // shows this in its own placeholder instead.
            if matches!(snapshot.health, zaplex_cockpit::ScanHealth::Degraded(_)) {
                col = col.with_child(
                    Container::new(self.render_scan_placeholder(
                        &snapshot.health,
                        enabled,
                        appearance,
                    ))
                    .with_margin_bottom(CARD_SPACING * 2.0)
                    .finish(),
                );
            }
            col = col.with_child(
                Container::new(self.render_aggregate(&snapshot.accounts, appearance))
                    .with_margin_bottom(CARD_SPACING * 2.0)
                    .finish(),
            );
            // No Conductor tree here any more (P6). The sidebar carries it, and
            // carrying it twice meant maintaining two renderings of one thing —
            // which is how the two drifted: this one still mapped session state
            // to colour by hand long after the sidebar stopped (E1).
            //
            // Nothing was dropped with it. Every verb it held has a home: the
            // ⋯ drive on each table row, `workspace:stop_all_agents` for the
            // fleet-wide stop, the group header's "+" for a project-scoped
            // launch, and the launch menu's per-host entries for a host-scoped
            // one. That check is mechanical, not a claim — see spec §4.4.
            //
            // The pane is now the fleet's *numbers* (aggregate + one card per
            // account); the tree is the sidebar's job, and each account's
            // sessions are its own pane's table.
            let selected = CockpitModel::as_ref(app)
                .selected_account()
                .map(str::to_string);
            for acct in &snapshot.accounts {
                // Per-account override color (instances.json), resolved from the
                // model and parsed from its hex string.
                let override_color = CockpitModel::as_ref(app)
                    .override_color(&acct.account.key)
                    .and_then(parse_hex_color);
                let is_selected = selected.as_deref() == Some(acct.account.key.as_str());
                col =
                    col.with_child(self.render_card(acct, override_color, is_selected, appearance));
            }
            // C3b: explain the ~ marker whenever any bar shows an estimate
            // (real numbers stay unmarked — no chrome for the good case).
            if snapshot
                .accounts
                .iter()
                .any(|a| a.provenance == UsageProvenance::Estimate)
            {
                col = col.with_child(Self::text(
                    crate::t!("cockpit-pane-provenance-legend"),
                    family,
                    body,
                    muted,
                ));
            }
            ClippedScrollable::vertical(
                self.scroll_state.clone(),
                col.finish(),
                ScrollbarWidth::Auto,
                theme.disabled_text_color(theme.background()).into(),
                theme.main_text_color(theme.background()).into(),
                ElementFill::None,
            )
            .with_overlayed_scrollbar()
            .finish()
        };

        Container::new(content)
            .with_uniform_padding(PANE_PADDING)
            .with_background(theme.background())
            .finish()
    }
}

impl Entity for CockpitPaneView {
    type Event = PaneEvent;
}

impl TypedActionView for CockpitPaneView {
    type Action = CockpitPaneAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CockpitPaneAction::ToggleTableGroup(root) => {
                // Absent = expanded (groups default open); the first click folds.
                let cur = self
                    .collapsed_table_groups
                    .get(root)
                    .copied()
                    .unwrap_or(false);
                self.collapsed_table_groups.insert(root.clone(), !cur);
                ctx.notify();
            }
            CockpitPaneAction::SetSessionFilter(filter) => {
                self.session_filter = *filter;
                ctx.notify();
            }
            CockpitPaneAction::StartAliasEdit => {
                let Some(key) = self.account_key.clone() else {
                    return;
                };
                // Seed with the name as shown — which is the alias if one is set,
                // and the discovered label otherwise. Editing starts from what the
                // user is looking at, not from an empty box that discards it.
                let current = Self::pane_title(Some(&key), ctx);
                self.alias_persistence_error = None;
                let editor = ctx.add_typed_action_view(move |ctx| {
                    let appearance = Appearance::as_ref(ctx);
                    let theme = appearance.theme();
                    let mut e = EditorView::single_line(
                        SingleLineEditorOptions {
                            is_password: false,
                            text: TextOptions {
                                font_size_override: Some(appearance.ui_font_heading_3()),
                                font_family_override: Some(appearance.ui_font_family()),
                                text_colors_override: Some(TextColors {
                                    default_color: theme.active_ui_text_color(),
                                    disabled_color: theme.disabled_ui_text_color(),
                                    hint_color: theme.disabled_ui_text_color(),
                                }),
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        ctx,
                    );
                    e.set_buffer_text(&current, ctx);
                    e
                });
                ctx.subscribe_to_view(&editor, |me: &mut Self, editor, event, ctx| match event {
                    // Enter and blur both commit: clicking away from a rename you
                    // typed should keep it, not throw it out.
                    EditorEvent::Enter | EditorEvent::Blurred => {
                        let text = editor.as_ref(ctx).buffer_text(ctx);
                        me.commit_alias(text, ctx);
                    }
                    EditorEvent::Escape => {
                        me.alias_editor = None;
                        me.alias_persistence_error = None;
                        ctx.notify();
                    }
                    _ => {}
                });
                ctx.focus(&editor);
                self.alias_editor = Some(editor);
                ctx.notify();
            }
            CockpitPaneAction::OpenRowMenu { row_key, position } => {
                self.row_menu = Some(RowMenu {
                    row_key: row_key.clone(),
                    position: *position,
                });
                // Seed this row's item handles NOW. They are seeded per open row
                // (all rows × ten items would be thousands of handles), and the
                // periodic sync only runs on a cockpit update — so without this
                // the first open renders an empty box: every `menu_item` finds no
                // handle and returns `None`. State set, handles missing, UI blank
                // — the same shape as the spawn card's stale-render bug.
                self.sync_table_states(ctx);
                ctx.notify();
            }
            CockpitPaneAction::CloseRowMenu => {
                self.row_menu = None;
                ctx.notify();
            }
            CockpitPaneAction::SortBy(column) => {
                self.sort = if self.sort.column == *column {
                    // Same column again: flip. That is what a second click means
                    // everywhere else, and a table is not the place to be novel.
                    Sort {
                        column: *column,
                        ascending: !self.sort.ascending,
                    }
                } else {
                    Sort {
                        column: *column,
                        ascending: column.default_ascending(),
                    }
                };
                ctx.notify();
            }
            CockpitPaneAction::ToggleHost(host_ident_key) => {
                // Flip relative to the *effective* state (which may be the
                // inverse-complexity auto decision), so one click always does the
                // visible thing regardless of whether an override exists yet.
                // `host_ident_key` is the stable host identity (`host_ident`),
                // not the display label, so we resolve the node by identity.
                let model = CockpitModel::as_ref(ctx);
                let fleet_large = fleet_is_large(model.inventory());
                let eff = model
                    .inventory()
                    .hosts
                    .iter()
                    .find(|h| &host_ident(h.is_local, h.host_id.as_deref()) == host_ident_key)
                    .map(|h| self.host_collapsed(h, fleet_large))
                    .unwrap_or(false);
                self.collapsed_hosts.insert(host_ident_key.clone(), !eff);
                ctx.notify();
            }
            CockpitPaneAction::ToggleProject(key) => {
                let eff = self.collapsed_projects.get(key).copied().unwrap_or(false);
                self.collapsed_projects.insert(key.clone(), !eff);
                ctx.notify();
            }
            CockpitPaneAction::MarkReviewed(session_id) => {
                let id = session_id.clone();
                crate::cockpit::reviewed::ReviewedStore::handle(ctx).update(ctx, |store, ctx| {
                    store.toggle(&id, ctx);
                });
                ctx.notify();
            }
            CockpitPaneAction::Rescan => {
                CockpitModel::handle(ctx).update(ctx, |model, ctx| model.rescan(ctx));
            }
        }
    }
}

impl BackingView for CockpitPaneView {
    type PaneHeaderOverflowMenuAction = ();
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
        // The cockpit pane exposes no overflow-menu actions
        // (`PaneHeaderOverflowMenuAction = ()`), so this handler is never
        // meaningfully dispatched. A no-op — never a panic — is the safe floor
        // if the framework ever routes a header overflow here (RC: no reachable
        // `unimplemented!()`).
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(PaneEvent::Close);
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> view::HeaderContent {
        // An account pane is titled by ITS account (alias→label, spec §4.1) —
        // two stacked account panes both reading "Cockpit" left the user unable
        // to tell whose numbers were whose (RC acceptance, 2026-07-17). The
        // fleet dashboard (no account key) keeps the generic title.
        view::HeaderContent::simple(Self::pane_title(self.account_key.as_deref(), app))
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

#[cfg(test)]
#[path = "pane_tests.rs"]
mod tests;
