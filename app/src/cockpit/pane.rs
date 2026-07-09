//! Cockpit **main-area pane** (C2b) — the roomy dashboard over the
//! `zaplex_cockpit` data spine: aggregate header, per-account cards with the
//! full cost/token matrix (today / 5h block / week), both heat bars and reset
//! timers. The compact glanceable variant is the sidebar (`CockpitPanel`);
//! this pane is a first-class zaplex pane (tab/split/promotable,
//! multi-instance), opened from the sidebar's expand action. See
//! docs/superpowers/specs/2026-07-01-cockpit-native-integration-design.md §3.3.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Empty, Fill as ElementFill, Flex, Hoverable, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, ParentElement, Radius, Rect, ScrollbarWidth, Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View, ViewContext,
};
use zaplex_cockpit::{
    fleet_is_large, fleet_session_count, format_cost, format_reset, format_tokens, heat_fill,
    heat_pct_label_with_provenance, host_auto_collapsed, host_ident, host_key, host_summary,
    session_glyph, AccountStatus, AccountUsage, FleetTree, HeatLevel, HostNode, ProjectNode,
    Provider, SessionSnapshot, SessionState, UsageProvenance, WindowTotals,
};

use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::style::{
    cluster_divider, ctx_pct_element, glyph_cell, heat_coloru, verb_button, verb_button_colored,
    VerbKind, INFO_VERBS_GAP, VERB_SPACING,
};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::terminal::cli_agent::CLIAgent;
use crate::WorkspaceAction;

const PANE_PADDING: f32 = 16.0;
const CARD_PADDING: f32 = 12.0;
const CARD_SPACING: f32 = 8.0;
const HEAT_BAR_WIDTH: f32 = 160.0;
const HEAT_BAR_HEIGHT: f32 = 8.0;
/// Fixed column width for the cost/token matrix cells.
const MATRIX_COL_WIDTH: f32 = 110.0;

