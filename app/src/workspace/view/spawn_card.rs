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
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use crate::ai::subscription_agent::ModelCapability;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::terminal::CLIAgent;
use crate::ui_components::modal_frame;
use crate::view_components::action_button::ActionButton;

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

#[derive(Clone, Debug)]
struct RemoteAccountOption {
    route: remote_server::proto::AgentLaunchRoute,
    label: String,
    email: Option<String>,
    capacity_5h: f64,
    capacity_week: f64,
    capacity_known: bool,
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
    models: Vec<ModelCapability>,
    model_discovery: ModelDiscoveryState,
    model_discovery_generation: u64,
    remote_accounts: Vec<RemoteAccountOption>,
    remote_account_discovery: RemoteAccountDiscoveryState,
    remote_account_generation: u64,
    remote_account_node_id: Option<String>,
}

#[derive(Clone, Debug, Default)]
enum ModelDiscoveryState {
    #[default]
    NotRequested,
    Loading,
    Ready,
    Error(String),
}

#[derive(Clone, Debug, Default)]
enum RemoteAccountDiscoveryState {
    #[default]
    NotRequested,
    Loading,
    Ready {
        auto_routing_available: bool,
    },
    Error(String),
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
    /// Antigravity has no Cockpit account-provider model yet. Installation is
    /// tracked separately so the card can offer an honest accountless `agy`
    /// launch without fabricating provider or subscription metadata.
    pub antigravity_installed: bool,
    /// Grok has no Cockpit account-provider model yet, so it follows the same
    /// accountless launch path and lets the CLI resolve its own authentication.
    pub grok_installed: bool,
    pub hosts: Vec<HostOption>,
    /// Pre-scoped host **id** in the same id space as [`HostOption::id`] — the
    /// SSH `node.id`. The Conductor scopes by the Agent-inventory's *daemon*
    /// `HostId`, so the workspace translates that to the hosting SSH node before
    /// filling this field (see `WorkspaceView::translate_scoped_daemon_host`);
    /// the two id spaces must not be compared directly. `None` = local, an
    /// unscoped launch, or a remote daemon that could not be translated to a
    /// live SSH node. This is the authoritative scoping key: same-named hosts
    /// are disambiguated by id, so when present it resolves the scoped host before
    /// [`Self::scoped_host_name`] is consulted.
    pub scoped_host_id: Option<String>,
    /// Pre-scoped host **name/label** (from a Conductor host/project-header `+`);
    /// `None` = local or no safe preselection. Used as the resolution fallback
    /// only when no stable identity was supplied; an untranslatable daemon scope
    /// clears this label at the workspace boundary rather than risking a
    /// different same-named host.
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

fn agent_is_installed(cfg: &SpawnCardConfig, agent: CLIAgent) -> bool {
    match agent {
        CLIAgent::Claude => cfg.claude.installed,
        CLIAgent::Codex => cfg.codex.installed,
        CLIAgent::Antigravity => cfg.antigravity_installed,
        CLIAgent::Grok => cfg.grok_installed,
        CLIAgent::Gemini
        | CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::OpenCode
        | CLIAgent::Copilot
        | CLIAgent::Pi
        | CLIAgent::Auggie
        | CLIAgent::CursorCli
        | CLIAgent::Goose
        | CLIAgent::DeepSeek
        | CLIAgent::Unknown => false,
    }
}

/// Which agents are actually launchable, per install-detection in `cfg`. Pure
/// helper (no view state) so the "neither installed" case is unit-testable:
/// an empty result means the card must not offer a launchable agent chip and
/// Confirm must stay disabled — there is nothing installed to run.
fn installed_agents(cfg: &SpawnCardConfig) -> Vec<CLIAgent> {
    [
        CLIAgent::Claude,
        CLIAgent::Codex,
        CLIAgent::Antigravity,
        CLIAgent::Grok,
    ]
    .into_iter()
    .filter(|agent| agent_is_installed(cfg, *agent))
    .collect()
}

fn unique_default(models: &[ModelCapability]) -> Option<&ModelCapability> {
    let mut defaults = models.iter().filter(|model| model.is_default);
    let default = defaults.next()?;
    defaults.next().is_none().then_some(default)
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
            editor
                .set_placeholder_text(crate::t!("cockpit-spawn-card-remote-dir-placeholder"), ctx);
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

        let close_button = ctx.add_view(|_ctx| modal_frame::close_button(SpawnCardAction::Close));

        SpawnCard {
            cfg: SpawnCardConfig::default(),
            agent: CLIAgent::Claude,
            model: String::new(),
            effort: String::new(),
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
        // so the opener's intent isn't silently changed. Otherwise use the first
        // installed agent in the card's stable order.
        let requested = cfg
            .default_agent
            .filter(|agent| agent.is_available_for_new_launch())
            .filter(|agent| agent_is_installed(&cfg, *agent));
        self.agent = requested
            .or_else(|| installed_agents(&cfg).first().copied())
            .unwrap_or(CLIAgent::Claude);
        self.model.clear();
        self.effort.clear();
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

        self.request_model_discovery(ctx);
        self.request_remote_account_discovery(ctx);

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
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let main = theme.main_text_color(theme.background()).into_solid();
        let faint = theme
            .sub_text_color(theme.background())
            .with_opacity(55)
            .into_solid();
        let Some(opts) = self.provider_options() else {
            return vec![Container::new(
                Text::new_inline(
                    crate::t!("cockpit-spawn-card-cli-default-login"),
                    family,
                    12.,
                )
                .with_color(muted)
                .finish(),
            )
            .finish()];
        };

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

    fn remote_account_controls(&self, appearance: &Appearance) -> Vec<Box<dyn Element>> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let Some(options) = self.provider_options() else {
            return vec![Container::new(
                Text::new_inline(
                    crate::t!("cockpit-spawn-card-cli-default-login"),
                    family,
                    12.,
                )
                .with_color(muted)
                .finish(),
            )
            .finish()];
        };
        match &options.remote_account_discovery {
            RemoteAccountDiscoveryState::NotRequested | RemoteAccountDiscoveryState::Loading => {
                return vec![Container::new(
                    Text::new_inline("Discovering host accounts…", family, 12.)
                        .with_color(muted)
                        .finish(),
                )
                .finish()];
            }
            RemoteAccountDiscoveryState::Error(error) => {
                return vec![Container::new(
                    Text::new_inline(error.clone(), family, 12.)
                        .with_color(muted)
                        .finish(),
                )
                .finish()];
            }
            RemoteAccountDiscoveryState::Ready { .. } => {}
        }

        let selected = self.selected_remote_account();
        if !self.show_accounts {
            let label = selected
                .map(|account| account.label.clone())
                .unwrap_or_else(|| "Select an account".to_string());
            return vec![
                Container::new(
                    Text::new_inline(label, family, 12.)
                        .with_color(muted)
                        .finish(),
                )
                .finish(),
                self.chip(
                    "remote-acct-change",
                    crate::t!("cockpit-spawn-card-change"),
                    false,
                    SpawnCardAction::ToggleAccountList,
                    appearance,
                ),
            ];
        }

        let mut chips = Vec::new();
        if self.freest_remote_account().is_some() {
            chips.push(self.chip(
                "remote-acct-freest",
                crate::t!("cockpit-spawn-card-freest"),
                self.account == AccountChoice::Freest,
                SpawnCardAction::SetAccountFreest,
                appearance,
            ));
        }
        for (index, account) in options.remote_accounts.iter().enumerate() {
            let label = if account.capacity_known {
                format!(
                    "{} ({:.0}% free)",
                    account.label,
                    account.capacity_5h * 100.0
                )
            } else {
                account.label.clone()
            };
            chips.push(self.chip(
                &format!("remote-acct-{index}"),
                label,
                self.account == AccountChoice::Specific(index),
                SpawnCardAction::SetAccount(index),
                appearance,
            ));
        }
        chips
    }

    fn selected_remote_account(&self) -> Option<&RemoteAccountOption> {
        let options = self.provider_options()?;
        match self.account {
            AccountChoice::Freest => self.freest_remote_account(),
            AccountChoice::Specific(index) => options.remote_accounts.get(index),
        }
    }

    fn freest_remote_account(&self) -> Option<&RemoteAccountOption> {
        let options = self.provider_options()?;
        let RemoteAccountDiscoveryState::Ready {
            auto_routing_available: true,
        } = &options.remote_account_discovery
        else {
            return None;
        };
        options
            .remote_accounts
            .iter()
            .filter(|account| account.capacity_known)
            .max_by(|left, right| {
                left.capacity_5h
                    .total_cmp(&right.capacity_5h)
                    .then_with(|| left.capacity_week.total_cmp(&right.capacity_week))
            })
    }

    fn remote_account_is_ready(&self) -> bool {
        !matches!(self.host, HostChoice::Remote(_))
            || !matches!(self.agent, CLIAgent::Claude | CLIAgent::Codex)
            || self.selected_remote_account().is_some()
    }

    fn request_remote_account_discovery(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(node_id) = self.resolved_node_id() else {
            return;
        };
        let Some(options) = self.provider_options_mut() else {
            return;
        };
        options.remote_accounts.clear();
        options.remote_account_discovery = RemoteAccountDiscoveryState::Loading;
        options.remote_account_generation += 1;
        options.remote_account_node_id = Some(node_id.clone());
        let generation = options.remote_account_generation;
        ctx.emit(SpawnCardEvent::DiscoverRemoteAccounts {
            generation,
            agent: self.agent,
            node_id,
        });
        ctx.notify();
    }

    pub fn apply_remote_accounts(
        &mut self,
        agent: CLIAgent,
        node_id: &str,
        generation: u64,
        result: Result<remote_server::proto::AgentAccountInventory, String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let provider = match agent {
            CLIAgent::Claude => "claude",
            CLIAgent::Codex => "codex",
            CLIAgent::Gemini
            | CLIAgent::Amp
            | CLIAgent::Droid
            | CLIAgent::OpenCode
            | CLIAgent::Copilot
            | CLIAgent::Pi
            | CLIAgent::Auggie
            | CLIAgent::CursorCli
            | CLIAgent::Goose
            | CLIAgent::DeepSeek
            | CLIAgent::Antigravity
            | CLIAgent::Grok
            | CLIAgent::Unknown => return,
        };
        let options = match agent {
            CLIAgent::Claude => &mut self.cfg.claude,
            CLIAgent::Codex => &mut self.cfg.codex,
            CLIAgent::Gemini
            | CLIAgent::Amp
            | CLIAgent::Droid
            | CLIAgent::OpenCode
            | CLIAgent::Copilot
            | CLIAgent::Pi
            | CLIAgent::Auggie
            | CLIAgent::CursorCli
            | CLIAgent::Goose
            | CLIAgent::DeepSeek
            | CLIAgent::Antigravity
            | CLIAgent::Grok
            | CLIAgent::Unknown => return,
        };
        if options.remote_account_generation != generation
            || options.remote_account_node_id.as_deref() != Some(node_id)
        {
            return;
        }
        match result {
            Ok(inventory) => {
                if inventory.schema_version != 1 {
                    options.remote_accounts.clear();
                    options.remote_account_discovery = RemoteAccountDiscoveryState::Error(
                        "This host returned an unsupported AI-account inventory version."
                            .to_string(),
                    );
                    ctx.notify();
                    return;
                }
                let auto_routing_available = inventory.health == "loaded";
                options.remote_accounts = inventory
                    .accounts
                    .into_iter()
                    .filter(|account| {
                        account.provider == provider && !account.account_id.is_empty()
                    })
                    .map(|account| RemoteAccountOption {
                        route: remote_server::proto::AgentLaunchRoute {
                            schema_version: 1,
                            provider: account.provider,
                            account_id: account.account_id,
                        },
                        label: if account.plan_tier.is_empty() {
                            account.display_label
                        } else {
                            format!("{} · {}", account.display_label, account.plan_tier)
                        },
                        email: (!account.email.is_empty()).then_some(account.email),
                        capacity_5h: account.capacity_5h,
                        capacity_week: account.capacity_week,
                        capacity_known: account.capacity_known,
                    })
                    .collect();
                options.remote_account_discovery = RemoteAccountDiscoveryState::Ready {
                    auto_routing_available,
                };
            }
            Err(error) => {
                options.remote_accounts.clear();
                options.remote_account_discovery = RemoteAccountDiscoveryState::Error(error);
            }
        }
        ctx.notify();
    }

    fn provider_options(&self) -> Option<&ProviderOptions> {
        match self.agent {
            CLIAgent::Claude => Some(&self.cfg.claude),
            CLIAgent::Codex => Some(&self.cfg.codex),
            CLIAgent::Gemini
            | CLIAgent::Amp
            | CLIAgent::Droid
            | CLIAgent::OpenCode
            | CLIAgent::Copilot
            | CLIAgent::Pi
            | CLIAgent::Auggie
            | CLIAgent::CursorCli
            | CLIAgent::Goose
            | CLIAgent::DeepSeek
            | CLIAgent::Antigravity
            | CLIAgent::Grok
            | CLIAgent::Unknown => None,
        }
    }

    fn provider_options_mut(&mut self) -> Option<&mut ProviderOptions> {
        match self.agent {
            CLIAgent::Claude => Some(&mut self.cfg.claude),
            CLIAgent::Codex => Some(&mut self.cfg.codex),
            CLIAgent::Gemini
            | CLIAgent::Amp
            | CLIAgent::Droid
            | CLIAgent::OpenCode
            | CLIAgent::Copilot
            | CLIAgent::Pi
            | CLIAgent::Auggie
            | CLIAgent::CursorCli
            | CLIAgent::Goose
            | CLIAgent::DeepSeek
            | CLIAgent::Antigravity
            | CLIAgent::Grok
            | CLIAgent::Unknown => None,
        }
    }

    fn selected_model_capability(&self) -> Option<&ModelCapability> {
        self.provider_options()?
            .models
            .iter()
            .find(|model| model.id == self.model)
    }

    fn model_is_ready(&self) -> bool {
        match self.agent {
            CLIAgent::Claude | CLIAgent::Codex => {
                matches!(
                    self.provider_options()
                        .map(|options| &options.model_discovery),
                    Some(ModelDiscoveryState::Ready)
                ) && self.selected_model_capability().is_some()
            }
            CLIAgent::Antigravity | CLIAgent::Grok => true,
            CLIAgent::Gemini
            | CLIAgent::Amp
            | CLIAgent::Droid
            | CLIAgent::OpenCode
            | CLIAgent::Copilot
            | CLIAgent::Pi
            | CLIAgent::Auggie
            | CLIAgent::CursorCli
            | CLIAgent::Goose
            | CLIAgent::DeepSeek
            | CLIAgent::Unknown => false,
        }
    }

    fn request_model_discovery(&mut self, ctx: &mut ViewContext<Self>) {
        if !agent_is_installed(&self.cfg, self.agent) {
            self.model.clear();
            self.effort.clear();
            ctx.notify();
            return;
        }
        let Some(_) = self.provider_options() else {
            self.model.clear();
            self.effort.clear();
            ctx.notify();
            return;
        };
        let config_dir = self.resolved_config_dir();
        let node_id = self.resolved_node_id();
        let host_name = self.remote_host_name().unwrap_or("Local").to_string();
        let working_directory = if matches!(self.host, HostChoice::Remote(_)) {
            self.remote_dir_editor
                .as_ref()
                .and_then(|editor| {
                    let raw = editor.as_ref(ctx).buffer_text(ctx);
                    remote_cwd_from_input(self.host, &raw).ok().flatten()
                })
                .unwrap_or_else(|| PathBuf::from("."))
        } else {
            self.project
                .clone()
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."))
        };
        self.model.clear();
        self.effort.clear();
        let agent = self.agent;
        let options = self
            .provider_options_mut()
            .expect("Claude and Codex always have provider options");
        options.models.clear();
        options.model_discovery = ModelDiscoveryState::Loading;
        options.model_discovery_generation += 1;
        let generation = options.model_discovery_generation;
        ctx.emit(SpawnCardEvent::DiscoverModels {
            generation,
            agent,
            config_dir,
            node_id,
            host_name,
            working_directory,
        });
        ctx.notify();
    }

    pub fn apply_model_capabilities(
        &mut self,
        agent: CLIAgent,
        generation: u64,
        result: Result<Vec<ModelCapability>, String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.agent != agent {
            return;
        }
        let options = match agent {
            CLIAgent::Claude => &mut self.cfg.claude,
            CLIAgent::Codex => &mut self.cfg.codex,
            CLIAgent::Gemini
            | CLIAgent::Amp
            | CLIAgent::Droid
            | CLIAgent::OpenCode
            | CLIAgent::Copilot
            | CLIAgent::Pi
            | CLIAgent::Auggie
            | CLIAgent::CursorCli
            | CLIAgent::Goose
            | CLIAgent::DeepSeek
            | CLIAgent::Antigravity
            | CLIAgent::Grok
            | CLIAgent::Unknown => return,
        };
        if options.model_discovery_generation != generation {
            return;
        }
        match result {
            Ok(models) => {
                let default_id = unique_default(&models).map(|model| model.id.clone());
                let default_effort =
                    unique_default(&models).and_then(|model| model.default_effort.clone());
                options.models = models;
                options.model_discovery = ModelDiscoveryState::Ready;
                self.model = default_id.unwrap_or_default();
                self.effort = default_effort.unwrap_or_default();
            }
            Err(error) => {
                options.models.clear();
                options.model_discovery = ModelDiscoveryState::Error(error);
                self.model.clear();
                self.effort.clear();
            }
        }
        ctx.notify();
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
        let options = self.provider_options()?;
        match self.account {
            AccountChoice::Freest => options.freest_dir.clone(),
            AccountChoice::Specific(i) => options.accounts.get(i).map(|a| a.config_dir.clone()),
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
        (self.any_agent_installed() && self.model_is_ready() && self.remote_account_is_ready())
            .then(|| SpawnCardEvent::Launch {
                agent: self.agent,
                config_dir: self.resolved_config_dir(),
                agent_launch_route: matches!(self.host, HostChoice::Remote(_))
                    .then(|| {
                        self.selected_remote_account()
                            .map(|account| account.route.clone())
                    })
                    .flatten(),
                remote_account_email: matches!(self.host, HostChoice::Remote(_))
                    .then(|| {
                        self.selected_remote_account()
                            .and_then(|account| account.email.clone())
                    })
                    .flatten(),
                cwd: self.project.clone(),
                node_id: self.resolved_node_id(),
                model: (!self.model.is_empty()).then(|| self.model.clone()),
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

    fn context_label(context_window: u64) -> String {
        if context_window >= 1_000_000 {
            format!("{}M ctx", context_window / 1_000_000)
        } else {
            format!("{}k ctx", context_window / 1_000)
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
            HostChoice::Remote(_) => self
                .selected_remote_account()
                .map(|account| account.label.clone())
                .unwrap_or_else(|| "account not selected".to_string()),
            HostChoice::Local => match (self.provider_options(), self.account) {
                (Some(_), AccountChoice::Freest) => {
                    crate::t!("cockpit-spawn-card-sum-freest")
                }
                (Some(options), AccountChoice::Specific(i)) => options
                    .accounts
                    .get(i)
                    .map(|a| a.label.clone())
                    .unwrap_or_else(|| crate::t!("cockpit-spawn-card-sum-freest")),
                (None, _) => crate::t!("cockpit-spawn-card-cli-default-login"),
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
        let mut parts = vec![self.agent.display_name().to_string()];
        if self.model.is_empty() {
            parts.push(crate::t!("cockpit-spawn-card-cli-default"));
        } else {
            parts.push(self.model.clone());
            if !self.effort.is_empty() {
                parts.push(Self::cap(&self.effort));
            }
            if let Some(context_window) = self
                .selected_model_capability()
                .and_then(|model| model.context_window)
            {
                parts.push(Self::context_label(context_window));
            }
        }
        parts.extend([account, host, dir]);
        parts.join(" · ")
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

        // Agent row (only installed launchable CLIs).
        let available = installed_agents(&self.cfg);
        if available.is_empty() {
            // No supported CLI is installed: there is nothing to launch,
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
            col = col.with_child(self.row(
                &crate::t!("cockpit-spawn-card-agent"),
                agent_chips,
                appearance,
            ));
        }

        // Model row + a live context-window readout when the CLI contract is
        // known. Antigravity launches with its own default; the card does not
        // invent a curated model list or a context size.
        let mut model_chips: Vec<Box<dyn Element>> = match self.provider_options() {
            Some(options) => match &options.model_discovery {
                ModelDiscoveryState::NotRequested | ModelDiscoveryState::Loading => {
                    vec![Container::new(
                        Text::new_inline("Discovering models…", family, 13.)
                            .with_color(muted)
                            .finish(),
                    )
                    .finish()]
                }
                ModelDiscoveryState::Error(error) => vec![Container::new(
                    Text::new_inline(error.clone(), family, 12.)
                        .with_color(muted)
                        .finish(),
                )
                .finish()],
                ModelDiscoveryState::Ready => options
                    .models
                    .iter()
                    .map(|model| {
                        self.chip(
                            &format!("model-{}", model.id),
                            model.display_name.clone(),
                            self.model == model.id,
                            SpawnCardAction::SetModel(model.id.clone()),
                            appearance,
                        )
                    })
                    .collect(),
            },
            None => vec![Container::new(
                Text::new_inline(crate::t!("cockpit-spawn-card-cli-default"), family, 13.)
                    .with_color(muted)
                    .finish(),
            )
            .finish()],
        };
        if let Some(context_window) = self
            .selected_model_capability()
            .and_then(|model| model.context_window)
        {
            model_chips.push(
                Container::new(
                    Text::new_inline(Self::context_label(context_window), family, 12.)
                        .with_color(muted)
                        .finish(),
                )
                .with_margin_left(8.)
                .finish(),
            );
        }
        col = col.with_child(self.row(
            &crate::t!("cockpit-spawn-card-model"),
            model_chips,
            appearance,
        ));

        // Effort row.
        let effort_options = self
            .selected_model_capability()
            .map(|model| model.supported_efforts.as_slice())
            .unwrap_or_default();
        let effort_chips: Vec<Box<dyn Element>> = if effort_options.is_empty() {
            vec![Container::new(
                Text::new_inline(crate::t!("cockpit-spawn-card-cli-default"), family, 13.)
                    .with_color(muted)
                    .finish(),
            )
            .finish()]
        } else {
            effort_options
                .iter()
                .map(|effort| {
                    self.chip(
                        &format!("effort-{}", effort.id),
                        effort.display_name.clone(),
                        self.effort == effort.id,
                        SpawnCardAction::SetEffort(effort.id.clone()),
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

        // Account row — local config identities stay local; remote identities are
        // opaque daemon account ids discovered for the selected host.
        if matches!(self.host, HostChoice::Remote(_)) {
            col = col.with_child(self.row(
                &crate::t!("cockpit-spawn-card-account"),
                self.remote_account_controls(appearance),
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
        col = col.with_child(self.row(
            &crate::t!("cockpit-spawn-card-host"),
            host_chips,
            appearance,
        ));

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
                    Text::new_inline(
                        crate::t!("cockpit-spawn-card-dir-host-home", host = host),
                        family,
                        12.,
                    )
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
            col = col.with_child(self.row(
                &crate::t!("cockpit-spawn-card-directory"),
                dir_chips,
                appearance,
            ));
        }

        // Summary line.
        col = col.with_child(
            Container::new(
                Text::new_inline(
                    crate::t!("cockpit-spawn-card-launching", summary = self.summary(app)),
                    family,
                    12.,
                )
                .with_color(muted)
                .finish(),
            )
            .with_margin_bottom(16.)
            .finish(),
        );

        // Confirm + cancel. Confirm renders inert (dimmed, no click handler) when
        // no supported agent CLI is installed — there is nothing it could launch.
        let can_launch = self.any_agent_installed()
            && self.model_is_ready()
            && self.remote_account_is_ready()
            && self.selected_remote_cwd(app).is_ok();
        let confirm: Box<dyn Element> = if can_launch {
            self.chip(
                "confirm",
                crate::t!(
                    "cockpit-spawn-card-launch",
                    agent = self.agent.display_name()
                ),
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
        modal_frame::modal_overlay(
            card,
            modal_frame::unsaved_input_dismiss_action::<SpawnCardAction>(),
            app,
        )
    }
}

impl TypedActionView for SpawnCard {
    type Action = SpawnCardAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SpawnCardAction::SetAgent(agent) => {
                if self.agent != *agent {
                    self.agent = *agent;
                    // Account indices are provider-specific; fall back to freest.
                    self.account = AccountChoice::Freest;
                    self.request_model_discovery(ctx);
                    self.request_remote_account_discovery(ctx);
                }
            }
            SpawnCardAction::SetModel(m) => {
                self.model = m.clone();
                self.effort = self
                    .selected_model_capability()
                    .and_then(|model| model.default_effort.clone())
                    .unwrap_or_default();
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
                if matches!(self.host, HostChoice::Local) {
                    self.request_model_discovery(ctx);
                }
            }
            SpawnCardAction::SetAccount(i) => {
                self.account = AccountChoice::Specific(*i);
                self.show_accounts = false;
                if matches!(self.host, HostChoice::Local) {
                    self.request_model_discovery(ctx);
                }
            }
            SpawnCardAction::SetHostLocal => {
                if self.host != HostChoice::Local {
                    self.host = HostChoice::Local;
                    self.account = AccountChoice::Freest;
                    self.request_model_discovery(ctx);
                }
            }
            SpawnCardAction::SetHost(i) => {
                if self.host != HostChoice::Remote(*i) {
                    self.host = HostChoice::Remote(*i);
                    self.account = AccountChoice::Freest;
                    self.request_model_discovery(ctx);
                    self.request_remote_account_discovery(ctx);
                }
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
                    ctx.emit(SpawnCardEvent::BrowseRemoteDir {
                        node_id,
                        start_path,
                    });
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
    DiscoverRemoteAccounts {
        generation: u64,
        agent: CLIAgent,
        node_id: String,
    },
    DiscoverModels {
        generation: u64,
        agent: CLIAgent,
        config_dir: Option<PathBuf>,
        node_id: Option<String>,
        host_name: String,
        working_directory: PathBuf,
    },
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
        agent_launch_route: Option<remote_server::proto::AgentLaunchRoute>,
        remote_account_email: Option<String>,
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
#[path = "spawn_card_tests.rs"]
mod tests;
