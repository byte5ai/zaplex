//! The **Spawn-Karte** — the launch card that makes model + thinking-effort a
//! real, *visible* launch attribute.
//!
//! Launching an agent used to set no model/effort at all — the core gap this
//! closes. Starting Haiku/Low is a very different thing from starting a top
//! model at Extra-High, so the card surfaces, in labeled rows the user cannot
//! misread, exactly what will start: **Agent · Model · Effort · Context ·
//! Account · Host · Project**. Smart defaults (a sane agent+model, `freest`
//! account, the launch context's host+project) make the common case a single
//! confirm; every control is still one click away.
//!
//! The card is self-contained: it holds its own selection state and its option
//! lists (accounts/hosts) are injected by the workspace via [`SpawnCardConfig`]
//! when it opens, so the modal never reaches into app-global state itself. On
//! confirm it emits [`SpawnCardEvent::Launch`]; the workspace turns that into a
//! [`crate::workspace::action::WorkspaceAction::LaunchAgent`] launch.

use std::path::PathBuf;

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Flex, Hoverable,
    MainAxisSize, MouseStateHandle, ParentElement, Radius, Rect, Text,
};
use warpui::keymap::FixedBinding;
use warpui::platform::file_picker::{FilePickerConfiguration, FilePickerError};
use warpui::platform::Cursor;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ui_components::modal_frame;
use crate::view_components::action_button::ActionButton;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::terminal::CLIAgent;

const MODAL_WIDTH: f32 = 480.;

/// One account option offered in the card.
#[derive(Clone, Debug)]
pub struct AccountOption {
    /// Display name — already the user's alias where one is set (A1: the
    /// overrides layer replaced it before the snapshot existed).
    pub label: String,
    pub config_dir: PathBuf,
    /// Binding-window utilisation, already formatted (`~` marks an estimate).
    pub heat_label: String,
    /// The same figure as a fraction, so the card can colour it by the one
    /// utilisation rule instead of parsing its own label back.
    pub heat: f64,
    /// Plan tier, when the provider told us.
    pub plan: Option<String>,
    pub provider: zaplex_cockpit::Provider,
}

/// The account options for one provider, plus its precomputed freest pick.
#[derive(Clone, Debug, Default)]
pub struct ProviderOptions {
    pub installed: bool,
    /// Display label of the freest account (heat included), if any.
    pub freest_label: Option<String>,
    /// Config dir of the freest account (`None` = only the default login).
    pub freest_dir: Option<PathBuf>,
    /// The freest account itself — what the auto line shows (X1). The pick comes
    /// from `routing::pick_freest`, which stays the truth: it ranks by the
    /// binding window and deprioritises working accounts, and this card only
    /// *shows* its answer. It never skips an account, at 85 % or anywhere —
    /// "fast voll" is a visual mark (§1.2), not a routing rule.
    pub freest: Option<AccountOption>,
    pub accounts: Vec<AccountOption>,
}

/// One connected SSH host the agent can be launched on.
#[derive(Clone, Debug)]
pub struct HostOption {
    pub id: String,
    pub name: String,
}

/// Everything the workspace injects when opening the card. Rebuilt on each open
/// so account heat / host lists are fresh.
#[derive(Clone, Debug, Default)]
pub struct SpawnCardConfig {
    pub claude: ProviderOptions,
    pub codex: ProviderOptions,
    pub hosts: Vec<HostOption>,
    /// Pre-scoped host **id** in the same id space as [`HostOption::id`] — the
    /// SSH `node.id`. The Conductor scopes by the Agent-inventory's *daemon*
    /// `HostId`, so the workspace translates that to the hosting SSH node before
    /// filling this field (see `WorkspaceView::translate_scoped_daemon_host`);
    /// the two id spaces must not be compared directly. `None` = local, or a
    /// remote daemon that could not be translated to a live SSH node. This is
    /// the authoritative scoping key: same-named hosts are disambiguated by id,
    /// so when present it resolves the scoped host before
    /// [`Self::scoped_host_name`] is consulted.
    pub scoped_host_id: Option<String>,
    /// Pre-scoped host **name/label** (from a Conductor host/project-header `+`);
    /// `None` = local. Used as the resolution fallback when
    /// [`Self::scoped_host_id`] is absent — including the remote case where the
    /// daemon id could not be translated to a live SSH node, so a remote-scoped
    /// open still lands on the right-named host rather than silently on Local.
    pub scoped_host_name: Option<String>,
    /// Pre-scoped project dir (from a Conductor project-header `+` / context).
    pub project: Option<PathBuf>,
    /// Optional task prompt to prefill into the launched agent's input — the
    /// contextual "run this task with an agent" flows (Fix-with-agent, the GitHub
    /// instance-flows) route through the card carrying this, so every launch goes
    /// through the one explicit launch grammar instead of a blind one-click.
    pub prompt: Option<String>,
    /// Explicit agent the opener requested (e.g. "Fix with Codex", the per-agent
    /// menu action). Preselects the agent in the card so the user's intent is not
    /// silently changed to the default; ignored if that agent isn't installed.
    /// `None` = use the card's own default (Claude if installed, else Codex).
    pub default_agent: Option<CLIAgent>,
}

/// Which account the launch pins to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccountChoice {
    /// The least-loaded account for the chosen provider (`pick_freest`).
    Freest,
    /// A specific account by index into the provider's list.
    Specific(usize),
}

/// Where the agent launches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostChoice {
    Local,
    /// A connected SSH host by index into [`SpawnCardConfig::hosts`].
    Remote(usize),
}

/// Resolve a pre-scoped Conductor host to a [`HostChoice`].
///
/// Prefers the stable `scoped_id`: two connected hosts can share a display
/// label, so matching by name alone picks the *first* same-named node and can
/// prep a launch on the wrong remote host. Only when no id is available (or it
/// matches nothing) do we fall back to matching by `scoped_name`. Absent both,
/// or with no match, the launch stays local.
fn resolve_scoped_host(
    hosts: &[HostOption],
    scoped_id: Option<&str>,
    scoped_name: Option<&str>,
) -> HostChoice {
    // Authoritative: resolve by stable id.
    if let Some(id) = scoped_id {
        if let Some(pos) = hosts.iter().position(|h| h.id == id) {
            return HostChoice::Remote(pos);
        }
    }
    // Fallback: resolve by display label (only when no id was supplied).
    if scoped_id.is_none() {
        if let Some(name) = scoped_name {
            if let Some(pos) = hosts.iter().position(|h| h.name == name) {
                return HostChoice::Remote(pos);
            }
        }
    }
    HostChoice::Local
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;
    app.register_fixed_bindings([
        FixedBinding::new("escape", SpawnCardAction::Close, id!("SpawnCard")),
        FixedBinding::new("enter", SpawnCardAction::Confirm, id!("SpawnCard")),
    ]);
}