/// Parse a `#RRGGBB` or `#RGB` hex string into an opaque color. Returns `None`
/// for anything malformed, so an invalid instances.json override color simply
/// yields no tint (never a panic, never a wrong color).
fn parse_hex_color(s: &str) -> Option<ColorU> {
    let hex = s.strip_prefix('#')?;
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
pub struct CockpitPaneView {
    scroll_state: ClippedScrollStateHandle,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    /// Hover state of each session row's "⑂ fork" action (key = session_id).
    /// Synced against the snapshot on every cockpit update so handles persist
    /// across renders (hover needs a stable handle).
    session_fork_states: HashMap<String, MouseStateHandle>,
    /// Hover state of each session row's "⑂ fork in worktree" action.
    session_forkwt_states: HashMap<String, MouseStateHandle>,
    /// Hover state of each session row's "▸ adopt" action (resume-in-place):
    /// pull an idle CLI session discovered by the cockpit into a live pane.
    session_adopt_states: HashMap<String, MouseStateHandle>,
    /// Hover state of each session row's "◇ log" action (open transcript).
    session_transcript_states: HashMap<String, MouseStateHandle>,
    /// Whether a session's cwd sits inside a git repo (key = session_id) —
    /// precomputed on cockpit updates so render never touches the filesystem.
    /// Non-repo cwds simply don't get the worktree action (design §3: toggle
    /// disabled, never a broken session).
    session_in_repo: HashMap<String, bool>,
    /// Hover/click state of each Conductor session row (key = `host_ident\0id`:
    /// the stable host identity — never the display label — since session ids are
    /// unique only within a host). Clicking the row attaches the agent (adopt in
    /// place for a local host). Synced against the unified inventory on every
    /// update.
    conductor_row_states: HashMap<String, MouseStateHandle>,
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
    /// keyed `"{verb}\0{host_ident}\0{id}"` (verb ∈
    /// review/approve/redirect/commit/pr). One combined map (rather than five)
    /// keeps the sync/retain cheap; hover still needs a stable handle per (verb,
    /// session) across renders.
    conductor_review_states: HashMap<String, MouseStateHandle>,
    /// Sessions the user marked reviewed (approve verb), keyed by
    /// `host_ident\0id`.
    /// A lightweight local marker — never mutates the agent — retained across
    /// the 45s reconcile like the hover-state maps, so approving doesn't flicker.
    reviewed_sessions: std::collections::HashSet<String>,
    /// Hover state of each Conductor session row's guardrail verbs (step 7):
    /// `⏸ stop` / `⨯ kill`, keyed `"{verb}\0{host_ident}\0{id}"` like the review-loop
    /// map. Unlike the review cluster, guardrails render on **every** live row
    /// — local and remote — since interrupt/kill are cross-host operations.
    conductor_guardrail_states: HashMap<String, MouseStateHandle>,
    /// Hover state of each Conductor session row's model-lever verbs (step 8):
    /// `⚙ /compact` · `⌫ /clear` · `⑂ fork` · `⑂ +worktree`, keyed
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
const REVIEW_VERB_KEYS: [&str; 5] = ["review", "approve", "redirect", "commit", "pr"];

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
    /// Fold/unfold a host node (key = stable host identity `host_ident`, not the
    /// display label — two remote daemons can share a label).
    ToggleHost(String),
    /// Fold/unfold a project node (key = `host_ident\0root`).
    ToggleProject(String),
    /// Mark a reviewed session as reviewed (approve verb, key = `host_ident\0id`).
    /// A local, non-mutating marker — dims the row's review affordance so the
    /// user's eye moves on; toggles off if approved twice.
    MarkReviewed(String),
}

impl CockpitPaneView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&Appearance::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&CockpitModel::handle(ctx), |me, _, event, ctx| {
            if matches!(event, CockpitEvent::Updated) {
                me.sync_session_action_states(ctx);
                ctx.notify();
            }
        });
        let pane_configuration =
            ctx.add_model(|_ctx| PaneConfiguration::new(crate::t!("cockpit-pane-title")));
        let mut me = Self {
            scroll_state: ClippedScrollStateHandle::default(),
            pane_configuration,
            focus_handle: None,
            session_fork_states: HashMap::new(),
            session_forkwt_states: HashMap::new(),
            session_adopt_states: HashMap::new(),
            session_transcript_states: HashMap::new(),
            session_in_repo: HashMap::new(),
            conductor_row_states: HashMap::new(),
            conductor_host_toggle_states: HashMap::new(),
            conductor_project_toggle_states: HashMap::new(),
            conductor_plus_states: HashMap::new(),
            collapsed_hosts: HashMap::new(),
            collapsed_projects: HashMap::new(),
            conductor_review_states: HashMap::new(),
            reviewed_sessions: std::collections::HashSet::new(),
            conductor_guardrail_states: HashMap::new(),
            conductor_lever_states: HashMap::new(),
            conductor_stop_all_state: MouseStateHandle::default(),
        };
        me.sync_session_action_states(ctx);
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
    fn sync_session_action_states(&mut self, ctx: &mut ViewContext<Self>) {
        let sessions: Vec<(String, String)> = CockpitModel::as_ref(ctx)
            .snapshot()
            .accounts
            .iter()
            .flat_map(|a| a.sessions.iter())
            .map(|s| (s.session_id.clone(), s.cwd.clone()))
            .collect();
        let live: std::collections::HashSet<&String> = sessions.iter().map(|(id, _)| id).collect();
        self.session_fork_states.retain(|id, _| live.contains(id));
        self.session_forkwt_states.retain(|id, _| live.contains(id));
        self.session_adopt_states.retain(|id, _| live.contains(id));
        self.session_transcript_states
            .retain(|id, _| live.contains(id));
        self.session_in_repo.retain(|id, _| live.contains(id));
        for (id, cwd) in sessions {
            // `.git` may be a dir (repo root) or a file (linked worktree) —
            // `exists()` covers both, so forking from inside a worktree chains.
            let in_repo = Path::new(&cwd).ancestors().any(|p| p.join(".git").exists());
            self.session_in_repo.insert(id.clone(), in_repo);
            self.session_fork_states.entry(id.clone()).or_default();
            self.session_forkwt_states.entry(id.clone()).or_default();
            self.session_adopt_states.entry(id.clone()).or_default();
            self.session_transcript_states.entry(id).or_default();
        }

        // Conductor maps: keyed off the unified cross-host inventory (which
        // includes remote sessions the local `accounts` list never sees), by the
        // **stable host identity** (`host_ident`) — `(host-identity, id)` for
        // sessions/projects, bare host identity for hosts — never the display
        // label (two remote daemons can share a label and would then alias each
        // other's UI state). Retain live keys, drop the rest, and prune stale
        // collapse overrides so a disconnected host doesn't leak.
        let inv = CockpitModel::as_ref(ctx).inventory();
        let live_rows: std::collections::HashSet<String> = inv
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
        // Review-loop maps: keyed `"{verb}\0{host_ident}\0{id}"`; the tail after the
        // first `\0` is the row's `host_key`. Retain live rows, drop the rest.
        self.conductor_review_states.retain(|k, _| {
            k.split_once('\u{0}')
                .map(|(_, rest)| live_rows.contains(rest))
                .unwrap_or(false)
        });
        self.reviewed_sessions.retain(|k| live_rows.contains(k));
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
                    let rk = host_key(host.is_local, host.host_id.as_deref(), &session.session_id);
                    self.conductor_row_states.entry(rk.clone()).or_default();
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

    /// One session-row fork action ("⑂ fork" / "⑂ +worktree"): muted, accent
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
        let agent = match acct.account.provider {
            Provider::Claude => CLIAgent::Claude,
            Provider::Codex => CLIAgent::Codex,
        };
        // Capability gate — agents without a fork mechanism get no surface.
        agent.fork_command(&session.session_id)?;
        if into_worktree
            && !self
                .session_in_repo
                .get(&session.session_id)
                .copied()
                .unwrap_or(false)
        {
            return None;
        }
        let states = if into_worktree {
            &self.session_forkwt_states
        } else {
            &self.session_fork_states
        };
        let state = states.get(&session.session_id).cloned()?;

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
            into_worktree,
        };
        Some(verb_button(
            state,
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
        let agent = match acct.account.provider {
            Provider::Claude => CLIAgent::Claude,
            Provider::Codex => CLIAgent::Codex,
        };
        // Capability gate — agents without a resume mechanism get no surface.
        agent.resume_command(&session.session_id)?;
        let state = self
            .session_adopt_states
            .get(&session.session_id)
            .cloned()?;

        let action = WorkspaceAction::AdoptAgentSession {
            agent,
            session_id: session.session_id.clone(),
            cwd: PathBuf::from(&session.cwd),
            // Non-default accounts resume on the same subscription.
            config_dir: (!acct.account.is_default).then(|| acct.account.config_dir.clone()),
        };
        Some(verb_button(
            state,
            crate::t!("cockpit-session-adopt").to_string(),
            VerbKind::Constructive,
            appearance,
            action,
        ))
    }

    /// One session-row "◇ log" action: open the session's conversation
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
        let state = self
            .session_transcript_states
            .get(&session.session_id)
            .cloned()?;

        let action = WorkspaceAction::ViewTranscript {
            session_id: session.session_id.clone(),
            config_dir: acct.account.config_dir.clone(),
            cwd: PathBuf::from(&session.cwd),
            // Follow live: the opened transcript refreshes on each cockpit
            // reconcile (claudeplex-desktop watch parity).
            watch: true,
        };
        Some(verb_button(
            state,
            crate::t!("cockpit-session-transcript").to_string(),
            VerbKind::Constructive,
            appearance,
            action,
        ))
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
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
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
                heat_coloru(level),
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
    fn render_card(
        &self,
        acct: &AccountUsage,
        override_color: Option<ColorU>,
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
        let (status_glyph, status_color) = match acct.status {
            AccountStatus::Working => ("●", heat_coloru(HeatLevel::Ok)),
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
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.0)))
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
            .with_child(self.heat_bar("5h", acct.heat, acct.provenance, appearance))
            .with_child(self.heat_bar("wk", acct.heat_week, acct.provenance, appearance));
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
            let (glyph, color) = match session.state {
                SessionState::Waiting => ("✋", heat_coloru(HeatLevel::Critical)),
                SessionState::Active => ("●", heat_coloru(HeatLevel::Ok)),
                SessionState::Monitor => ("◌", muted),
                SessionState::Idle => ("◦", muted),
            };
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

        Container::new(col.finish())
            .with_uniform_padding(CARD_PADDING)
            .with_margin_bottom(CARD_SPACING)
            .with_background(internal_colors::fg_overlay_1(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.0)))
            .finish()
    }

    /// The **Conductor**: the unified cross-host Agent-Inventory, rendered
    /// Host ▸ Project ▸ Session, waiting-first (the tree is pre-sorted).
    ///
    /// Drives off [`CockpitModel::inventory`] — the whole fleet (this machine
    /// plus every connected daemon), not a locally-rebuilt tree. The
    /// inverse-complexity law governs density: calm hosts fold to a one-line
    /// summary once the fleet is large (see [`Self::host_collapsed`]); a host
    /// that needs you stays open. One consistent glyph vocabulary throughout
    /// ([`session_glyph`]), with a `✋ N` needs-me badge bubbling up host→project.
    /// `None` when the fleet has no live sessions (no empty section — no noise).
    fn render_conductor(
        &self,
        tree: &FleetTree,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        if tree.hosts.is_empty() {
            return None;
        }
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let fleet_large = fleet_is_large(tree);

        // Header: title, and — when at least one agent is live anywhere in
        // the inventory — the fleet-wide "stop all" guardrail control (step
        // 7), pinned to the trailing edge like the host/project "+" buttons.
        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Self::text(
                crate::t!("cockpit-conductor-title").to_string(),
                family,
                body,
                muted,
            ))
            .with_child(Shrinkable::new(1.0, Empty::new().finish()).finish());
        if fleet_session_count(tree) > 0 {
            header = header.with_child(self.render_conductor_stop_all(appearance));
        }

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(CARD_SPACING)
            .with_child(header.finish());
        for host in &tree.hosts {
            // Locality comes from the inventory's explicit marker, never from a
            // label comparison (a remote label can collide with the local one).
            col = col.with_child(self.render_conductor_host(
                host,
                host.is_local,
                fleet_large,
                app,
                appearance,
            ));
        }
        Some(col.finish())
    }

    /// A single Conductor host node: a collapse header (chevron + label +
    /// `✋ N` badge + working/idle tallies), then — when expanded — its projects.
    /// Collapsed, it shows only the one-line [`host_summary`] (60 rows become 1).
    /// The contextual "+" on a Conductor host/project header: opens the
    /// Spawn-Karte pre-scoped to that host/project (design: a new agent pre-bound
    /// to where the user is looking). Muted, accent on hover. `None` when its
    /// hover-state handle isn't ready yet (first frame before reconcile).
    fn render_conductor_plus(
        &self,
        plus_key: &str,
        action: WorkspaceAction,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let state = self.conductor_plus_states.get(plus_key).cloned()?;
        Some(verb_button(
            state,
            "+",
            VerbKind::Constructive,
            appearance,
            action,
        ))
    }

    fn render_conductor_host(
        &self,
        host: &HostNode,
        is_local: bool,
        fleet_large: bool,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let collapsed = self.host_collapsed(host, fleet_large);
        // Stable host identity — keys every per-host map (collapse, hover, "+")
        // and the ToggleHost action, so a label collision never crosses hosts.
        let hident = host_ident(is_local, host.host_id.as_deref());

        let chevron = if collapsed { "▸" } else { "▾" };
        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(Self::text(chevron.to_string(), family, body, muted))
            .with_child(
                Shrinkable::new(1.0, Self::text(host.host.clone(), family, body, main)).finish(),
            );
        if host.needs_me > 0 {
            header = header.with_child(Self::text(
                format!("✋ {}", host.needs_me),
                family,
                body,
                heat_coloru(HeatLevel::Critical),
            ));
        }
        if !is_local {
            // A quiet "remote" marker so the user knows attach behaves differently.
            header = header.with_child(Self::text("· remote".to_string(), family, body, muted));
        }
        // The header content toggles collapse; the trailing "+" opens a
        // pre-scoped Spawn-Karte (its own click, kept outside the toggle area).
        let header_el: Box<dyn Element> =
            if let Some(state) = self.conductor_host_toggle_states.get(&hident).cloned() {
                // ToggleHost carries the stable host identity, not the label, so
                // the collapse override lands on the right host.
                let toggle_key = hident.clone();
                let inner = header.with_main_axis_size(MainAxisSize::Min).finish();
                Hoverable::new(state, move |_mouse| inner)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(CockpitPaneAction::ToggleHost(toggle_key.clone()))
                    })
                    .finish()
            } else {
                header.with_main_axis_size(MainAxisSize::Min).finish()
            };

        // A new agent scoped to this host (remote hosts pass their label; local
        // passes `None` so the card defaults to the local machine).
        let plus = self.render_conductor_plus(
            &format!("host:{hident}"),
            WorkspaceAction::OpenSpawnCard {
                // Carry the stable host id so a later same-named host scopes the
                // launch to the *right* remote node (label alone is ambiguous).
                host_id: (!is_local).then(|| host.host_id.clone()).flatten(),
                host: (!is_local).then(|| host.host.clone()),
                project: None,
            },
            appearance,
        );
        let mut header_bar = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(header_el)
            .with_child(Shrinkable::new(1.0, Empty::new().finish()).finish());
        if let Some(plus) = plus {
            header_bar = header_bar.with_child(plus);
        }

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            // A touch of air between rows — calm rhythm over density.
            .with_spacing(4.0)
            .with_child(header_bar.finish());

        if collapsed {
            // One calm line instead of the host's whole subtree.
            col = col.with_child(
                Container::new(Self::text(host_summary(host), family, body, muted))
                    .with_padding_left(16.0)
                    .finish(),
            );
        } else {
            for project in &host.projects {
                col = col.with_child(self.render_conductor_project(
                    &host.host,
                    host.host_id.as_deref(),
                    project,
                    is_local,
                    app,
                    appearance,
                ));
            }
        }
        col.finish()
    }

    /// A Conductor project node: a collapse header (`✋ N` badge + repo name +
    /// session count), then — when expanded — its session rows (waiting-first).
    fn render_conductor_project(
        &self,
        host_label: &str,
        host_id: Option<&str>,
        project: &ProjectNode,
        is_local: bool,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        // Project UI state (collapse, toggle hover, "+") keys on the stable host
        // identity + git root, never the display label.
        let key = host_key(is_local, host_id, &project.root);
        let collapsed = self.project_collapsed(is_local, host_id, &project.root);

        let chevron = if collapsed { "▸" } else { "▾" };
        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(Self::text(chevron.to_string(), family, body, muted));
        if project.needs_me > 0 {
            header = header.with_child(Self::text(
                format!("✋ {}", project.needs_me),
                family,
                body,
                heat_coloru(HeatLevel::Critical),
            ));
        }
        header = header
            .with_child(
                Shrinkable::new(1.0, Self::text(project.name.clone(), family, body, main)).finish(),
            )
            .with_child(Self::text(
                format!(
                    "{} session{}",
                    project.sessions.len(),
                    if project.sessions.len() == 1 { "" } else { "s" }
                ),
                family,
                body,
                muted,
            ));

        let header_el: Box<dyn Element> =
            if let Some(state) = self.conductor_project_toggle_states.get(&key).cloned() {
                let inner = header.with_main_axis_size(MainAxisSize::Min).finish();
                let key = key.clone();
                Hoverable::new(state, move |_mouse| inner)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(CockpitPaneAction::ToggleProject(key.clone()))
                    })
                    .finish()
            } else {
                header.with_main_axis_size(MainAxisSize::Min).finish()
            };

        // A new agent scoped to this project dir (and its host).
        let plus = self.render_conductor_plus(
            &format!("proj:{key}"),
            WorkspaceAction::OpenSpawnCard {
                // A project belongs to a host — carry that host's stable id so the
                // launch scopes to the right remote node even for same-named hosts.
                host_id: (!is_local).then(|| host_id.map(str::to_string)).flatten(),
                host: (!is_local).then(|| host_label.to_string()),
                project: Some(std::path::PathBuf::from(&project.root)),
            },
            appearance,
        );
        let mut header_bar = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(header_el)
            .with_child(Shrinkable::new(1.0, Empty::new().finish()).finish());
        if let Some(plus) = plus {
            header_bar = header_bar.with_child(plus);
        }

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            // Same row rhythm as the host level — one consistent cadence.
            .with_spacing(4.0)
            .with_child(
                Container::new(header_bar.finish())
                    .with_padding_left(16.0)
                    .finish(),
            );
        if !collapsed {
            for session in &project.sessions {
                col = col.with_child(
                    Container::new(self.render_conductor_session(
                        host_label, host_id, session, is_local, app, appearance,
                    ))
                    .with_padding_left(32.0)
                    .finish(),
                );
            }
        }
        col.finish()
    }

    /// One Conductor session row: `<glyph> <name — dir> <model> · <ctx%>`, using
    /// the shared glyph vocabulary and the model-family/colored-context% styling.
    /// The whole row is the **attach** affordance — clicking it adopts the agent
    /// in place (local or remote host) via [`WorkspaceAction::AttachFleetSession`],
    /// the same path the `w`-jump uses. Step 8's per-session levers (`/compact`,
    /// `/clear`, fork) hang as trailing children on this row.
    fn render_conductor_session(
        &self,
        host_label: &str,
        host_id: Option<&str>,
        session: &SessionSnapshot,
        is_local: bool,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let accent = theme.accent().into_solid();

        let (glyph, glyph_color) = match session.state {
            SessionState::Waiting => (
                session_glyph(session.state),
                heat_coloru(HeatLevel::Critical),
            ),
            SessionState::Active | SessionState::Monitor => {
                (session_glyph(session.state), heat_coloru(HeatLevel::Ok))
            }
            SessionState::Idle => (session_glyph(session.state), muted),
        };
        let dir = Path::new(&session.cwd)
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
            .with_child(glyph_cell(glyph, glyph_color, appearance))
            .with_child(Shrinkable::new(1.0, Self::text(label, family, body, main)).finish());
        // Always-visible model·effort·context attributes (step 8): the compact
        // "Opus·High" label in accent, then the colored context-fill %. Effort
        // comes from the snapshot; when the transcript didn't carry it, fall
        // back to the launch registry's best-known intent for this (agent, host,
        // cwd) — honest "unknown" (label omits the effort) when neither knows.
        let effort = crate::cockpit::session_effort(session, is_local, host_id);
        let attrs = zaplex_cockpit::session_attrs(
            &session.model,
            effort.as_deref(),
            session.ctx_tokens,
            session.state,
        );
        if !attrs.model_effort.is_empty() {
            row = row.with_child(Self::text(attrs.model_effort, family, body, accent));
        }
        if let Some(pct) = attrs.ctx_pct {
            row = row.with_child(ctx_pct_element(pct, attrs.ctx_fill, true, appearance));
        }
        let info = row.with_main_axis_size(MainAxisSize::Max).finish();

        // Attach on click of the info span — for BOTH local and remote sessions
        // now that remote in-place adopt is wired (`attach_fleet_session` resumes
        // a remote session on its host, the same path the `w`-jump uses). The
        // click target is the info span (not the whole row) so the trailing
        // review-loop (step 6) and guardrail (step 7) verbs keep their own click
        // targets alongside it.
        let key = host_key(is_local, host_id, &session.session_id);
        let info_el = match self.conductor_row_states.get(&key).cloned() {
            Some(state) => {
                let action = WorkspaceAction::AttachFleetSession {
                    host: host_label.to_string(),
                    host_id: host_id.map(str::to_string),
                    session_id: session.session_id.clone(),
                    is_local,
                };
                Hoverable::new(state, move |_mouse| info)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                    .finish()
            }
            None => info,
        };
        // Review verbs are local-only (they inspect the repo on this
        // machine); guardrails are cross-host, so every live row — local and
        // remote — gets them (step 7 design: attempt remote, never fake it).
        let review_verbs = is_local
            .then(|| self.render_review_verbs(is_local, host_id, session, appearance))
            .flatten();
        let guardrail_verbs =
            self.render_guardrail_verbs(host_label, host_id, is_local, session, appearance);
        // Model levers are local-only (they resume/fork into a local PTY);
        // compact/clear are additionally Claude-only (Claude Code slash commands).
        let lever_verbs = is_local
            .then(|| self.render_lever_verbs(is_local, host_id, session, app, appearance))
            .flatten();

        if review_verbs.is_none() && guardrail_verbs.is_none() && lever_verbs.is_none() {
            return info_el;
        }
        // The trailing toolbelt: review · guardrail · lever clusters separated
        // by hairline dividers so they read as tidy segments, not loose glyphs.
        let mut verbs_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(VERB_SPACING);
        let mut first = true;
        for cluster in [review_verbs, guardrail_verbs, lever_verbs]
            .into_iter()
            .flatten()
        {
            if !first {
                verbs_row = verbs_row.with_child(cluster_divider(appearance));
            }
            verbs_row = verbs_row.with_child(cluster);
            first = false;
        }
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(INFO_VERBS_GAP)
            .with_child(Shrinkable::new(1.0, info_el).finish())
            .with_child(verbs_row.with_main_axis_size(MainAxisSize::Min).finish())
            .with_main_axis_size(MainAxisSize::Max)
            .finish()
    }

    /// The model-lever cluster on a **local** Conductor session row (step 8):
    /// `⚙ /compact · ⌫ /clear · ⑂ fork · ⑂ +worktree`. Compact/clear resume the
    /// same conversation into a local tab and prefill the Claude Code slash
    /// command ([`WorkspaceAction::SlashCommandSession`]) — Claude-only; fork /
    /// +worktree branch the conversation ([`WorkspaceAction::ForkAgentSession`])
    /// for any provider with a fork mechanism, the worktree variant gated on the
    /// cwd being inside a git repo. Muted, accent on hover (constructive verbs,
    /// like the review cluster). `None` before the hover handles seed (first
    /// frame) or when the row exposes no lever at all.
    fn render_lever_verbs(
        &self,
        is_local: bool,
        host_id: Option<&str>,
        session: &SessionSnapshot,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let agent = match session.provider {
            Provider::Claude => CLIAgent::Claude,
            Provider::Codex => CLIAgent::Codex,
        };
        // Same subscription as the source session (None = default login).
        let config_dir = CockpitModel::as_ref(app).config_dir_for_session(&session.session_id);

        let rk = host_key(is_local, host_id, &session.session_id);
        let state = |verb: &str| {
            self.conductor_lever_states
                .get(&format!("{verb}\u{0}{rk}"))
                .cloned()
        };

        // One shared-style verb dispatching a WorkspaceAction (muted → accent).
        let make =
            |st: MouseStateHandle, label: &str, action: WorkspaceAction| -> Box<dyn Element> {
                verb_button(st, label, VerbKind::Constructive, appearance, action)
            };

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(VERB_SPACING);
        let mut any = false;

        // /compact + /clear — Claude Code slash commands; gated to Claude and to
        // a resumable session (belt-and-braces with the SlashCommandSession
        // handler's own resume-command guard).
        let has_resume = agent.resume_command(&session.session_id).is_some();
        if session.provider == Provider::Claude && has_resume {
            let slash = |command: &'static str| WorkspaceAction::SlashCommandSession {
                agent,
                session_id: session.session_id.clone(),
                cwd: PathBuf::from(&session.cwd),
                config_dir: config_dir.clone(),
                command: command.to_string(),
            };
            if let Some(st) = state("compact") {
                row = row.with_child(make(
                    st,
                    &crate::t!("cockpit-session-compact"),
                    slash("/compact"),
                ));
                any = true;
            }
            if let Some(st) = state("clear") {
                row = row.with_child(make(
                    st,
                    &crate::t!("cockpit-session-clear"),
                    slash("/clear"),
                ));
                any = true;
            }
        }

        // fork / +worktree — any provider with a fork mechanism; the worktree
        // variant only when the cwd is inside a git repo (design §3).
        if agent.fork_command(&session.session_id).is_some() {
            let fork = |into_worktree: bool| WorkspaceAction::ForkAgentSession {
                agent,
                session_id: session.session_id.clone(),
                cwd: PathBuf::from(&session.cwd),
                config_dir: config_dir.clone(),
                into_worktree,
            };
            if let Some(st) = state("fork") {
                row = row.with_child(make(st, &crate::t!("cockpit-session-fork"), fork(false)));
                any = true;
            }
            let in_repo = self
                .session_in_repo
                .get(&session.session_id)
                .copied()
                .unwrap_or(false);
            if in_repo {
                if let Some(st) = state("forkwt") {
                    row = row.with_child(make(
                        st,
                        &crate::t!("cockpit-session-fork-worktree"),
                        fork(true),
                    ));
                    any = true;
                }
            }
        }

        any.then(|| row.with_main_axis_size(MainAxisSize::Min).finish())
    }

    /// The review-loop verb cluster for a **local** Conductor session row (step
    /// 6): `◈ review · ✓ approve · ↻ redirect · ⎙ commit · ⬈ PR`. Each verb is
    /// muted, accent on hover, and dispatches its action — review / redirect /
    /// commit / PR to the workspace ([`WorkspaceAction`]); approve to this pane
    /// ([`CockpitPaneAction::MarkReviewed`], a local non-mutating marker that
    /// dims to "✓ reviewed"). `None` before the hover handles are seeded (first
    /// frame), so a row never renders half a cluster. Remote sessions get no
    /// cluster — their repo lives on the host (remote review is a follow-up via
    /// the daemon's `RunCommandRequest`).
    fn render_review_verbs(
        &self,
        is_local: bool,
        host_id: Option<&str>,
        session: &SessionSnapshot,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let theme = appearance.theme();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let accent = theme.accent().into_solid();

        let rk = host_key(is_local, host_id, &session.session_id);
        let state = |verb: &str| {
            self.conductor_review_states
                .get(&format!("{verb}\u{0}{rk}"))
                .cloned()
        };
        let project_root = PathBuf::from(&session.project_root);
        let project_name = session.project_name.clone();
        let reviewed = self.reviewed_sessions.contains(&rk);

        // One shared-style verb dispatching a WorkspaceAction (muted → accent).
        let make =
            |st: MouseStateHandle, label: &str, action: WorkspaceAction| -> Box<dyn Element> {
                verb_button(st, label, VerbKind::Constructive, appearance, action)
            };

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(VERB_SPACING);

        if let Some(st) = state("review") {
            row = row.with_child(make(
                st,
                "◈ review",
                WorkspaceAction::ReviewSession {
                    project_root: project_root.clone(),
                    project_name: project_name.clone(),
                },
            ));
        }
        // ✓ approve — local marker; label + rest color flip once reviewed
        // (a reviewed row rests in accent so the eye can move on).
        if let Some(st) = state("approve") {
            let label = if reviewed {
                "✓ reviewed"
            } else {
                "✓ approve"
            };
            let rest = if reviewed { accent } else { muted };
            row = row.with_child(verb_button_colored(
                st,
                label,
                rest,
                accent,
                appearance,
                CockpitPaneAction::MarkReviewed(rk.clone()),
            ));
        }
        if let Some(st) = state("redirect") {
            row = row.with_child(make(
                st,
                "↻ redirect",
                WorkspaceAction::AskAgentRouted {
                    prompt: review_redirect_prompt(&project_name),
                    config_dir: None,
                },
            ));
        }
        if let Some(st) = state("commit") {
            row = row.with_child(make(
                st,
                "⎙ commit",
                WorkspaceAction::CommitReviewChanges {
                    project_root: project_root.clone(),
                },
            ));
        }
        if let Some(st) = state("pr") {
            row = row.with_child(make(
                st,
                "⬈ PR",
                WorkspaceAction::CreateReviewPr { project_root },
            ));
        }

        Some(row.with_main_axis_size(MainAxisSize::Min).finish())
    }

    /// The guardrail verb cluster (step 7) for **every** live Conductor
    /// session row, local and remote alike: `⏸ stop` (SIGINT, dispatched
    /// immediately — no confirmation, mirrors Ctrl-C) and `⨯ kill` (SIGKILL,
    /// opens the confirm dialog first — destructive). Muted, turning Critical
    /// on hover (a danger affordance, distinct from the review cluster's
    /// muted→accent) — see `zaplex_cockpit::conductor`'s glyph/color
    /// vocabulary. Unlike [`Self::render_review_verbs`] this renders
    /// regardless of `is_local`: interrupt/kill are cross-host operations
    /// (local `libc::kill`, remote daemon `RunCommandRequest`).
    fn render_guardrail_verbs(
        &self,
        host_label: &str,
        host_id: Option<&str>,
        is_local: bool,
        session: &SessionSnapshot,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let rk = host_key(is_local, host_id, &session.session_id);
        let state = |verb: &str| {
            self.conductor_guardrail_states
                .get(&format!("{verb}\u{0}{rk}"))
                .cloned()
        };
        let label = zaplex_cockpit::session_label(session);

        // One shared-style verb dispatching a WorkspaceAction (muted →
        // attention amber — a danger affordance, quiet until reached for).
        let make =
            |st: MouseStateHandle, text: &str, action: WorkspaceAction| -> Box<dyn Element> {
                verb_button(st, text, VerbKind::Destructive, appearance, action)
            };

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(VERB_SPACING);

        if let Some(st) = state("pause") {
            row = row.with_child(make(
                st,
                "⏸ stop",
                WorkspaceAction::StopAgent {
                    host: host_label.to_string(),
                    host_id: host_id.map(str::to_string),
                    session_id: session.session_id.clone(),
                    pid: session.pid,
                    is_local,
                    agent_label: label.clone(),
                },
            ));
        }
        if let Some(st) = state("kill") {
            row = row.with_child(make(
                st,
                "⨯ kill",
                WorkspaceAction::KillAgentRequest {
                    host: host_label.to_string(),
                    host_id: host_id.map(str::to_string),
                    session_id: session.session_id.clone(),
                    pid: session.pid,
                    is_local,
                    agent_label: label,
                    project_name: session.project_name.clone(),
                },
            ));
        }

        Some(row.with_main_axis_size(MainAxisSize::Min).finish())
    }

    /// The fleet-wide "⏹ stop all" control (step 7), rendered in the Conductor
    /// header when at least one agent is live anywhere in the inventory.
    /// Dispatches [`WorkspaceAction::StopAllRequest`], which opens the confirm
    /// dialog (never sends anything without confirmation — this is the
    /// broadest, most destructive guardrail). Muted, Critical on hover.
    fn render_conductor_stop_all(&self, appearance: &Appearance) -> Box<dyn Element> {
        verb_button(
            self.conductor_stop_all_state.clone(),
            "⏹ stop all",
            VerbKind::Destructive,
            appearance,
            WorkspaceAction::StopAllRequest,
        )
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
                    heat_coloru(HeatLevel::Critical)
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

        let snapshot = CockpitModel::as_ref(app).snapshot().clone();

        let content: Box<dyn Element> = if snapshot.accounts.is_empty() {
            Self::text(
                crate::t!("workspace-left-panel-cockpit-empty"),
                family,
                body,
                muted,
            )
        } else {
            let mut col = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(
                    Container::new(self.render_aggregate(&snapshot.accounts, appearance))
                        .with_margin_bottom(CARD_SPACING * 2.0)
                        .finish(),
                );
            // The Conductor — the unified cross-host Agent-Inventory (this
            // machine + every connected daemon), not a locally-rebuilt tree.
            let model = CockpitModel::as_ref(app);
            let inventory = model.inventory().clone();
            if let Some(conductor) = self.render_conductor(&inventory, app, appearance) {
                col = col.with_child(
                    Container::new(conductor)
                        .with_margin_bottom(CARD_SPACING * 2.0)
                        .finish(),
                );
            }
            for acct in &snapshot.accounts {
                // Per-account override color (instances.json), resolved from the
                // model and parsed from its hex string.
                let override_color = CockpitModel::as_ref(app)
                    .override_color(&acct.account.key)
                    .and_then(parse_hex_color);
                col = col.with_child(self.render_card(acct, override_color, appearance));
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
            CockpitPaneAction::MarkReviewed(key) => {
                if !self.reviewed_sessions.remove(key) {
                    self.reviewed_sessions.insert(key.clone());
                }
                ctx.notify();
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
        unimplemented!()
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
        _app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::simple(crate::t!("cockpit-pane-title"))
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

#[cfg(test)]
mod tests {
    use super::parse_hex_color;

    #[test]
    fn parses_six_digit_hex() {
        let c = parse_hex_color("#22C55E").expect("valid 6-digit hex");
        assert_eq!((c.r, c.g, c.b, c.a), (0x22, 0xC5, 0x5E, 255));
    }

    #[test]
    fn parses_three_digit_shorthand() {
        // #f0a → ff 00 aa (each nibble doubled).
        let c = parse_hex_color("#f0a").expect("valid 3-digit hex");
        assert_eq!((c.r, c.g, c.b, c.a), (0xff, 0x00, 0xaa, 255));
    }

    #[test]
    fn rejects_malformed_returns_none() {
        for bad in [
            "", "22C55E", "#", "#12", "#1234", "#12345", "#GGGGGG", "#12345Z",
        ] {
            assert!(parse_hex_color(bad).is_none(), "{bad:?} must not parse");
        }
    }
}