pub struct SpawnCard {
    cfg: SpawnCardConfig,
    agent: CLIAgent,
    model: String,
    effort: String,
    account: AccountChoice,
    /// Whether the account list is unfolded (X1). Collapsed by default: the
    /// router already chose, so the card states the answer instead of asking the
    /// question again on every launch.
    show_accounts: bool,
    host: HostChoice,
    project: Option<PathBuf>,
    /// Task prompt to prefill into the launched agent after start (contextual
    /// flows); `None` for a plain "new agent" open.
    prompt: Option<String>,
    /// Single-line text input for the remote launch directory. Remote hosts need
    /// a *selectable* directory (Codex gate), but a native folder picker — used
    /// for local launches — cannot browse a remote filesystem, so remote hosts
    /// type the absolute path here. `Option` so the pure unit tests can build
    /// `SpawnCard` literals without a `ViewContext` (set to `None`); the real
    /// [`Self::new`] always builds it (`Some`).
    remote_dir_editor: Option<ViewHandle<EditorView>>,
    chip_states: std::cell::RefCell<std::collections::HashMap<String, MouseStateHandle>>,
    /// The shared modal close ✕ (top-right), built via [`modal_frame::close_button`]
    /// so the Spawn-Karte carries the same corner ✕ as every other modal.
    /// `Option` for the same reason as `remote_dir_editor`: the pure unit tests
    /// build `SpawnCard` literals without a `ViewContext` (`None`); the real
    /// [`Self::new`] always builds it (`Some`).
    close_button: Option<ViewHandle<ActionButton>>,
}

/// The models offered per agent (curated, high-signal — not an exhaustive list).
/// First entry is the default. Claude uses the short aliases its CLI accepts.
fn models_for(agent: CLIAgent) -> &'static [&'static str] {
    match agent {
        CLIAgent::Claude => &["opus", "sonnet", "haiku"],
        CLIAgent::Codex => &["gpt-5-codex", "gpt-5", "gpt-5-mini"],
        _ => &[],
    }
}

/// The thinking-effort presets exposed by each CLI. Codex accepts the three
/// explicit `model_reasoning_effort` values; Claude has no equivalent CLI flag,
/// so the card must not pretend its selection changes the launched command.
const CODEX_EFFORTS: &[&str] = &["low", "medium", "high"];
const CLAUDE_EFFORTS: &[&str] = &["CLI default"];

fn effort_options_for(agent: CLIAgent) -> &'static [&'static str] {
    match agent {
        CLIAgent::Codex => CODEX_EFFORTS,
        CLIAgent::Claude => CLAUDE_EFFORTS,
        _ => &[],
    }
}

fn default_effort_for(agent: CLIAgent) -> &'static str {
    match agent {
        CLIAgent::Codex => "high",
        CLIAgent::Claude => "CLI default",
        _ => "",
    }
}

/// Which agents are actually launchable, per install-detection in `cfg`. Pure
/// helper (no view state) so the "neither installed" case is unit-testable:
/// an empty result means the card must not offer a launchable agent chip and
/// Confirm must stay disabled — there is nothing installed to run.
fn installed_agents(cfg: &SpawnCardConfig) -> Vec<CLIAgent> {
    [CLIAgent::Claude, CLIAgent::Codex]
        .into_iter()
        .filter(|agent| match agent {
            CLIAgent::Codex => cfg.codex.installed,
            _ => cfg.claude.installed,
        })
        .collect()
}

/// Map the remote-dir text input to a launch cwd. Only remote hosts use the
/// typed field (a native folder picker can't browse a remote filesystem); a
/// blank field means "the host's home directory" (`None`). Local hosts never
/// use it, so they map to `None` too. Pure + ctx-free so the trim/blank→None
/// mapping is unit-testable without an editor view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteCwdError {
    RelativePath,
}

fn remote_cwd_from_input(host: HostChoice, raw: &str) -> Result<Option<PathBuf>, RemoteCwdError> {
    if !matches!(host, HostChoice::Remote(_)) {
        return Ok(None);
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Ok(None)
    // Remote native-session hosts use Unix shells even when the client runs on
    // Windows. Validate their POSIX path grammar rather than the client OS's
    // `Path::is_absolute` semantics.
    } else if trimmed.starts_with('/') {
        Ok(Some(PathBuf::from(trimmed)))
    } else {
        Err(RemoteCwdError::RelativePath)
    }
}

impl SpawnCard {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        // The remote-dir input. Built here where a `ViewContext` is available;
        // the pure unit tests construct `SpawnCard` literals with
        // `remote_dir_editor: None`, which is why the field is an `Option`.
        let remote_dir_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = SingleLineEditorOptions {
                text: TextOptions::ui_text(Some(13.), appearance),
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text(
                crate::t!("cockpit-spawn-card-remote-dir-placeholder"),
                ctx,
            );
            editor
        });

        // Re-render the card as the user edits the remote dir, so the
        // "Launching: …" summary line stays truthful to the typed path (the
        // child editor's own edits notify *itself*, not this parent view —
        // mirrors the drive enum dialog, which notifies on editor edits).
        ctx.subscribe_to_view(&remote_dir_editor, |_, _, event, ctx| {
            if matches!(event, EditorEvent::Edited(_)) {
                ctx.notify();
            }
        });

        let close_button =
            ctx.add_view(|_ctx| modal_frame::close_button(SpawnCardAction::Close));

        SpawnCard {
            cfg: SpawnCardConfig::default(),
            agent: CLIAgent::Claude,
            model: "opus".to_string(),
            effort: CLAUDE_EFFORTS[0].to_string(),
            account: AccountChoice::Freest,
            show_accounts: false,
            host: HostChoice::Local,
            project: None,
            prompt: None,
            remote_dir_editor: Some(remote_dir_editor),
            chip_states: Default::default(),
            close_button: Some(close_button),
        }
    }

    /// (Re)initialize the card from a fresh config + optional pre-scoping. Picks
    /// smart defaults so the common case is a single confirm.
    ///
    /// Takes a `ViewContext` because a remote-scoped open may prefill the
    /// remote-dir editor buffer (touching an editor view needs ctx).
    pub fn configure(&mut self, cfg: SpawnCardConfig, ctx: &mut ViewContext<Self>) {
        // Agent: honor an explicit requested agent (contextual "Fix with <agent>"
        // / per-agent action) when it is one of the supported CLIs and installed,
        // so the opener's intent isn't silently changed. Otherwise fall back to
        // Claude if available, else Codex.
        let requested = cfg
            .default_agent
            .filter(|a| matches!(a, CLIAgent::Claude | CLIAgent::Codex))
            .filter(|a| match a {
                CLIAgent::Codex => cfg.codex.installed,
                _ => cfg.claude.installed,
            });
        self.agent = requested.unwrap_or({
            if cfg.claude.installed || !cfg.codex.installed {
                CLIAgent::Claude
            } else {
                CLIAgent::Codex
            }
        });
        self.model = models_for(self.agent)
            .first()
            .copied()
            .unwrap_or("opus")
            .to_string();
        self.effort = default_effort_for(self.agent).to_string();
        self.account = AccountChoice::Freest;
        // Pre-scope host from a Conductor host/project `+`, else local. Resolve
        // by stable id first so same-named hosts route to the right node.
        self.host = resolve_scoped_host(
            &cfg.hosts,
            cfg.scoped_host_id.as_deref(),
            cfg.scoped_host_name.as_deref(),
        );
        // `self.project` is the LOCAL launch dir (native folder picker). For a
        // remote-scoped open the pre-scoped dir is a REMOTE path, which belongs in
        // the remote-dir editor (seeded below), NOT here — otherwise switching the
        // host to Local would launch locally into a remote-only path (Codex: host
        // switch dir leakage). So keep the local project empty for a remote host.
        self.project = match self.host {
            HostChoice::Local => cfg.project.clone(),
            HostChoice::Remote(_) => None,
        };
        self.prompt = cfg.prompt.clone();
        self.cfg = cfg;

        // Prefill the remote-dir input from the pre-scoped project (a Conductor
        // remote project node) so the common case is a single confirm; every other
        // open (local, or remote without a project) resets it to empty (blank =
        // host home). Reset on every open so a stale path can't leak across opens.
        if let Some(editor) = self.remote_dir_editor.clone() {
            let prefill = match (self.host, &self.cfg.project) {
                (HostChoice::Remote(_), Some(dir)) => dir.display().to_string(),
                _ => String::new(),
            };
            editor.update(ctx, |ed, ctx| {
                ed.set_buffer_text_with_base_buffer(&prefill, ctx);
            });
        }

        // Invalidate THIS view so the freshly-applied config actually repaints.
        // Mutating a child view via `ViewHandle::update` does not mark it dirty —
        // `open_spawn_card` only notifies the WorkspaceView, not the card — so
        // without this the card kept showing its first cached render built from
        // `SpawnCardConfig::default()` (both providers `installed=false`), i.e. a
        // permanent, false "No agent CLI installed" no matter what detection found.
        // Every interactive mutation in `handle_action` already notifies; this was
        // the one entry point (external configure) that didn't. (Root cause found
        // independently by codex + grok.)
        ctx.notify();
    }

    /// Fill the remote-directory field from a directory chosen in the SFTP
    /// browser pick flow (#105). The card's other selections are untouched (it is
    /// a persistent view — hidden, not rebuilt, during the browse).
    pub fn set_remote_dir(&mut self, path: &std::path::Path, ctx: &mut ViewContext<Self>) {
        if let Some(editor) = self.remote_dir_editor.clone() {
            let text = path.display().to_string();
            editor.update(ctx, |ed, ctx| {
                ed.set_buffer_text_with_base_buffer(&text, ctx);
            });
            ctx.notify();
        }
    }

    /// **X1** — the account section: one calm auto line, and the list only when
    /// asked for.
    ///
    /// Auto is the point. The router already picks well — by the binding window,
    /// deprioritising accounts that are working — so the card's job is to say
    /// *which* account that is and let you look away. A row of chips made the
    /// user re-decide a decision that had already been made well, every single
    /// launch.
    ///
    /// "Ändern" reveals the full list with each account's utilisation. Nothing is
    /// hidden — it is one click behind the answer instead of in front of it.
    fn account_controls(&self, appearance: &Appearance) -> Vec<Box<dyn Element>> {
        let opts = self.provider_options();
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let main = theme.main_text_color(theme.background()).into_solid();
        let faint = theme
            .sub_text_color(theme.background())
            .with_opacity(55)
            .into_solid();

        // Collapsed: the account the router chose, stated plainly.
        if !self.show_accounts {
            let Some(freest) = opts.freest.clone() else {
                // No accounts discovered — the card launches under the provider's
                // default login, and saying "auto" would imply a choice was made.
                return vec![Container::new(
                    Text::new_inline(crate::t!("cockpit-spawn-card-freest"), family, 12.)
                        .with_color(muted)
                        .finish(),
                )
                .finish()];
            };
            let selected = match self.account {
                AccountChoice::Freest => freest.clone(),
                AccountChoice::Specific(i) => {
                    opts.accounts.get(i).cloned().unwrap_or(freest.clone())
                }
            };
            let auto = matches!(self.account, AccountChoice::Freest);

            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(7.0)
                // Provider tile — the same identity colour the cards carry,
                // contrast-picked for the theme.
                .with_child(
                    ConstrainedBox::new(
                        Rect::new()
                            .with_background_color(crate::cockpit::style::provider_color_on(
                                selected.provider,
                                theme.background().into_solid(),
                            ))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.0)))
                            .finish(),
                    )
                    .with_width(8.0)
                    .with_height(8.0)
                    .finish(),
                )
                .with_child(
                    Text::new_inline(selected.label.clone(), family, 12.)
                        .with_color(main)
                        .finish(),
                );
            if let Some(plan) = &selected.plan {
                row = row.with_child(
                    Text::new_inline(plan.clone(), family, 12.)
                        .with_color(faint)
                        .finish(),
                );
            }
            // The binding window's utilisation, by the one rule: calm grey below
            // the threshold, true red at or above it (§1.2).
            row = row.with_child(
                Text::new_inline(selected.heat_label.clone(), family, 12.)
                    .with_color(crate::cockpit::style::utilisation_coloru(
                        selected.heat,
                        appearance,
                    ))
                    .finish(),
            );
            if auto {
                row = row.with_child(
                    Text::new_inline(crate::t!("cockpit-spawn-card-auto"), family, 11.)
                        .with_color(faint)
                        .finish(),
                );
            }
            let line = row.with_main_axis_size(MainAxisSize::Min).finish();
            return vec![
                line,
                self.chip(
                    "acct-change",
                    crate::t!("cockpit-spawn-card-change"),
                    false,
                    SpawnCardAction::ToggleAccountList,
                    appearance,
                ),
            ];
        }

        // Expanded: every account with its utilisation, plus the auto option.
        let freest_label = opts
            .freest_label
            .clone()
            .map(|l| crate::t!("cockpit-spawn-card-freest-named", label = l))
            .unwrap_or_else(|| crate::t!("cockpit-spawn-card-freest"));
        let mut chips = vec![self.chip(
            "acct-freest",
            freest_label,
            self.account == AccountChoice::Freest,
            SpawnCardAction::SetAccountFreest,
            appearance,
        )];
        for (i, a) in opts.accounts.iter().enumerate() {
            chips.push(self.chip(
                &format!("acct-{i}"),
                format!("{} ({})", a.label, a.heat_label),
                self.account == AccountChoice::Specific(i),
                SpawnCardAction::SetAccount(i),
                appearance,
            ));
        }
        chips
    }

    fn provider_options(&self) -> &ProviderOptions {
        match self.agent {
            CLIAgent::Codex => &self.cfg.codex,
            _ => &self.cfg.claude,
        }
    }

    /// `true` once at least one supported agent CLI is installed. When this is
    /// `false` the card has nothing it could actually launch, so Confirm must
    /// be inert (see [`SpawnCardAction::Confirm`] handling).
    fn any_agent_installed(&self) -> bool {
        !installed_agents(&self.cfg).is_empty()
    }

    /// Resolve the chosen account to a config dir for the launch. Remote hosts
    /// always use their own default account (config dirs are local paths), so a
    /// remote launch yields `None` regardless of the account chip.
    fn resolved_config_dir(&self) -> Option<PathBuf> {
        if matches!(self.host, HostChoice::Remote(_)) {
            return None;
        }
        match self.account {
            AccountChoice::Freest => self.provider_options().freest_dir.clone(),
            AccountChoice::Specific(i) => self
                .provider_options()
                .accounts
                .get(i)
                .map(|a| a.config_dir.clone()),
        }
    }

    fn resolved_node_id(&self) -> Option<String> {
        match self.host {
            HostChoice::Local => None,
            HostChoice::Remote(i) => self.cfg.hosts.get(i).map(|h| h.id.clone()),
        }
    }

    /// Display name of the currently selected remote host (`None` when local).
    fn remote_host_name(&self) -> Option<&str> {
        match self.host {
            HostChoice::Local => None,
            HostChoice::Remote(i) => self.cfg.hosts.get(i).map(|h| h.name.as_str()),
        }
    }

    fn selected_remote_cwd(&self, app: &AppContext) -> Result<Option<PathBuf>, RemoteCwdError> {
        let raw = self
            .remote_dir_editor
            .as_ref()
            .map(|editor| editor.as_ref(app).buffer_text(app))
            .unwrap_or_default();
        remote_cwd_from_input(self.host, &raw)
    }

    /// The [`SpawnCardEvent::Launch`] the current selection will emit on Confirm,
    /// or `None` when nothing is installed to launch (Confirm must then be inert
    /// — a phantom chip must never launch a missing binary). Kept pure (no
    /// `ViewContext`) so the confirm payload — the model/effort/account/host/
    /// project the launch actually carries — is unit-testable.
    fn launch_payload(&self) -> Option<SpawnCardEvent> {
        self.any_agent_installed().then(|| SpawnCardEvent::Launch {
            agent: self.agent,
            config_dir: self.resolved_config_dir(),
            cwd: self.project.clone(),
            node_id: self.resolved_node_id(),
            model: Some(self.model.clone()),
            effort: matches!(self.agent, CLIAgent::Codex).then(|| self.effort.clone()),
            prompt: self.prompt.clone(),
        })
    }

    fn launch_payload_for_remote_input(&self, raw: Option<&str>) -> Option<SpawnCardEvent> {
        let mut launch = self.launch_payload()?;
        if matches!(self.host, HostChoice::Remote(_)) {
            let cwd = remote_cwd_from_input(self.host, raw.unwrap_or_default()).ok()?;
            if let SpawnCardEvent::Launch {
                cwd: launch_cwd, ..
            } = &mut launch
            {
                *launch_cwd = cwd;
            }
        }
        Some(launch)
    }

    fn chip_handle(&self, id: &str) -> MouseStateHandle {
        self.chip_states
            .borrow_mut()
            .entry(id.to_string())
            .or_insert_with(MouseStateHandle::default)
            .clone()
    }

    /// A labeled, clickable selection chip. Selected = accent fill; unselected =
    /// a calm surface fill that brightens on hover.
    fn chip(
        &self,
        id: &str,
        label: String,
        selected: bool,
        action: SpawnCardAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let accent = theme.accent();
        let surface = theme.surface_2();
        let surface_hover = theme.surface_3();
        let main = theme.main_text_color(theme.background()).into_solid();
        let on_accent = ColorU::white();
        let handle = self.chip_handle(id);
        Hoverable::new(handle, move |mouse| {
            let (bg, fg) = if selected {
                (accent, on_accent)
            } else if mouse.is_hovered() {
                (surface_hover, main)
            } else {
                (surface, main)
            };
            Container::new(
                Text::new_inline(label.clone(), family, 13.)
                    .with_color(fg)
                    .finish(),
            )
            .with_horizontal_padding(10.)
            .with_vertical_padding(5.)
            .with_background(bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
    }

    fn label_text(&self, s: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        Container::new(
            Text::new_inline(s.to_string(), appearance.ui_font_family(), 11.)
                .with_color(muted)
                .finish(),
        )
        .with_margin_bottom(4.)
        .finish()
    }

    /// A `label:` heading over a horizontal wrap of chips.
    fn row(
        &self,
        label: &str,
        chips: Vec<Box<dyn Element>>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut chip_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        for (i, chip) in chips.into_iter().enumerate() {
            if i > 0 {
                chip_row = chip_row.with_child(Container::new(chip).with_margin_left(6.).finish());
            } else {
                chip_row = chip_row.with_child(chip);
            }
        }
        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(self.label_text(label, appearance))
                .with_child(chip_row.finish())
                .finish(),
        )
        .with_margin_bottom(14.)
        .finish()
    }

    fn context_label(model: &str) -> String {
        let ctx = zaplex_cockpit::context_window(model);
        if ctx >= 1_000_000 {
            format!("{}M ctx", ctx / 1_000_000)
        } else {
            format!("{}k ctx", ctx / 1_000)
        }
    }

    fn cap(s: &str) -> String {
        let mut c = s.chars();
        match c.next() {
            Some(first) => first.to_uppercase().chain(c).collect(),
            None => String::new(),
        }
    }

    /// The full launch summary, e.g.
    /// `Claude Code · opus · High · 1M ctx · freest · local`.
    ///
    /// Takes `app` so the remote-host branch can reflect the *typed* remote dir
    /// (read from the editor buffer) — the "Launching: …" line must be the truth
    /// of what will start, and remote dir is steering. Local still reads
    /// `self.project` (the folder-picker result), unchanged.
    fn summary(&self, app: &AppContext) -> String {
        let account = match self.host {
            HostChoice::Remote(_) => crate::t!("cockpit-spawn-card-sum-host-account"),
            HostChoice::Local => match self.account {
                AccountChoice::Freest => crate::t!("cockpit-spawn-card-sum-freest"),
                AccountChoice::Specific(i) => self
                    .provider_options()
                    .accounts
                    .get(i)
                    .map(|a| a.label.clone())
                    .unwrap_or_else(|| crate::t!("cockpit-spawn-card-sum-freest")),
            },
        };
        let host = match self.host {
            HostChoice::Local => crate::t!("cockpit-spawn-card-sum-local"),
            HostChoice::Remote(i) => self
                .cfg
                .hosts
                .get(i)
                .map(|h| h.name.clone())
                .unwrap_or_else(|| crate::t!("cockpit-spawn-card-sum-remote")),
        };
        // Directory is always part of the summary — a first-class launch
        // attribute, never omitted (Codex gate: "dir is steering"). An unset dir
        // reads as the explicit default rather than silently vanishing. For a
        // remote host the dir is the *typed* input (read live from the editor so
        // the preview matches what Confirm will launch); local uses the
        // folder-picker result in `self.project`.
        let dir = if matches!(self.host, HostChoice::Remote(_)) {
            match self.selected_remote_cwd(app) {
                Ok(Some(path)) => path.display().to_string(),
                Ok(None) => crate::t!(
                    "cockpit-spawn-card-sum-host-home",
                    host = self.remote_host_name().unwrap_or("host")
                ),
                Err(RemoteCwdError::RelativePath) => {
                    crate::t!("fm-toast-invalid-target-path")
                }
            }
        } else {
            match &self.project {
                Some(dir) => dir.display().to_string(),
                None => crate::t!("cockpit-spawn-card-sum-default"),
            }
        };
        format!(
            "{} · {} · {} · {} · {} · {} · {}",
            self.agent.display_name(),
            self.model,
            Self::cap(&self.effort),
            Self::context_label(&self.model),
            account,
            host,
            dir,
        )
    }

    /// The remote launch dir as an editable single-line text input. A native
    /// folder picker can't browse a remote filesystem, so remote hosts type the
    /// absolute path here (blank = the host's home dir). Mirrors the drive
    /// enum-dialog's `render_name_editor` element construction — a bordered
    /// container wrapping the editor's `text_input`. Returns `None` only when no
    /// editor was built (the ctx-free unit-test path where it is `None`).
    fn render_remote_dir_input(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let editor = self.remote_dir_editor.as_ref()?;
        let theme = appearance.theme();
        Some(
            ConstrainedBox::new(
                Container::new(
                    appearance
                        .ui_builder()
                        .text_input(editor.clone())
                        .with_style(UiComponentStyles::default())
                        .build()
                        .finish(),
                )
                .with_horizontal_padding(10.)
                .with_vertical_padding(6.)
                .with_background(theme.surface_2())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .finish(),
            )
            // Card inner width = MODAL_WIDTH minus the 24px uniform padding each
            // side, less room for the trailing "Browse…" chip (#105).
            .with_width(MODAL_WIDTH - 48. - 104.)
            .finish(),
        )
    }

    fn render_card(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        // The one shared modal header (title · subtitle · close ✕) — identical
        // grammar to the attention inbox and every migrated dialog.
        let close = match self.close_button.as_ref() {
            Some(view) => warpui::elements::ChildView::new(view).finish(),
            None => Container::new(Flex::row().finish()).finish(),
        };
        let header = modal_frame::modal_header(
            crate::t!("cockpit-spawn-card-title"),
            Some(crate::t!("cockpit-spawn-card-subtitle")),
            close,
            appearance,
        );

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(Container::new(header).with_margin_bottom(18.).finish());

        // Agent row (only installed providers).
        let available = installed_agents(&self.cfg);
        if available.is_empty() {
            // Neither Claude nor Codex is installed: there is nothing to launch,
            // so show a calm install prompt instead of a phantom, unlaunchable
            // chip (Confirm is disabled below for the same reason).
            col = col.with_child(self.row(
                &crate::t!("cockpit-spawn-card-agent"),
                vec![Container::new(
                    Text::new_inline(crate::t!("cockpit-spawn-card-no-cli"), family, 12.)
                        .with_color(muted)
                        .finish(),
                )
                .finish()],
                appearance,
            ));
        } else {
            let agent_chips: Vec<Box<dyn Element>> = available
                .into_iter()
                .map(|agent| {
                    self.chip(
                        &format!("agent-{}", agent.to_serialized_name()),
                        agent.display_name().to_string(),
                        self.agent == agent,
                        SpawnCardAction::SetAgent(agent),
                        appearance,
                    )
                })
                .collect();
            col = col.with_child(self.row(&crate::t!("cockpit-spawn-card-agent"), agent_chips, appearance));
        }

        // Model row + a live context-window readout.
        let mut model_chips: Vec<Box<dyn Element>> = models_for(self.agent)
            .iter()
            .map(|m| {
                self.chip(
                    &format!("model-{m}"),
                    m.to_string(),
                    self.model == *m,
                    SpawnCardAction::SetModel(m.to_string()),
                    appearance,
                )
            })
            .collect();
        model_chips.push(
            Container::new(
                Text::new_inline(Self::context_label(&self.model), family, 12.)
                    .with_color(muted)
                    .finish(),
            )
            .with_margin_left(8.)
            .finish(),
        );
        col = col.with_child(self.row(&crate::t!("cockpit-spawn-card-model"), model_chips, appearance));

        // Effort row.
        let effort_chips: Vec<Box<dyn Element>> = if matches!(self.agent, CLIAgent::Claude) {
            vec![Container::new(
                Text::new_inline(CLAUDE_EFFORTS[0].to_string(), family, 13.)
                    .with_color(muted)
                    .finish(),
            )
            .finish()]
        } else {
            effort_options_for(self.agent)
                .iter()
                .map(|e| {
                    self.chip(
                        &format!("effort-{e}"),
                        Self::cap(e),
                        self.effort == *e,
                        SpawnCardAction::SetEffort(e.to_string()),
                        appearance,
                    )
                })
                .collect()
        };
        col = col.with_child(self.row(
            &crate::t!("cockpit-spawn-card-effort"),
            effort_chips,
            appearance,
        ));

        // Account row — freest + explicit accounts, unless a remote host owns it.
        // On a remote host there is no local account routing: the agent runs under
        // that host's own CLI login, so we say so explicitly (rather than letting
        // the local "freest"/per-account choice silently apply and mislead).
        if matches!(self.host, HostChoice::Remote(_)) {
            let host = self.remote_host_name().unwrap_or("the host");
            col = col.with_child(self.row(
                &crate::t!("cockpit-spawn-card-account"),
                vec![Container::new(
                    Text::new_inline(
                        crate::t!("cockpit-spawn-card-remote-login", host = host),
                        family,
                        12.,
                    )
                    .with_color(muted)
                    .finish(),
                )
                .finish()],
                appearance,
            ));
        } else {
            col = col.with_child(self.row(
                &crate::t!("cockpit-spawn-card-account"),
                self.account_controls(appearance),
                appearance,
            ));
        }

        // Host row — local + connected SSH hosts.
        let mut host_chips = vec![self.chip(
            "host-local",
            crate::t!("cockpit-spawn-card-host-local"),
            self.host == HostChoice::Local,
            SpawnCardAction::SetHostLocal,
            appearance,
        )];
        for (i, h) in self.cfg.hosts.iter().enumerate() {
            host_chips.push(self.chip(
                &format!("host-{i}"),
                h.name.clone(),
                self.host == HostChoice::Remote(i),
                SpawnCardAction::SetHost(i),
                appearance,
            ));
        }
        col = col.with_child(self.row(&crate::t!("cockpit-spawn-card-host"), host_chips, appearance));

        // Directory row — the launch dir as an *explicit* choice (Codex #2), not
        // a blind default. Local: a native folder picker, plus a reset to the
        // home default once a dir is chosen. Remote: the path lives on the host,
        // which a local picker cannot browse, so the user types it into a
        // single-line text input (prefilled from a project-scoped `+`, blank =
        // host home).
        let dir_display = self.project.as_ref().map(|p| p.display().to_string());
        if matches!(self.host, HostChoice::Remote(_)) {
            let host = self.remote_host_name().unwrap_or("the host");
            // A local folder picker cannot browse a remote filesystem, so the
            // remote launch dir is a typed text input (an absolute path on the
            // host; blank = the host's home). The row heading names the host so
            // it is unambiguous which filesystem the path targets.
            let label = crate::t!("cockpit-spawn-card-directory-on", host = host);
            let input = self.render_remote_dir_input(appearance).unwrap_or_else(|| {
                // Fallback for the (unit-test-only) case where no editor exists.
                Container::new(
                    Text::new_inline(crate::t!("cockpit-spawn-card-dir-host-home", host = host), family, 12.)
                        .with_color(muted)
                        .finish(),
                )
                .finish()
            });
            // "Browse…" opens the host's SFTP browser in pick mode (#105) — a
            // native folder picker can't reach a remote FS, so the visual browser
            // (with its MC-style select bar) fills the gap next to the text field.
            let browse = self.chip(
                "dir-browse",
                crate::t!("cockpit-spawn-card-browse"),
                false,
                SpawnCardAction::BrowseRemoteDir,
                appearance,
            );
            col = col.with_child(self.row(&label, vec![input, browse], appearance));
        } else {
            let mut dir_chips = vec![self.chip(
                "dir-pick",
                dir_display
                    .clone()
                    .unwrap_or_else(|| crate::t!("cockpit-spawn-card-choose-folder")),
                dir_display.is_some(),
                SpawnCardAction::OpenDirectoryPicker,
                appearance,
            )];
            if dir_display.is_some() {
                dir_chips.push(self.chip(
                    "dir-default",
                    crate::t!("cockpit-spawn-card-dir-default"),
                    false,
                    SpawnCardAction::ClearDirectory,
                    appearance,
                ));
            }
            col = col.with_child(self.row(&crate::t!("cockpit-spawn-card-directory"), dir_chips, appearance));
        }

        // Summary line.
        col = col.with_child(
            Container::new(
                Text::new_inline(crate::t!("cockpit-spawn-card-launching", summary = self.summary(app)), family, 12.)
                    .with_color(muted)
                    .finish(),
            )
            .with_margin_bottom(16.)
            .finish(),
        );

        // Confirm + cancel. Confirm renders inert (dimmed, no click handler) when
        // no supported agent CLI is installed — there is nothing it could launch.
        let can_launch = self.any_agent_installed() && self.selected_remote_cwd(app).is_ok();
        let confirm: Box<dyn Element> = if can_launch {
            self.chip(
                "confirm",
                crate::t!("cockpit-spawn-card-launch", agent = self.agent.display_name()),
                true,
                SpawnCardAction::Confirm,
                appearance,
            )
        } else {
            Container::new(
                Text::new_inline(crate::t!("cockpit-spawn-card-launch-plain"), family, 13.)
                    .with_color(muted)
                    .finish(),
            )
            .with_horizontal_padding(10.)
            .with_vertical_padding(5.)
            .with_background(theme.surface_2())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish()
        };
        let cancel = self.chip(
            "cancel",
            crate::t!("cockpit-spawn-card-cancel"),
            false,
            SpawnCardAction::Close,
            appearance,
        );
        col = col.with_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(confirm)
                .with_child(Container::new(cancel).with_margin_left(8.).finish())
                .finish(),
        );

        // The card chrome (padding · background · radius · border · shadow) is
        // supplied by the shared [`modal_frame::modal_card`] in `render`; here we
        // return just the inner column.
        col.finish()
    }
}

impl Entity for SpawnCard {
    type Event = SpawnCardEvent;
}

impl View for SpawnCard {
    fn ui_name() -> &'static str {
        "SpawnCard"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        // The one shared modal card + scrim. The Spawn-Karte holds unsaved launch
        // config, so a stray backdrop click must never discard it — no
        // click-outside dismiss (Esc / Cancel / ✕ close it); this is the "modals
        // with unsaved input" arm of the unified dismiss policy.
        let card = modal_frame::modal_card(self.render_card(app), MODAL_WIDTH, appearance);
        modal_frame::modal_overlay(card, None::<SpawnCardAction>, app)
    }
}

impl TypedActionView for SpawnCard {
    type Action = SpawnCardAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SpawnCardAction::SetAgent(agent) => {
                if self.agent != *agent {
                    self.agent = *agent;
                    // Model and effort options are provider-specific.
                    self.model = models_for(*agent)
                        .first()
                        .copied()
                        .unwrap_or("opus")
                        .to_string();
                    self.effort = default_effort_for(*agent).to_string();
                    // Account indices are provider-specific; fall back to freest.
                    self.account = AccountChoice::Freest;
                    ctx.notify();
                }
            }
            SpawnCardAction::SetModel(m) => {
                self.model = m.clone();
                ctx.notify();
            }
            SpawnCardAction::SetEffort(e) => {
                self.effort = e.clone();
                ctx.notify();
            }
            SpawnCardAction::ToggleAccountList => {
                self.show_accounts = !self.show_accounts;
                ctx.notify();
            }
            SpawnCardAction::SetAccountFreest => {
                self.account = AccountChoice::Freest;
                // Chosen — fold back to the calm line. Leaving the list open
                // after a pick would keep asking a question already answered.
                self.show_accounts = false;
                ctx.notify();
            }
            SpawnCardAction::SetAccount(i) => {
                self.account = AccountChoice::Specific(*i);
                self.show_accounts = false;
                ctx.notify();
            }
            SpawnCardAction::SetHostLocal => {
                self.host = HostChoice::Local;
                ctx.notify();
            }
            SpawnCardAction::SetHost(i) => {
                self.host = HostChoice::Remote(*i);
                ctx.notify();
            }
            SpawnCardAction::OpenDirectoryPicker => {
                // Same pattern as the session-config modal: the picker callback
                // dispatches a typed action carrying the chosen path back to this
                // view, which sets `project` in `DirectorySelected` below.
                ctx.open_file_picker(
                    |result, ctx| {
                        if let Some(path_result) =
                            result.map(|paths| paths.into_iter().next()).transpose()
                        {
                            ctx.dispatch_typed_action(&SpawnCardAction::DirectorySelected(
                                path_result,
                            ));
                        }
                    },
                    FilePickerConfiguration::new().folders_only(),
                );
            }
            SpawnCardAction::DirectorySelected(result) => match result {
                Ok(path) => {
                    self.project = Some(PathBuf::from(path));
                    ctx.notify();
                }
                Err(err) => {
                    log::warn!("Spawn card directory picker error: {err}");
                }
            },
            SpawnCardAction::ClearDirectory => {
                self.project = None;
                ctx.notify();
            }
            SpawnCardAction::BrowseRemoteDir => {
                // Hand off to the workspace: open the host's SFTP browser in pick
                // mode, seeded at the currently-typed absolute path if any. The
                // card is hidden meanwhile (its selections persist) and the chosen
                // dir returns via `WorkspaceAction::RemoteSpawnDirPicked` (#105).
                if let Some(node_id) = self.resolved_node_id() {
                    let start_path = self.remote_dir_editor.as_ref().and_then(|editor| {
                        let raw = editor.as_ref(ctx).buffer_text(ctx);
                        remote_cwd_from_input(self.host, &raw).ok().flatten()
                    });
                    ctx.emit(SpawnCardEvent::BrowseRemoteDir { node_id, start_path });
                }
            }
            SpawnCardAction::Confirm => {
                // Guard here too (not just in the chip's on_click), so the
                // "enter" keybinding can't launch an uninstalled CLI either:
                // `launch_payload` is `None` when nothing is installed.
                let remote_input = self
                    .remote_dir_editor
                    .as_ref()
                    .map(|editor| editor.as_ref(ctx).buffer_text(ctx));
                if let Some(launch) = self.launch_payload_for_remote_input(remote_input.as_deref())
                {
                    ctx.emit(launch);
                }
            }
            SpawnCardAction::Close => {
                ctx.emit(SpawnCardEvent::Close);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum SpawnCardEvent {
    Close,
    /// "Browse…" on the remote directory row: the workspace opens the host's
    /// SFTP browser in pick mode and returns the chosen dir via
    /// `WorkspaceAction::RemoteSpawnDirPicked` (#105). `start_path` is the
    /// currently-typed path (if absolute) so the browser opens there.
    BrowseRemoteDir {
        node_id: String,
        start_path: Option<PathBuf>,
    },
    Launch {
        agent: CLIAgent,
        config_dir: Option<PathBuf>,
        cwd: Option<PathBuf>,
        node_id: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        /// Task prompt to prefill into the launched agent's input, if this launch
        /// came from a contextual "run this task" flow.
        prompt: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub enum SpawnCardAction {
    SetAgent(CLIAgent),
    SetModel(String),
    SetEffort(String),
    SetAccountFreest,
    SetAccount(usize),
    /// Unfold (or fold) the account list behind the auto line.
    ToggleAccountList,
    SetHostLocal,
    SetHost(usize),
    /// Open the native folder picker to choose the launch directory (local host).
    OpenDirectoryPicker,
    /// Result delivered from the folder picker (dispatched from its callback).
    DirectorySelected(Result<String, FilePickerError>),
    /// Reset the launch directory to the default (agent's home / cwd).
    ClearDirectory,
    /// Open the remote host's SFTP browser to pick the launch directory (#105).
    BrowseRemoteDir,
    Confirm,
    Close,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(id: &str, name: &str) -> HostOption {
        HostOption {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    /// A stable id must route to the matching node — not the *first* node that
    /// happens to share the display label. This is the regression the Codex
    /// review flagged: clicking `+` on a later same-named host scoped the launch
    /// to the wrong remote.
    #[test]
    fn scoped_id_resolves_past_same_named_hosts() {
        let hosts = vec![
            host("id-a", "devbox"),
            host("id-b", "devbox"), // same label, different node
        ];
        // Given the second node's id, we must land on index 1 despite the shared
        // "devbox" label that a name-only match would resolve to index 0.
        assert_eq!(
            resolve_scoped_host(&hosts, Some("id-b"), Some("devbox")),
            HostChoice::Remote(1),
        );
        assert_eq!(
            resolve_scoped_host(&hosts, Some("id-a"), Some("devbox")),
            HostChoice::Remote(0),
        );
    }

    /// With no id (e.g. a source that lacks a stable id), fall back to matching
    /// by name — the first same-named node is the best we can do.
    #[test]
    fn name_fallback_when_no_id() {
        let hosts = vec![host("id-a", "alpha"), host("id-b", "beta")];
        assert_eq!(
            resolve_scoped_host(&hosts, None, Some("beta")),
            HostChoice::Remote(1),
        );
    }

    /// Local scoping (no id, no name) stays local — the unscoped / local case.
    #[test]
    fn no_scope_stays_local() {
        let hosts = vec![host("id-a", "alpha")];
        assert_eq!(resolve_scoped_host(&hosts, None, None), HostChoice::Local);
    }

    /// A stale/unknown id does not silently fall back to a name match (which
    /// could route to the wrong host — the very bug we are fixing); it stays
    /// local, the safe default.
    #[test]
    fn unknown_id_does_not_fall_back_to_name() {
        let hosts = vec![host("id-a", "devbox"), host("id-b", "devbox")];
        assert_eq!(
            resolve_scoped_host(&hosts, Some("id-missing"), Some("devbox")),
            HostChoice::Local,
        );
    }

    /// End-to-end contract at the spawn-card boundary (Codex review regression):
    /// the Conductor scopes by the *daemon* `HostId`, which the workspace
    /// translates to the SSH `node.id` before it reaches `resolve_scoped_host`.
    ///
    /// * A successfully translated daemon id arrives here already as the SSH
    ///   node id, so it resolves to the matching remote node — past a same-named
    ///   sibling (a name-only match would have picked the wrong one).
    /// * An untranslatable daemon id arrives as `scoped_id = None` with the host
    ///   name still set, so resolution falls back to name — a remote node, NOT
    ///   Local.
    #[test]
    fn daemon_scoped_open_resolves_translated_node_and_falls_back_to_name() {
        // `hosts` is keyed by SSH node.id; two nodes share the "devbox" label.
        let hosts = vec![host("node-1", "devbox"), host("node-2", "devbox")];

        // Translation succeeded: the workspace passes SSH node id "node-2"
        // (translated from the second host's daemon id). Must resolve to it,
        // not the first same-named node.
        assert_eq!(
            resolve_scoped_host(&hosts, Some("node-2"), Some("devbox")),
            HostChoice::Remote(1),
        );

        // Translation failed (daemon dropped): scoped id is None but the name
        // survives. Resolution falls back to name — a remote node, never Local.
        assert_eq!(
            resolve_scoped_host(&hosts, None, Some("devbox")),
            HostChoice::Remote(0),
        );
    }

    fn provider(installed: bool) -> ProviderOptions {
        ProviderOptions {
            installed,
            ..Default::default()
        }
    }

    /// Codex review regression: when neither Claude nor Codex is installed,
    /// there must be no launchable agent — a phantom chip previously let
    /// Confirm emit a `Launch` for a binary that isn't there.
    #[test]
    fn installed_agents_empty_when_none_installed() {
        let cfg = SpawnCardConfig {
            claude: provider(false),
            codex: provider(false),
            ..Default::default()
        };
        assert_eq!(installed_agents(&cfg), Vec::new());
    }

    /// Exactly one installed provider yields exactly that agent — the normal
    /// single-CLI case must keep working unchanged.
    #[test]
    fn installed_agents_only_lists_installed_provider() {
        let claude_only = SpawnCardConfig {
            claude: provider(true),
            codex: provider(false),
            ..Default::default()
        };
        assert_eq!(installed_agents(&claude_only), vec![CLIAgent::Claude]);

        let codex_only = SpawnCardConfig {
            claude: provider(false),
            codex: provider(true),
            ..Default::default()
        };
        assert_eq!(installed_agents(&codex_only), vec![CLIAgent::Codex]);
    }

    /// Both installed: both agents are offered, in the stable Claude-then-Codex
    /// order the row renders.
    #[test]
    fn installed_agents_lists_both_when_both_installed() {
        let cfg = SpawnCardConfig {
            claude: provider(true),
            codex: provider(true),
            ..Default::default()
        };
        assert_eq!(
            installed_agents(&cfg),
            vec![CLIAgent::Claude, CLIAgent::Codex]
        );
    }

    /// Confirm emits a `Launch` that carries the current selection verbatim: the
    /// chosen agent, model, effort and project cwd, and — for a local launch —
    /// the resolved account config dir with no remote node. Positive counterpart
    /// to the "nothing installed ⇒ no launch" guard.
    #[test]
    fn confirm_payload_carries_local_selection() {
        let card = SpawnCard {
            cfg: SpawnCardConfig {
                claude: provider(true),
                ..Default::default()
            },
            agent: CLIAgent::Claude,
            model: "sonnet".to_string(),
            effort: "low".to_string(),
            account: AccountChoice::Freest,
            show_accounts: false,
            host: HostChoice::Local,
            project: Some(PathBuf::from("/home/dev/projects/zaplex")),
            prompt: None,
            // The pure tests build the card without a `ViewContext`, so there is
            // no editor view to construct — remote-dir prefill/read is exercised
            // at the Confirm site, not here.
            remote_dir_editor: None,
            chip_states: Default::default(),
            close_button: None,
        };

        match card
            .launch_payload()
            .expect("an installed agent must yield a Launch on Confirm")
        {
            SpawnCardEvent::Launch {
                agent,
                config_dir,
                cwd,
                node_id,
                model,
                effort,
                ..
            } => {
                assert_eq!(agent, CLIAgent::Claude);
                assert_eq!(model.as_deref(), Some("sonnet"));
                assert_eq!(
                    effort, None,
                    "Claude exposes only its real CLI-default effort"
                );
                assert_eq!(cwd, Some(PathBuf::from("/home/dev/projects/zaplex")));
                assert_eq!(node_id, None, "a local launch has no remote node");
                // Freest with no configured freest_dir means the default login.
                assert_eq!(config_dir, None);
            }
            _ => panic!("Confirm must emit Launch"),
        }
    }

    /// A remote-scoped launch routes to the selected host's stable node id and,
    /// because account config dirs are local paths, carries no config dir.
    #[test]
    fn confirm_payload_routes_remote_launch_to_node_id() {
        let card = SpawnCard {
            cfg: SpawnCardConfig {
                claude: provider(true),
                hosts: vec![host("node-7", "devbox")],
                ..Default::default()
            },
            agent: CLIAgent::Claude,
            model: "opus".to_string(),
            effort: "high".to_string(),
            account: AccountChoice::Freest,
            show_accounts: false,
            host: HostChoice::Remote(0),
            project: None,
            prompt: None,
            // The pure tests build the card without a `ViewContext`, so there is
            // no editor view to construct — remote-dir prefill/read is exercised
            // at the Confirm site, not here.
            remote_dir_editor: None,
            chip_states: Default::default(),
            close_button: None,
        };

        match card
            .launch_payload()
            .expect("an installed agent must yield a Launch on Confirm")
        {
            SpawnCardEvent::Launch {
                node_id,
                config_dir,
                ..
            } => {
                assert_eq!(node_id.as_deref(), Some("node-7"));
                assert_eq!(
                    config_dir, None,
                    "remote launches use the host's own account"
                );
            }
            _ => panic!("Confirm must emit Launch"),
        }
    }

    /// Confirm is inert when nothing is installed — no phantom launch of a
    /// missing binary (the guard the pane's Confirm relies on).
    #[test]
    fn confirm_payload_none_when_nothing_installed() {
        let card = SpawnCard {
            cfg: SpawnCardConfig {
                claude: provider(false),
                codex: provider(false),
                ..Default::default()
            },
            agent: CLIAgent::Claude,
            model: "opus".to_string(),
            effort: "high".to_string(),
            account: AccountChoice::Freest,
            show_accounts: false,
            host: HostChoice::Local,
            project: None,
            prompt: None,
            // The pure tests build the card without a `ViewContext`, so there is
            // no editor view to construct — remote-dir prefill/read is exercised
            // at the Confirm site, not here.
            remote_dir_editor: None,
            chip_states: Default::default(),
            close_button: None,
        };
        assert!(card.launch_payload().is_none());
    }

    /// The remote-dir text input maps to the launch cwd: a blank field means the
    /// host's home (`None`); a typed absolute path becomes the cwd, trimmed so
    /// stray whitespace never produces a bogus path. A local host never uses the
    /// typed field. Pure — no editor/ctx needed (the Confirm site reads the
    /// editor into `raw` and delegates the mapping here).
    #[test]
    fn relative_remote_path_is_rejected() {
        let remote = HostChoice::Remote(0);
        assert_eq!(
            remote_cwd_from_input(remote, "srv/app"),
            Err(RemoteCwdError::RelativePath),
        );
        assert_eq!(
            remote_cwd_from_input(remote, "../x"),
            Err(RemoteCwdError::RelativePath),
        );
    }

    #[test]
    fn empty_path_maps_explicitly_home() {
        let remote = HostChoice::Remote(0);
        assert_eq!(remote_cwd_from_input(remote, ""), Ok(None));
        assert_eq!(remote_cwd_from_input(remote, "   "), Ok(None));
        assert_eq!(
            remote_cwd_from_input(remote, "  /srv/app  "),
            Ok(Some(PathBuf::from("/srv/app"))),
        );
        // A local host never uses the typed field, regardless of input.
        assert_eq!(
            remote_cwd_from_input(HostChoice::Local, "/srv/app"),
            Ok(None)
        );
    }

    #[test]
    fn claude_exposes_only_cli_default() {
        assert_eq!(effort_options_for(CLIAgent::Claude), &["CLI default"]);
        assert_eq!(default_effort_for(CLIAgent::Claude), "CLI default");
        assert_eq!(
            effort_options_for(CLIAgent::Codex),
            &["low", "medium", "high"]
        );
        assert_eq!(default_effort_for(CLIAgent::Codex), "high");
    }

    #[test]
    fn effort_payload_matches_cli_capability() {
        let mut card = SpawnCard {
            cfg: SpawnCardConfig {
                claude: provider(true),
                codex: provider(true),
                ..Default::default()
            },
            agent: CLIAgent::Codex,
            model: "gpt-5-codex".to_string(),
            effort: "high".to_string(),
            account: AccountChoice::Freest,
            show_accounts: false,
            host: HostChoice::Local,
            project: None,
            prompt: None,
            remote_dir_editor: None,
            chip_states: Default::default(),
            close_button: None,
        };
        let SpawnCardEvent::Launch { effort, .. } =
            card.launch_payload().expect("Codex is installed")
        else {
            panic!("expected launch payload");
        };
        assert_eq!(effort.as_deref(), Some("high"));

        card.agent = CLIAgent::Claude;
        card.effort = default_effort_for(CLIAgent::Claude).to_string();
        let SpawnCardEvent::Launch { effort, .. } =
            card.launch_payload().expect("Claude is installed")
        else {
            panic!("expected launch payload");
        };
        assert_eq!(effort, None);
    }

    #[test]
    fn relative_remote_path_prevents_launch_event() {
        let card = SpawnCard {
            cfg: SpawnCardConfig {
                codex: provider(true),
                hosts: vec![host("node-7", "devbox")],
                ..Default::default()
            },
            agent: CLIAgent::Codex,
            model: "gpt-5-codex".to_string(),
            effort: "high".to_string(),
            account: AccountChoice::Freest,
            show_accounts: false,
            host: HostChoice::Remote(0),
            project: None,
            prompt: None,
            remote_dir_editor: None,
            chip_states: Default::default(),
            close_button: None,
        };

        assert!(card
            .launch_payload_for_remote_input(Some("srv/app"))
            .is_none());
        let SpawnCardEvent::Launch { cwd, .. } = card
            .launch_payload_for_remote_input(Some("/srv/app"))
            .expect("absolute remote cwd is launchable")
        else {
            panic!("expected launch payload");
        };
        assert_eq!(cwd, Some(PathBuf::from("/srv/app")));
    }
}
