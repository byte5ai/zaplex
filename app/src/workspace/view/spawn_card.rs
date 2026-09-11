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

pub(crate) mod bulk;
pub(crate) mod history;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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

use crate::ai::subscription_agent::{model_identity_label, ModelCapability};
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::terminal::CLIAgent;
use crate::ui_components::modal_frame;
use crate::view_components::action_button::ActionButton;

use self::bulk::{
    selected_account_ids, BulkLaunchLedger, BulkLaunchPlan, BulkLaunchPlanId, BulkLaunchTarget,
    BulkLaunchTargetId, LaunchAccountId, LaunchAccountTarget,
};
use self::history::{
    DirectoryValidation, DirectoryValidationRequest, DirectoryValidationState, FolderHistory,
    FolderHistoryHost, FolderNavigation,
};

const MODAL_WIDTH: f32 = 480.;

/// One account option offered in the card.
#[derive(Clone, Debug)]
pub struct AccountOption {
    /// Display name — already the user's alias where one is set (A1: the
    /// overrides layer replaced it before the snapshot existed).
    pub label: String,
    pub config_dir: PathBuf,
    /// The provider's default login is routed by absence of a pin. Its config
    /// root remains part of the stable UI identity, but must not be exported as
    /// a provider config environment override.
    pub is_default: bool,
    /// Binding-window utilisation, already formatted (`~` marks an estimate).
    pub heat_label: String,
    /// The same figure as a fraction, so the card can colour it by the one
    /// utilisation rule instead of parsing its own label back.
    pub heat: f64,
    /// Plan tier, when the provider told us.
    pub plan: Option<String>,
    pub provider: zaplex_cockpit::Provider,
}

fn account_config_pin(account: &AccountOption) -> Option<PathBuf> {
    (!account.is_default).then(|| account.config_dir.clone())
}

#[derive(Clone, Debug)]
struct RemoteAccountOption {
    route: remote_server::proto::AgentLaunchRoute,
    label: String,
    email: Option<String>,
    provider_account_id: Option<String>,
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
    model_discovery_target: Option<ModelDiscoveryTarget>,
    model_discovery_cli_version: Option<String>,
    model_catalogs: BTreeMap<LaunchAccountId, AccountModelCatalog>,
    model_discovery_pending: BTreeSet<LaunchAccountId>,
    remote_accounts: Vec<RemoteAccountOption>,
    remote_account_discovery: RemoteAccountDiscoveryState,
    remote_account_generation: u64,
    remote_account_node_id: Option<String>,
}

/// Exact routing identity for the account whose model list is currently
/// displayed. The generation rejects late responses; this identity also makes
/// ready state fail closed if a future UI transition forgets to invalidate it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelDiscoveryTarget {
    agent: CLIAgent,
    node_id: Option<String>,
    account_id: String,
    provider_account_id: Option<String>,
    config_dir: Option<PathBuf>,
    cli_version: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct DiscoveredModels {
    pub(crate) cli_version: String,
    pub(crate) models: Vec<ModelCapability>,
}

#[derive(Clone, Debug)]
struct AccountModelCatalog {
    target: ModelDiscoveryTarget,
    models: Vec<ModelCapability>,
}

#[derive(Clone, Debug, Default)]
enum ModelDiscoveryState {
    #[default]
    NotRequested,
    Loading,
    Ready,
    Error(ModelDiscoveryFailure),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModelDiscoveryFailure {
    HostUnavailable {
        host: String,
    },
    IncompatibleCli {
        agent: CLIAgent,
        host: String,
    },
    Failed {
        agent: CLIAgent,
        host: String,
        detail: String,
    },
}

impl ModelDiscoveryFailure {
    pub(crate) fn host_unavailable(host: impl Into<String>) -> Self {
        Self::HostUnavailable { host: host.into() }
    }

    pub(crate) fn classify(
        agent: CLIAgent,
        host: impl Into<String>,
        remote: bool,
        error: impl Into<String>,
    ) -> Self {
        let host = host.into();
        let detail = error.into();
        let normalized = detail.to_ascii_lowercase();
        if normalized.contains("does not support the required")
            || normalized.contains("incompatible cli")
            || normalized.contains("unsupported cli")
        {
            Self::IncompatibleCli { agent, host }
        } else if remote
            && (normalized.contains("timed out")
                || normalized.contains("connection refused")
                || normalized.contains("unreachable")
                || normalized.contains("could not resolve hostname")
                || normalized.contains("no route to host"))
        {
            Self::HostUnavailable { host }
        } else {
            Self::Failed {
                agent,
                host,
                detail,
            }
        }
    }

    fn label(&self) -> String {
        match self {
            Self::HostUnavailable { host } => {
                crate::t!("cockpit-spawn-card-model-host-unavailable", host = host)
            }
            Self::IncompatibleCli { agent, host } => crate::t!(
                "cockpit-spawn-card-model-cli-incompatible",
                agent = agent.display_name(),
                host = host
            ),
            Self::Failed {
                agent,
                host,
                detail,
            } => crate::t!(
                "cockpit-spawn-card-model-discovery-failed",
                agent = agent.display_name(),
                host = host,
                detail = detail
            ),
        }
    }
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
    /// True only for a currently connected daemon that negotiated the full
    /// managed-fleet contract. Registered/offline/old hosts remain ordinary.
    pub managed_fleet_available: bool,
}

fn host_identity_label(host: &HostOption) -> String {
    if host.name == host.id {
        host.name.clone()
    } else {
        format!("{} · ID {}", host.name, host.id)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ManagedLaunchMode {
    #[default]
    Ordinary,
    ManagedInteractive,
    ClaudeRemoteControl,
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
    /// the two id spaces must not be compared directly. `None` can mean local,
    /// an unscoped launch, or an untranslatable daemon; the latter is separated
    /// by [`Self::require_explicit_host`]. This is the authoritative scoping key:
    /// same-named hosts are disambiguated by id, so when present it resolves the
    /// scoped host before [`Self::scoped_host_name`] is consulted.
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
    /// silently changed to the default. If it is no longer installed, the card
    /// keeps that unavailable choice visible and requires an explicit switch.
    /// `None` = use the first installed CLI in the card's stable order.
    pub default_agent: Option<CLIAgent>,
    /// A contextual remote scope existed but could no longer be resolved to a
    /// registered SSH host. Keep the host selection empty instead of silently
    /// changing the launch to Local.
    pub require_explicit_host: bool,
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
    /// A requested host disappeared or became unresolvable. Launch remains
    /// blocked until the user explicitly selects Local or a registered host.
    Unselected,
    Local,
    /// A connected SSH host by index into [`SpawnCardConfig::hosts`].
    Remote(usize),
}

/// Resolve a pre-scoped Conductor host to a [`HostChoice`].
///
/// Prefers the stable `scoped_id`: two connected hosts can share a display
/// label, so matching by name alone picks the *first* same-named node and can
/// prep a launch on the wrong remote host. Only when no id is available do we
/// fall back to matching by `scoped_name`. An explicit scope that no longer
/// resolves remains unselected; only an actually unscoped launch defaults to
/// Local.
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
        return HostChoice::Unselected;
    }
    // Fallback: resolve by display label (only when no id was supplied).
    if scoped_id.is_none() {
        if let Some(name) = scoped_name {
            if let Some(pos) = hosts.iter().position(|h| h.name == name) {
                return HostChoice::Remote(pos);
            }
            return HostChoice::Unselected;
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
    managed_mode: ManagedLaunchMode,
    account: AccountChoice,
    /// Additional exact account identities selected for a multi-launch. Empty
    /// retains the calm automatic/single-account path.
    batch_accounts: BTreeSet<LaunchAccountId>,
    select_all_accounts: bool,
    /// Whether the account list is unfolded (X1). Collapsed by default: the
    /// router already chose, so the card states the answer instead of asking the
    /// question again on every launch.
    show_accounts: bool,
    host: HostChoice,
    project: Option<PathBuf>,
    folder_history: FolderHistory,
    folder_navigation: FolderNavigation,
    folder_history_open: bool,
    folder_validation: DirectoryValidationState,
    history_validation: BTreeMap<PathBuf, DirectoryValidation>,
    history_search_editor: Option<ViewHandle<EditorView>>,
    bulk_launch: Option<BulkLaunchLedger>,
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

fn remote_agents(cfg: &SpawnCardConfig) -> Vec<CLIAgent> {
    let mut agents = vec![CLIAgent::Claude, CLIAgent::Codex];
    agents.extend(
        [CLIAgent::Antigravity, CLIAgent::Grok]
            .into_iter()
            .filter(|agent| agent_is_installed(cfg, *agent)),
    );
    agents
}

fn initial_agent(cfg: &SpawnCardConfig) -> CLIAgent {
    cfg.default_agent
        .filter(|agent| agent.is_available_for_new_launch())
        .or_else(|| installed_agents(cfg).first().copied())
        .unwrap_or(CLIAgent::Claude)
}

fn unique_default(models: &[ModelCapability]) -> Option<&ModelCapability> {
    let mut defaults = models.iter().filter(|model| model.is_default);
    let default = defaults.next()?;
    defaults.next().is_none().then_some(default)
}

fn same_model_target_scope(left: &ModelDiscoveryTarget, right: &ModelDiscoveryTarget) -> bool {
    left.agent == right.agent
        && left.node_id == right.node_id
        && left.account_id == right.account_id
        && left.provider_account_id == right.provider_account_id
        && left.config_dir == right.config_dir
}

fn common_model_capabilities<'a>(
    catalogs: impl IntoIterator<Item = &'a AccountModelCatalog>,
) -> Vec<ModelCapability> {
    let mut catalogs = catalogs.into_iter();
    let Some(first) = catalogs.next() else {
        return Vec::new();
    };
    let mut common = first.models.clone();
    for catalog in catalogs {
        common.retain_mut(|candidate| {
            let Some(other) = catalog.models.iter().find(|model| model.id == candidate.id) else {
                return false;
            };
            candidate.is_default &= other.is_default;
            candidate.supported_efforts.retain(|effort| {
                other
                    .supported_efforts
                    .iter()
                    .any(|other_effort| other_effort.id == effort.id)
            });
            if candidate.default_effort.as_ref().is_some_and(|default| {
                !candidate
                    .supported_efforts
                    .iter()
                    .any(|effort| &effort.id == default)
            }) {
                candidate.default_effort = None;
            }
            if candidate.resolved_model != other.resolved_model {
                return false;
            }
            candidate.context_window = match (candidate.context_window, other.context_window) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (Some(_), None) | (None, Some(_)) | (None, None) => None,
            };
            true
        });
    }
    common
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
        ctx.subscribe_to_view(&remote_dir_editor, |card, _, event, ctx| {
            if matches!(event, EditorEvent::Edited(_)) {
                if let Some(path) = card.selected_directory(ctx) {
                    card.folder_navigation.select(path);
                    card.begin_selected_directory_validation(ctx);
                } else {
                    card.folder_validation.clear();
                }
                card.invalidate_bulk_plan();
                card.invalidate_model_for_target_change();
                ctx.notify();
            }
        });

        let history_search_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = SingleLineEditorOptions {
                text: TextOptions::ui_text(Some(13.), appearance),
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text(crate::t!("cockpit-spawn-card-filter-folders"), ctx);
            editor
        });
        ctx.subscribe_to_view(&history_search_editor, |_, _, event, ctx| {
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
            managed_mode: ManagedLaunchMode::Ordinary,
            account: AccountChoice::Freest,
            batch_accounts: BTreeSet::new(),
            select_all_accounts: false,
            show_accounts: false,
            host: HostChoice::Local,
            project: None,
            folder_history: FolderHistory::load(),
            folder_navigation: FolderNavigation::default(),
            folder_history_open: false,
            folder_validation: DirectoryValidationState::default(),
            history_validation: BTreeMap::new(),
            history_search_editor: Some(history_search_editor),
            bulk_launch: None,
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
        self.model.clear();
        self.effort.clear();
        self.managed_mode = ManagedLaunchMode::Ordinary;
        self.account = AccountChoice::Freest;
        self.batch_accounts.clear();
        self.select_all_accounts = false;
        self.bulk_launch = None;
        self.folder_history_open = false;
        self.history_validation.clear();
        // Pre-scope host from a Conductor host/project `+`, else local. Resolve
        // by stable id first so same-named hosts route to the right node.
        self.host = if cfg.require_explicit_host {
            HostChoice::Unselected
        } else {
            resolve_scoped_host(
                &cfg.hosts,
                cfg.scoped_host_id.as_deref(),
                cfg.scoped_host_name.as_deref(),
            )
        };
        // A remote daemon owns its own CLI installations. Do not make a remote
        // Claude/Codex launch depend on whether the same CLI is installed on the
        // client. Explicit requests remain selected so an unavailable target is
        // never silently replaced.
        self.agent = cfg.default_agent.unwrap_or_else(|| match self.host {
            HostChoice::Remote(_) => CLIAgent::Claude,
            HostChoice::Unselected | HostChoice::Local => initial_agent(&cfg),
        });
        // `self.project` is the LOCAL launch dir (native folder picker). For a
        // remote-scoped open the pre-scoped dir is a REMOTE path, which belongs in
        // the remote-dir editor (seeded below), NOT here — otherwise switching the
        // host to Local would launch locally into a remote-only path (Codex: host
        // switch dir leakage). So keep the local project empty for a remote host.
        self.project = match self.host {
            HostChoice::Unselected => None,
            HostChoice::Local => cfg.project.clone(),
            HostChoice::Remote(_) => None,
        };
        self.prompt = cfg.prompt.clone();
        self.cfg = cfg;

        if let Some(editor) = self.history_search_editor.clone() {
            editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text_with_base_buffer("", ctx);
            });
        }

        // Prefill the remote-dir input from the pre-scoped project (a Conductor
        // remote project node) so the common case is a single confirm; every other
        // open (local, or remote without a project) resets it to empty (blank =
        // host home). Reset on every open so a stale path can't leak across opens.
        if let Some(editor) = self.remote_dir_editor.clone() {
            let prefill = match (self.host, &self.cfg.project) {
                (HostChoice::Remote(_), Some(dir)) => dir.display().to_string(),
                (HostChoice::Unselected | HostChoice::Local, _) | (HostChoice::Remote(_), None) => {
                    String::new()
                }
            };
            editor.update(ctx, |ed, ctx| {
                ed.set_buffer_text_with_base_buffer(&prefill, ctx);
            });
        }

        self.folder_navigation.reset(self.selected_directory(ctx));
        self.begin_selected_directory_validation(ctx);

        match self.host {
            HostChoice::Unselected => self.invalidate_model_for_target_change(),
            HostChoice::Local => self.request_model_discovery(ctx),
            HostChoice::Remote(_) => self.request_remote_account_discovery(ctx),
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
            self.folder_navigation.select(path.to_path_buf());
            self.begin_selected_directory_validation(ctx);
            self.invalidate_bulk_plan();
            self.invalidate_model_for_target_change();
            self.request_model_discovery(ctx);
        }
    }

    fn history_host(&self) -> FolderHistoryHost {
        self.resolved_node_id()
            .and_then(FolderHistoryHost::remote)
            .unwrap_or(FolderHistoryHost::Local)
    }

    fn selected_directory(&self, app: &AppContext) -> Option<PathBuf> {
        match self.host {
            HostChoice::Unselected => None,
            HostChoice::Local => self.project.clone(),
            HostChoice::Remote(_) => self.remote_dir_editor.as_ref().and_then(|editor| {
                let raw = editor.as_ref(app).buffer_text(app);
                remote_cwd_from_input(self.host, &raw).ok().flatten()
            }),
        }
    }

    fn set_selected_directory(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        match self.host {
            HostChoice::Unselected => return,
            HostChoice::Local => self.project = Some(path.clone()),
            HostChoice::Remote(_) => {
                if let Some(editor) = self.remote_dir_editor.clone() {
                    let text = path.display().to_string();
                    editor.update(ctx, |editor, ctx| {
                        editor.set_buffer_text_with_base_buffer(&text, ctx);
                    });
                }
            }
        }
        self.folder_navigation.select(path);
        self.begin_selected_directory_validation(ctx);
        self.invalidate_bulk_plan();
        self.invalidate_model_for_target_change();
        self.request_model_discovery(ctx);
    }

    fn begin_selected_directory_validation(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(path) = self.selected_directory(ctx) else {
            self.folder_validation.clear();
            return;
        };
        let host = self.history_host();
        let request = self.folder_validation.begin(host, path.clone());
        self.history_validation
            .insert(path, DirectoryValidation::Checking);
        ctx.emit(SpawnCardEvent::ValidateDirectory(request));
    }

    pub fn apply_directory_validation(
        &mut self,
        request: &DirectoryValidationRequest,
        result: DirectoryValidation,
        ctx: &mut ViewContext<Self>,
    ) {
        self.history_validation.insert(request.path.clone(), result);
        self.folder_validation.apply(request, result);
        ctx.notify();
    }

    fn invalidate_bulk_plan(&mut self) {
        self.bulk_launch = None;
    }

    fn local_account_targets(&self) -> Vec<LaunchAccountTarget> {
        let Some(options) = self.provider_options() else {
            return Vec::new();
        };
        let all = options
            .accounts
            .iter()
            .map(|account| LaunchAccountTarget {
                id: LaunchAccountId::local(self.agent, &account.config_dir),
                label: account.label.clone(),
                config_dir: account_config_pin(account),
                account_email: None,
                provider_account_id: None,
                remote_route: None,
            })
            .collect::<Vec<_>>();
        let selected = selected_account_ids(&all, &self.batch_accounts, self.select_all_accounts);
        if !selected.is_empty() {
            return selected;
        }
        match self.account {
            AccountChoice::Freest => options
                .freest
                .as_ref()
                .map(|account| LaunchAccountTarget {
                    id: LaunchAccountId::local(self.agent, &account.config_dir),
                    label: account.label.clone(),
                    config_dir: account_config_pin(account),
                    account_email: None,
                    provider_account_id: None,
                    remote_route: None,
                })
                .map(|account| vec![account])
                .unwrap_or_else(|| {
                    options
                        .accounts
                        .is_empty()
                        .then(|| LaunchAccountTarget {
                            id: LaunchAccountId(format!(
                                "{}:local:default",
                                self.agent.to_serialized_name()
                            )),
                            label: crate::t!("cockpit-spawn-card-cli-default-login").to_string(),
                            config_dir: None,
                            account_email: None,
                            provider_account_id: None,
                            remote_route: None,
                        })
                        .into_iter()
                        .collect()
                }),
            AccountChoice::Specific(index) => all.get(index).cloned().into_iter().collect(),
        }
    }

    fn remote_account_targets(&self) -> Vec<LaunchAccountTarget> {
        let Some(options) = self.remote_options_for_selected_host() else {
            return Vec::new();
        };
        let all = options
            .remote_accounts
            .iter()
            .map(|account| LaunchAccountTarget {
                id: LaunchAccountId::remote(self.agent, &account.route.account_id),
                label: account.label.clone(),
                config_dir: None,
                account_email: account.email.clone(),
                provider_account_id: account.provider_account_id.clone(),
                remote_route: Some(account.route.clone()),
            })
            .collect::<Vec<_>>();
        let selected = selected_account_ids(&all, &self.batch_accounts, self.select_all_accounts);
        if !selected.is_empty() {
            return selected;
        }
        self.selected_remote_account()
            .map(|account| LaunchAccountTarget {
                id: LaunchAccountId::remote(self.agent, &account.route.account_id),
                label: account.label.clone(),
                config_dir: None,
                account_email: account.email.clone(),
                provider_account_id: account.provider_account_id.clone(),
                remote_route: Some(account.route.clone()),
            })
            .into_iter()
            .collect()
    }

    fn launch_accounts(&self) -> Vec<LaunchAccountTarget> {
        match self.agent {
            CLIAgent::Claude | CLIAgent::Codex => match self.host {
                HostChoice::Unselected => Vec::new(),
                HostChoice::Local => self.local_account_targets(),
                HostChoice::Remote(_) => self.remote_account_targets(),
            },
            CLIAgent::Antigravity | CLIAgent::Grok => vec![LaunchAccountTarget {
                id: LaunchAccountId(format!("{}:default", self.agent.to_serialized_name())),
                label: crate::t!("cockpit-spawn-card-cli-default-login").to_string(),
                config_dir: None,
                account_email: None,
                provider_account_id: None,
                remote_route: None,
            }],
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
            | CLIAgent::Unknown => Vec::new(),
        }
    }

    fn build_bulk_plan(&self, app: &AppContext) -> Option<BulkLaunchPlan> {
        if !self.selected_agent_is_available()
            || !self.host_is_ready()
            || !self.model_is_ready()
            || !self.account_is_ready()
            || !self.managed_mode_is_valid()
        {
            return None;
        }
        let cwd = self.selected_directory(app);
        if self.managed_mode != ManagedLaunchMode::Ordinary && cwd.is_none() {
            return None;
        }
        let node_id = self.resolved_node_id();
        let managed = self.managed_mode != ManagedLaunchMode::Ordinary;
        let targets = self
            .launch_accounts()
            .into_iter()
            .map(|account| BulkLaunchTarget {
                account,
                agent: self.agent,
                node_id: node_id.clone(),
                cwd: cwd.clone(),
                model: (!managed && !self.model.is_empty()).then(|| self.model.clone()),
                effort: (!managed && matches!(self.agent, CLIAgent::Codex))
                    .then(|| self.effort.clone()),
                prompt: (!managed).then(|| self.prompt.clone()).flatten(),
                managed_mode: self.managed_mode,
                managed_launch_id: managed.then(|| uuid::Uuid::new_v4().to_string()),
            });
        let plan = BulkLaunchPlan::new(targets);
        (!plan.targets.is_empty()).then_some(plan)
    }

    fn launch_attempt(&mut self, app: &AppContext) -> Option<SpawnCardEvent> {
        if self.bulk_launch.is_none() {
            let plan = self.build_bulk_plan(app)?;
            self.bulk_launch = Some(BulkLaunchLedger::new(plan));
        }
        let ledger = self.bulk_launch.as_ref()?;
        let targets = ledger.targets_for_attempt();
        (!targets.is_empty()).then_some(SpawnCardEvent::LaunchBatch {
            plan_id: ledger.plan.id,
            targets,
        })
    }

    pub fn apply_launch_result(
        &mut self,
        plan_id: BulkLaunchPlanId,
        target_id: &BulkLaunchTargetId,
        result: Result<String, String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let history_host = self.history_host();
        let Some(ledger) = self.bulk_launch.as_mut() else {
            return;
        };
        let already_recorded_history = ledger.any_succeeded();
        if !ledger.apply(plan_id, target_id, result) {
            return;
        }
        if !already_recorded_history && ledger.any_succeeded() {
            if let Some(path) = ledger
                .plan
                .targets
                .values()
                .next()
                .and_then(|target| target.cwd.as_deref())
            {
                if let Err(error) =
                    self.folder_history
                        .record_success(&history_host, path, chrono::Utc::now())
                {
                    log::error!("failed to record spawn-folder history: {error:#}");
                }
            }
        }
        ctx.notify();
    }

    pub fn mark_launch_in_flight(
        &mut self,
        plan_id: BulkLaunchPlanId,
        target_id: &BulkLaunchTargetId,
        launch_id: String,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let marked = self
            .bulk_launch
            .as_mut()
            .is_some_and(|ledger| ledger.mark_in_flight(plan_id, target_id, launch_id));
        if marked {
            ctx.notify();
        }
        marked
    }

    pub fn launch_batch_succeeded(&self, plan_id: BulkLaunchPlanId) -> bool {
        self.bulk_launch
            .as_ref()
            .is_some_and(|ledger| ledger.plan.id == plan_id && ledger.all_succeeded())
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
            let batch_count = if self.select_all_accounts {
                opts.accounts.len()
            } else {
                self.batch_accounts.len()
            };
            if batch_count > 1 {
                return vec![
                    Container::new(
                        Text::new_inline(
                            crate::t!("cockpit-spawn-card-account-count", count = batch_count),
                            family,
                            12.,
                        )
                        .with_color(main)
                        .finish(),
                    )
                    .finish(),
                    self.chip(
                        "acct-change",
                        crate::t!("cockpit-spawn-card-change"),
                        false,
                        SpawnCardAction::ToggleAccountList,
                        appearance,
                    ),
                ];
            }
            let (selected, auto) = match self.account {
                AccountChoice::Freest => match opts.freest.clone() {
                    Some(account) => (account, true),
                    None => {
                        // No accounts discovered means the verified CLI login
                        // is the only honest target. If accounts do exist but
                        // no routing pick was produced, require a selection.
                        let label = if opts.accounts.is_empty() {
                            crate::t!("cockpit-spawn-card-cli-default-login")
                        } else {
                            crate::t!("cockpit-spawn-card-select-account")
                        };
                        return vec![Container::new(
                            Text::new_inline(label, family, 12.)
                                .with_color(muted)
                                .finish(),
                        )
                        .finish()];
                    }
                },
                AccountChoice::Specific(i) => match opts.accounts.get(i).cloned() {
                    Some(account) => (account, false),
                    None => {
                        return vec![Container::new(
                            Text::new_inline(
                                crate::t!("cockpit-spawn-card-account-unavailable"),
                                family,
                                12.,
                            )
                            .with_color(muted)
                            .finish(),
                        )
                        .finish()]
                    }
                },
            };

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
        if opts.accounts.len() > 1 {
            chips.push(self.chip(
                "acct-all",
                crate::t!("cockpit-spawn-card-all-accounts"),
                self.select_all_accounts,
                SpawnCardAction::SetAllAccounts,
                appearance,
            ));
        }
        for (i, a) in opts.accounts.iter().enumerate() {
            let id = LaunchAccountId::local(self.agent, &a.config_dir);
            chips.push(self.chip(
                &format!("acct-{i}"),
                format!("{} ({})", a.label, a.heat_label),
                self.batch_accounts.contains(&id)
                    || (!self.select_all_accounts
                        && self.batch_accounts.is_empty()
                        && self.account == AccountChoice::Specific(i)),
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
                    Text::new_inline(
                        crate::t!("cockpit-spawn-card-discovering-accounts"),
                        family,
                        12.,
                    )
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
            let batch_count = if self.select_all_accounts {
                options.remote_accounts.len()
            } else {
                self.batch_accounts.len()
            };
            if batch_count > 1 {
                return vec![
                    Container::new(
                        Text::new_inline(
                            crate::t!("cockpit-spawn-card-account-count", count = batch_count),
                            family,
                            12.,
                        )
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
            let label = selected
                .map(|account| account.label.clone())
                .unwrap_or_else(|| crate::t!("cockpit-spawn-card-select-account"));
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
        if options.remote_accounts.len() > 1 {
            chips.push(self.chip(
                "remote-acct-all",
                crate::t!("cockpit-spawn-card-all-accounts"),
                self.select_all_accounts,
                SpawnCardAction::SetAllAccounts,
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
                self.batch_accounts.contains(&LaunchAccountId::remote(
                    self.agent,
                    &account.route.account_id,
                )) || (!self.select_all_accounts
                    && self.batch_accounts.is_empty()
                    && self.account == AccountChoice::Specific(index)),
                SpawnCardAction::SetAccount(index),
                appearance,
            ));
        }
        chips
    }

    fn selected_remote_account(&self) -> Option<&RemoteAccountOption> {
        match self.account {
            AccountChoice::Freest => self.freest_remote_account(),
            AccountChoice::Specific(index) => self
                .remote_options_for_selected_host()?
                .remote_accounts
                .get(index),
        }
    }

    fn freest_remote_account(&self) -> Option<&RemoteAccountOption> {
        let options = self.remote_options_for_selected_host()?;
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
                    .min(left.capacity_week)
                    .total_cmp(&right.capacity_5h.min(right.capacity_week))
                    .then_with(|| left.capacity_5h.total_cmp(&right.capacity_5h))
                    .then_with(|| left.capacity_week.total_cmp(&right.capacity_week))
            })
    }

    fn remote_options_for_selected_host(&self) -> Option<&ProviderOptions> {
        let HostChoice::Remote(index) = self.host else {
            return None;
        };
        let node_id = self.cfg.hosts.get(index)?.id.as_str();
        let options = self.provider_options()?;
        (options.remote_account_node_id.as_deref() == Some(node_id)).then_some(options)
    }

    fn remote_account_is_ready(&self) -> bool {
        !matches!(self.host, HostChoice::Remote(_))
            || !matches!(self.agent, CLIAgent::Claude | CLIAgent::Codex)
            || self.selected_remote_account().is_some()
    }

    fn local_account_is_ready(&self) -> bool {
        if !matches!(self.host, HostChoice::Local)
            || !matches!(self.agent, CLIAgent::Claude | CLIAgent::Codex)
        {
            return true;
        }
        let Some(options) = self.provider_options() else {
            return false;
        };
        match self.account {
            AccountChoice::Freest => options.freest.is_some() || options.accounts.is_empty(),
            AccountChoice::Specific(index) => options.accounts.get(index).is_some(),
        }
    }

    fn account_is_ready(&self) -> bool {
        self.local_account_is_ready() && self.remote_account_is_ready()
    }

    fn host_is_ready(&self) -> bool {
        match self.host {
            HostChoice::Unselected => false,
            HostChoice::Local => true,
            HostChoice::Remote(index) => self.cfg.hosts.get(index).is_some(),
        }
    }

    /// Apply the routing-relevant part of a host change without performing I/O.
    /// The UI action adds editor/folder resets and starts fresh discovery after
    /// this state transition; keeping the transition pure makes the no-fallback
    /// and invalidation contract directly testable.
    fn select_host_for_launch(&mut self, host: HostChoice) -> bool {
        let host = match host {
            HostChoice::Remote(index) if self.cfg.hosts.get(index).is_none() => {
                HostChoice::Unselected
            }
            HostChoice::Unselected | HostChoice::Local | HostChoice::Remote(_) => host,
        };
        if self.host == host {
            return false;
        }
        self.host = host;
        if matches!(host, HostChoice::Unselected | HostChoice::Local) {
            self.managed_mode = ManagedLaunchMode::Ordinary;
        } else {
            self.normalize_managed_mode();
        }
        self.account = AccountChoice::Freest;
        self.batch_accounts.clear();
        self.select_all_accounts = false;
        self.invalidate_bulk_plan();
        self.invalidate_model_for_target_change();
        true
    }

    fn select_account_for_launch(&mut self, account: AccountChoice) {
        self.account = account;
        self.batch_accounts.clear();
        self.select_all_accounts = false;
        self.show_accounts = false;
        self.invalidate_bulk_plan();
        self.invalidate_model_for_target_change();
    }

    fn request_remote_account_discovery(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(node_id) = self.resolved_node_id() else {
            return;
        };
        // Remote model discovery is account-scoped. Clear any prior model list
        // before refreshing the account inventory, then start model discovery
        // only after that inventory has selected an exact account.
        self.invalidate_model_for_target_change();
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
                        crate::t!("cockpit-spawn-card-account-inventory-unsupported"),
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
                        provider_account_id: account.provider_account_id,
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
        let should_request_models = matches!(
            options.remote_account_discovery,
            RemoteAccountDiscoveryState::Ready { .. }
        );
        if should_request_models && self.selected_remote_account().is_some() {
            self.request_model_discovery(ctx);
        } else {
            ctx.notify();
        }
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

    fn selected_host_supports_managed_fleet(&self) -> bool {
        match self.host {
            HostChoice::Unselected | HostChoice::Local => false,
            HostChoice::Remote(index) => self
                .cfg
                .hosts
                .get(index)
                .is_some_and(|host| host.managed_fleet_available),
        }
    }

    fn managed_mode_is_valid(&self) -> bool {
        match self.managed_mode {
            ManagedLaunchMode::Ordinary => true,
            ManagedLaunchMode::ManagedInteractive => {
                self.selected_host_supports_managed_fleet()
                    && matches!(self.agent, CLIAgent::Claude | CLIAgent::Codex)
            }
            ManagedLaunchMode::ClaudeRemoteControl => {
                self.selected_host_supports_managed_fleet() && self.agent == CLIAgent::Claude
            }
        }
    }

    fn normalize_managed_mode(&mut self) {
        if !self.managed_mode_is_valid() {
            self.managed_mode = ManagedLaunchMode::Ordinary;
        }
    }

    fn model_catalogs_cover_launch_accounts(&self) -> bool {
        let accounts = self.launch_accounts();
        if accounts.is_empty() {
            return false;
        }
        let Some(options) = self.provider_options() else {
            return false;
        };
        if options.model_catalogs.is_empty() {
            return accounts.len() == 1
                && options
                    .model_discovery_target
                    .as_ref()
                    .is_none_or(|target| self.model_discovery_target().as_ref() == Some(target));
        }
        accounts.iter().all(|account| {
            let Some(target) = self.model_discovery_target_for_account(account) else {
                return false;
            };
            options
                .model_catalogs
                .get(&account.id)
                .is_some_and(|catalog| {
                    same_model_target_scope(&catalog.target, &target)
                        && catalog.models.iter().any(|model| {
                            model.id == self.model
                                && (self.effort.is_empty()
                                    || model
                                        .supported_efforts
                                        .iter()
                                        .any(|effort| effort.id == self.effort))
                        })
                })
        })
    }

    fn model_is_ready(&self) -> bool {
        if self.managed_mode != ManagedLaunchMode::Ordinary {
            return self.managed_mode_is_valid();
        }
        match self.agent {
            CLIAgent::Claude | CLIAgent::Codex => self.provider_options().is_some_and(|options| {
                matches!(options.model_discovery, ModelDiscoveryState::Ready)
                    && self.model_catalogs_cover_launch_accounts()
                    && self.selected_model_capability().is_some()
            }),
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

    fn invalidate_model_for_target_change(&mut self) {
        self.model.clear();
        self.effort.clear();
        if let Some(options) = self.provider_options_mut() {
            options.models.clear();
            options.model_discovery = ModelDiscoveryState::NotRequested;
            options.model_discovery_generation += 1;
            options.model_discovery_target = None;
            options.model_discovery_cli_version = None;
            options.model_catalogs.clear();
            options.model_discovery_pending.clear();
        }
    }

    fn request_model_discovery(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.host_is_ready() {
            self.invalidate_model_for_target_change();
            ctx.notify();
            return;
        }
        if !self.selected_agent_is_available() {
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
        let accounts = self.launch_accounts();
        let account_count = accounts.len();
        let requests = accounts
            .into_iter()
            .filter_map(|account| {
                self.model_discovery_target_for_account(&account)
                    .map(|target| (account, target))
            })
            .collect::<Vec<_>>();
        if requests.is_empty() || requests.len() != account_count {
            self.invalidate_model_for_target_change();
            ctx.notify();
            return;
        }
        let node_id = self.resolved_node_id();
        let host_name = self
            .remote_host_name()
            .map(str::to_string)
            .unwrap_or_else(|| crate::t!("cockpit-spawn-card-host-local"));
        let working_directory = if matches!(self.host, HostChoice::Remote(_)) {
            match self.selected_remote_cwd(ctx) {
                Ok(Some(path)) => path,
                Ok(None) => PathBuf::from("."),
                Err(RemoteCwdError::RelativePath) => {
                    self.invalidate_model_for_target_change();
                    ctx.notify();
                    return;
                }
            }
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
        options.model_discovery_cli_version = None;
        options.model_discovery = ModelDiscoveryState::Loading;
        options.model_discovery_generation += 1;
        options.model_discovery_target = (requests.len() == 1)
            .then(|| requests.first().map(|(_, target)| target.clone()))
            .flatten();
        options.model_catalogs.clear();
        options.model_discovery_pending = requests
            .iter()
            .map(|(account, _)| account.id.clone())
            .collect();
        let generation = options.model_discovery_generation;
        for (account, discovery_target) in requests {
            ctx.emit(SpawnCardEvent::DiscoverModels {
                generation,
                agent,
                account_id: account.id,
                discovery_target,
                config_dir: account.config_dir,
                agent_launch_route: account.remote_route,
                expected_provider_account_id: account.provider_account_id,
                node_id: node_id.clone(),
                host_name: host_name.clone(),
                working_directory: working_directory.clone(),
            });
        }
        ctx.notify();
    }

    pub fn apply_model_capabilities(
        &mut self,
        agent: CLIAgent,
        generation: u64,
        account_id: &LaunchAccountId,
        discovery_target: &ModelDiscoveryTarget,
        result: Result<DiscoveredModels, ModelDiscoveryFailure>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.agent != agent {
            return;
        }
        let target_is_current = self
            .launch_accounts()
            .into_iter()
            .find(|account| &account.id == account_id)
            .and_then(|account| self.model_discovery_target_for_account(&account))
            .is_some_and(|current| same_model_target_scope(&current, discovery_target));
        if !target_is_current {
            return;
        }
        let host_name = self
            .remote_host_name()
            .map(str::to_string)
            .unwrap_or_else(|| crate::t!("cockpit-spawn-card-host-local"));
        let mut completed_selection = None;
        let mut failed = false;
        {
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
            if options.model_discovery_generation != generation
                || !options.model_discovery_pending.remove(account_id)
            {
                return;
            }
            match result {
                Ok(discovery) if discovery.models.is_empty() => {
                    options.models.clear();
                    options.model_catalogs.clear();
                    options.model_discovery_pending.clear();
                    options.model_discovery_cli_version = None;
                    options.model_discovery =
                        ModelDiscoveryState::Error(ModelDiscoveryFailure::Failed {
                            agent,
                            host: host_name,
                            detail: crate::t!("cockpit-spawn-card-no-models"),
                        });
                    failed = true;
                }
                Ok(discovery) => {
                    let mut catalog_target = discovery_target.clone();
                    catalog_target.cli_version = Some(discovery.cli_version);
                    options.model_catalogs.insert(
                        account_id.clone(),
                        AccountModelCatalog {
                            target: catalog_target,
                            models: discovery.models,
                        },
                    );
                    if options.model_discovery_pending.is_empty() {
                        let models = common_model_capabilities(options.model_catalogs.values());
                        if models.is_empty() {
                            options.models.clear();
                            options.model_discovery_cli_version = None;
                            options.model_discovery =
                                ModelDiscoveryState::Error(ModelDiscoveryFailure::Failed {
                                    agent,
                                    host: host_name,
                                    detail: crate::t!("cockpit-spawn-card-no-common-models"),
                                });
                            failed = true;
                        } else {
                            let default_id = unique_default(&models).map(|model| model.id.clone());
                            let default_effort = unique_default(&models)
                                .and_then(|model| model.default_effort.clone());
                            let first_version = options
                                .model_catalogs
                                .values()
                                .next()
                                .and_then(|catalog| catalog.target.cli_version.clone());
                            options.model_discovery_cli_version = first_version.filter(|version| {
                                options.model_catalogs.values().all(|catalog| {
                                    catalog.target.cli_version.as_ref() == Some(version)
                                })
                            });
                            options.model_discovery_target = (options.model_catalogs.len() == 1)
                                .then(|| {
                                    options
                                        .model_catalogs
                                        .values()
                                        .next()
                                        .map(|catalog| catalog.target.clone())
                                })
                                .flatten();
                            options.models = models;
                            options.model_discovery = ModelDiscoveryState::Ready;
                            completed_selection = Some((
                                default_id.unwrap_or_default(),
                                default_effort.unwrap_or_default(),
                            ));
                        }
                    }
                }
                Err(error) => {
                    options.models.clear();
                    options.model_catalogs.clear();
                    options.model_discovery_pending.clear();
                    options.model_discovery_cli_version = None;
                    options.model_discovery = ModelDiscoveryState::Error(error);
                    failed = true;
                }
            }
        }
        if failed {
            self.model.clear();
            self.effort.clear();
        } else if let Some((model, effort)) = completed_selection {
            self.model = model;
            self.effort = effort;
        }
        ctx.notify();
    }

    /// `true` when the selected host can resolve the chosen agent. Local
    /// launches use client install detection; remote Claude/Codex launches use
    /// the daemon's account and model discovery instead.
    fn selected_agent_is_available(&self) -> bool {
        match self.host {
            HostChoice::Unselected => false,
            HostChoice::Local => agent_is_installed(&self.cfg, self.agent),
            HostChoice::Remote(_) => match self.agent {
                CLIAgent::Claude | CLIAgent::Codex => true,
                CLIAgent::Antigravity | CLIAgent::Grok => agent_is_installed(&self.cfg, self.agent),
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
            },
        }
    }

    /// Resolve the chosen account to a config dir for the launch. Remote hosts
    /// always use their own default account (config dirs are local paths), so a
    /// remote launch yields `None` regardless of the account chip.
    fn resolved_config_dir(&self) -> Option<PathBuf> {
        if !matches!(self.host, HostChoice::Local) {
            return None;
        }
        let options = self.provider_options()?;
        match self.account {
            AccountChoice::Freest => options.freest_dir.clone(),
            AccountChoice::Specific(i) => options.accounts.get(i).and_then(account_config_pin),
        }
    }

    fn model_discovery_target_for_account(
        &self,
        account: &LaunchAccountTarget,
    ) -> Option<ModelDiscoveryTarget> {
        if !self.host_is_ready() {
            return None;
        }
        let account_id = account
            .remote_route
            .as_ref()
            .map(|route| route.account_id.clone())
            .or_else(|| {
                account
                    .config_dir
                    .as_ref()
                    .map(|path| path.display().to_string())
            })
            .unwrap_or_else(|| "default".to_string());
        Some(ModelDiscoveryTarget {
            agent: self.agent,
            node_id: self.resolved_node_id(),
            account_id,
            provider_account_id: account.provider_account_id.clone(),
            config_dir: account.config_dir.clone(),
            cli_version: None,
        })
    }

    fn model_discovery_target(&self) -> Option<ModelDiscoveryTarget> {
        let accounts = self.launch_accounts();
        if accounts.len() != 1 {
            return None;
        }
        let account = accounts.first()?;
        let mut target = self.model_discovery_target_for_account(account)?;
        target.cli_version = self
            .provider_options()
            .and_then(|options| options.model_discovery_cli_version.clone());
        Some(target)
    }

    fn resolved_node_id(&self) -> Option<String> {
        match self.host {
            HostChoice::Unselected | HostChoice::Local => None,
            HostChoice::Remote(i) => self.cfg.hosts.get(i).map(|h| h.id.clone()),
        }
    }

    /// Display name of the currently selected remote host (`None` when local).
    fn remote_host_name(&self) -> Option<&str> {
        match self.host {
            HostChoice::Unselected | HostChoice::Local => None,
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
        let managed = self.managed_mode != ManagedLaunchMode::Ordinary;
        (self.selected_agent_is_available()
            && self.host_is_ready()
            && self.model_is_ready()
            && self.account_is_ready()
            && self.managed_mode_is_valid())
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
            model: (!managed && !self.model.is_empty()).then(|| self.model.clone()),
            effort: (!managed && matches!(self.agent, CLIAgent::Codex))
                .then(|| self.effort.clone()),
            prompt: (!managed).then(|| self.prompt.clone()).flatten(),
            managed_mode: self.managed_mode,
            managed_launch_id: (self.managed_mode != ManagedLaunchMode::Ordinary)
                .then(|| uuid::Uuid::new_v4().to_string()),
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
            .or_default()
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
            crate::t!(
                "cockpit-spawn-card-sum-context-million",
                count = context_window / 1_000_000_u64
            )
        } else {
            crate::t!(
                "cockpit-spawn-card-sum-context-thousand",
                count = context_window / 1_000_u64
            )
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
            HostChoice::Unselected => {
                crate::t!("cockpit-spawn-card-sum-account-pending-host")
            }
            HostChoice::Remote(_) => self
                .selected_remote_account()
                .map(|account| account.label.clone())
                .unwrap_or_else(|| crate::t!("cockpit-spawn-card-sum-account-not-selected")),
            HostChoice::Local => match (self.provider_options(), self.account) {
                (Some(_), AccountChoice::Freest) => {
                    crate::t!("cockpit-spawn-card-sum-freest")
                }
                (Some(options), AccountChoice::Specific(i)) => options
                    .accounts
                    .get(i)
                    .map(|a| a.label.clone())
                    .unwrap_or_else(|| crate::t!("cockpit-spawn-card-sum-account-unavailable")),
                (None, _) => crate::t!("cockpit-spawn-card-cli-default-login"),
            },
        };
        let host = match self.host {
            HostChoice::Unselected => crate::t!("cockpit-spawn-card-sum-choose-host"),
            HostChoice::Local => crate::t!("cockpit-spawn-card-sum-local"),
            HostChoice::Remote(i) => self
                .cfg
                .hosts
                .get(i)
                .map(host_identity_label)
                .unwrap_or_else(|| crate::t!("cockpit-spawn-card-sum-remote")),
        };
        // Directory is always part of the summary — a first-class launch
        // attribute, never omitted (Codex gate: "dir is steering"). An unset dir
        // reads as the explicit default rather than silently vanishing. For a
        // remote host the dir is the *typed* input (read live from the editor so
        // the preview matches what Confirm will launch); local uses the
        // folder-picker result in `self.project`.
        let dir = if self.host == HostChoice::Unselected {
            crate::t!("cockpit-spawn-card-sum-directory-pending-host")
        } else if matches!(self.host, HostChoice::Remote(_)) {
            match self.selected_remote_cwd(app) {
                Ok(Some(path)) => path.display().to_string(),
                Ok(None) => crate::t!(
                    "cockpit-spawn-card-sum-host-home",
                    host = self
                        .remote_host_name()
                        .map(str::to_string)
                        .unwrap_or_else(|| crate::t!("cockpit-spawn-card-sum-host"))
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
        if self.managed_mode == ManagedLaunchMode::ManagedInteractive {
            parts.push(crate::t!("cockpit-spawn-card-mode-managed"));
        } else if self.managed_mode == ManagedLaunchMode::ClaudeRemoteControl {
            parts.push(crate::t!("cockpit-spawn-card-mode-remote-control"));
        } else if self.model.is_empty() {
            parts.push(crate::t!("cockpit-spawn-card-cli-default"));
        } else {
            parts.push(self.model.clone());
            if let Some(resolved) = self
                .selected_model_capability()
                .and_then(|model| model.resolved_model.as_deref())
                .filter(|resolved| !resolved.is_empty() && *resolved != self.model.as_str())
            {
                parts.push(crate::t!(
                    "ai-footer-subscription-model-resolved",
                    model = resolved
                ));
            }
            if !self.effort.is_empty() {
                parts.push(
                    self.selected_model_capability()
                        .and_then(|model| {
                            model
                                .supported_efforts
                                .iter()
                                .find(|effort| effort.id == self.effort)
                        })
                        .map(|effort| effort.display_name.clone())
                        .unwrap_or_else(|| self.effort.clone()),
                );
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

    fn folder_history_controls(
        &self,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Vec<Box<dyn Element>> {
        let host = self.history_host();
        let entries = self.folder_history.entries(&host);
        let mut controls = Vec::new();
        if self.folder_navigation.can_back() {
            controls.push(self.chip(
                "folder-back",
                "←".to_string(),
                false,
                SpawnCardAction::FolderBack,
                appearance,
            ));
        }
        if self.folder_navigation.can_forward() {
            controls.push(self.chip(
                "folder-forward",
                "→".to_string(),
                false,
                SpawnCardAction::FolderForward,
                appearance,
            ));
        }
        if !entries.is_empty() {
            controls.push(
                self.chip(
                    "folder-history",
                    if self.folder_history_open {
                        "▾"
                    } else {
                        "▸"
                    }
                    .to_string(),
                    self.folder_history_open,
                    SpawnCardAction::ToggleFolderHistory,
                    appearance,
                ),
            );
        }
        if !self.folder_history_open || entries.is_empty() {
            return controls;
        }

        if let Some(editor) = self.history_search_editor.as_ref() {
            let theme = appearance.theme();
            controls.push(
                ConstrainedBox::new(
                    Container::new(
                        appearance
                            .ui_builder()
                            .text_input(editor.clone())
                            .with_style(UiComponentStyles::default())
                            .build()
                            .finish(),
                    )
                    .with_horizontal_padding(8.)
                    .with_vertical_padding(4.)
                    .with_background(theme.surface_2())
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                    .with_border(Border::all(1.).with_border_fill(theme.outline()))
                    .finish(),
                )
                .with_width(MODAL_WIDTH - 48.)
                .finish(),
            );
            let query = editor.as_ref(app).buffer_text(app);
            for entry in self.folder_history.search(&host, &query) {
                let status = match &host {
                    FolderHistoryHost::Local => {
                        if std::fs::metadata(&entry.path).is_ok_and(|metadata| metadata.is_dir()) {
                            String::new()
                        } else {
                            crate::t!("cockpit-spawn-card-folder-unavailable")
                        }
                    }
                    FolderHistoryHost::Remote { .. } => match self
                        .history_validation
                        .get(&entry.path)
                        .copied()
                        .unwrap_or(DirectoryValidation::Unknown)
                    {
                        DirectoryValidation::Valid => String::new(),
                        DirectoryValidation::Stale => {
                            crate::t!("cockpit-spawn-card-folder-unavailable")
                        }
                        DirectoryValidation::Checking => {
                            crate::t!("cockpit-spawn-card-folder-checking")
                        }
                        DirectoryValidation::Unknown | DirectoryValidation::Unverifiable => {
                            crate::t!("cockpit-spawn-card-folder-verify")
                        }
                    },
                };
                controls.push(self.chip(
                    &format!("history:{}", entry.path.display()),
                    if status.is_empty() {
                        entry.path.display().to_string()
                    } else {
                        format!("{} · {status}", entry.path.display())
                    },
                    self.selected_directory(app).as_deref() == Some(entry.path.as_path()),
                    SpawnCardAction::SelectHistoryPath(entry.path.clone()),
                    appearance,
                ));
            }
        }
        controls
    }

    fn bulk_preview_element(
        &self,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let plan = match self.bulk_launch.as_ref() {
            Some(ledger) => ledger.plan.clone(),
            None => self.build_bulk_plan(app)?,
        };
        if plan.targets.len() <= 1 && self.bulk_launch.is_none() {
            return None;
        }
        let host = self
            .remote_host_name()
            .map(str::to_string)
            .unwrap_or_else(|| crate::t!("cockpit-spawn-card-host-local").to_string());
        let directory = self
            .selected_directory(app)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| crate::t!("cockpit-spawn-card-sum-default").to_string());
        let preview = plan.preview(&host, &directory);
        let label = format!(
            "{} × {} · {} · {}\n{}",
            preview.count,
            preview.provider,
            preview.host,
            preview.directory,
            preview.accounts.join(" · ")
        );
        let muted = appearance
            .theme()
            .sub_text_color(appearance.theme().background())
            .into_solid();
        Some(
            Container::new(
                Text::new_inline(label, appearance.ui_font_family(), 12.)
                    .with_color(muted)
                    .finish(),
            )
            .with_margin_bottom(8.)
            .finish(),
        )
    }

    fn bulk_failure_elements(&self, appearance: &Appearance) -> Vec<Box<dyn Element>> {
        let Some(ledger) = self.bulk_launch.as_ref() else {
            return Vec::new();
        };
        let error_color = appearance.theme().ui_error_color();
        ledger
            .failed()
            .into_iter()
            .map(|(target, message)| {
                Container::new(
                    Text::new_inline(
                        format!("{}: {message}", target.account.label),
                        appearance.ui_font_family(),
                        12.,
                    )
                    .with_color(error_color)
                    .finish(),
                )
                .with_margin_bottom(4.)
                .finish()
            })
            .collect()
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

        // Agent row (only installed launchable CLIs). A requested CLI that is no
        // longer installed remains the selection but is described explicitly;
        // the user must choose one of the available alternatives.
        let available = match self.host {
            HostChoice::Remote(_) => remote_agents(&self.cfg),
            HostChoice::Unselected | HostChoice::Local => installed_agents(&self.cfg),
        };
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
            let mut agent_chips: Vec<Box<dyn Element>> = available
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
            if !self.selected_agent_is_available() {
                agent_chips.insert(
                    0,
                    Container::new(
                        Text::new_inline(
                            format!(
                                "{} is unavailable. Choose an installed agent.",
                                self.agent.display_name()
                            ),
                            family,
                            12.,
                        )
                        .with_color(theme.ui_error_color())
                        .finish(),
                    )
                    .finish(),
                );
            }
            col = col.with_child(self.row(
                &crate::t!("cockpit-spawn-card-agent"),
                agent_chips,
                appearance,
            ));
        }

        // Managed launches are an explicit remote-only choice. Capability is
        // known from the selected connected daemon; an old/offline host gets a
        // concise unavailable state and never a silent ordinary-session fallback.
        let mut mode_controls = vec![self.chip(
            "mode-ordinary",
            crate::t!("cockpit-spawn-card-mode-ordinary"),
            self.managed_mode == ManagedLaunchMode::Ordinary,
            SpawnCardAction::SetManagedMode(ManagedLaunchMode::Ordinary),
            appearance,
        )];
        if self.selected_host_supports_managed_fleet()
            && matches!(self.agent, CLIAgent::Claude | CLIAgent::Codex)
        {
            mode_controls.push(self.chip(
                "mode-managed",
                crate::t!("cockpit-spawn-card-mode-managed"),
                self.managed_mode == ManagedLaunchMode::ManagedInteractive,
                SpawnCardAction::SetManagedMode(ManagedLaunchMode::ManagedInteractive),
                appearance,
            ));
            if self.agent == CLIAgent::Claude {
                mode_controls.push(self.chip(
                    "mode-remote-control",
                    crate::t!("cockpit-spawn-card-mode-remote-control"),
                    self.managed_mode == ManagedLaunchMode::ClaudeRemoteControl,
                    SpawnCardAction::SetManagedMode(ManagedLaunchMode::ClaudeRemoteControl),
                    appearance,
                ));
            }
        } else if matches!(self.host, HostChoice::Remote(_)) {
            mode_controls.push(
                Container::new(
                    Text::new_inline(
                        crate::t!("cockpit-spawn-card-managed-unavailable"),
                        family,
                        12.,
                    )
                    .with_color(muted)
                    .finish(),
                )
                .finish(),
            );
        }
        col = col.with_child(self.row(
            &crate::t!("cockpit-spawn-card-mode"),
            mode_controls,
            appearance,
        ));

        // Model row + a live context-window readout when the CLI contract is
        // known. Antigravity launches with its own default; the card does not
        // invent a curated model list or a context size.
        if self.managed_mode == ManagedLaunchMode::Ordinary {
            let mut model_chips: Vec<Box<dyn Element>> = if self.host == HostChoice::Unselected {
                vec![Container::new(
                    Text::new_inline(
                        crate::t!("cockpit-spawn-card-choose-host-for-models"),
                        family,
                        12.,
                    )
                    .with_color(muted)
                    .finish(),
                )
                .finish()]
            } else {
                match self.provider_options() {
                    Some(options) => match &options.model_discovery {
                        ModelDiscoveryState::NotRequested => vec![Container::new(
                            Text::new_inline(
                                crate::t!("cockpit-spawn-card-apply-dir-for-models"),
                                family,
                                12.,
                            )
                            .with_color(muted)
                            .finish(),
                        )
                        .finish()],
                        ModelDiscoveryState::Loading => {
                            vec![Container::new(
                                Text::new_inline(
                                    crate::t!("cockpit-spawn-card-discovering-models"),
                                    family,
                                    13.,
                                )
                                .with_color(muted)
                                .finish(),
                            )
                            .finish()]
                        }
                        ModelDiscoveryState::Error(error) => vec![Container::new(
                            Text::new_inline(error.label(), family, 12.)
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
                                    model_identity_label(model),
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
                }
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
        } else {
            col = col.with_child(self.row(
                &crate::t!("cockpit-spawn-card-settings"),
                vec![Container::new(
                    Text::new_inline(
                        crate::t!("cockpit-spawn-card-managed-provider-defaults"),
                        family,
                        12.,
                    )
                    .with_color(muted)
                    .finish(),
                )
                .finish()],
                appearance,
            ));
        }

        // Account row — local config identities stay local; remote identities are
        // opaque daemon account ids discovered for the selected host.
        if self.host == HostChoice::Unselected {
            col = col.with_child(self.row(
                &crate::t!("cockpit-spawn-card-account"),
                vec![Container::new(
                    Text::new_inline(crate::t!("cockpit-spawn-card-choose-host-first"), family, 12.)
                        .with_color(muted)
                        .finish(),
                )
                .finish()],
                appearance,
            ));
        } else if matches!(self.host, HostChoice::Remote(_)) {
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
        let mut host_chips: Vec<Box<dyn Element>> = Vec::new();
        if self.host == HostChoice::Unselected {
            host_chips.push(
                Container::new(
                    Text::new_inline(
                        crate::t!("cockpit-spawn-card-requested-host-unavailable"),
                        family,
                        12.,
                    )
                    .with_color(theme.ui_error_color())
                    .finish(),
                )
                .finish(),
            );
        }
        host_chips.push(self.chip(
            "host-local",
            crate::t!("cockpit-spawn-card-host-local"),
            self.host == HostChoice::Local,
            SpawnCardAction::SetHostLocal,
            appearance,
        ));
        for (i, h) in self.cfg.hosts.iter().enumerate() {
            host_chips.push(self.chip(
                &format!("host-{i}"),
                host_identity_label(h),
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
        if self.host == HostChoice::Unselected {
            col = col.with_child(self.row(
                &crate::t!("cockpit-spawn-card-directory"),
                vec![Container::new(
                    Text::new_inline(
                        crate::t!("cockpit-spawn-card-choose-host-for-directory"),
                        family,
                        12.,
                    )
                        .with_color(muted)
                        .finish(),
                )
                .finish()],
                appearance,
            ));
        } else if matches!(self.host, HostChoice::Remote(_)) {
            let fallback_host = crate::t!("cockpit-spawn-card-host-fallback");
            let host = self.remote_host_name().unwrap_or(&fallback_host);
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
            let apply = self.chip(
                "dir-apply",
                crate::t!("cockpit-spawn-card-use-directory"),
                false,
                SpawnCardAction::ApplyRemoteDirectory,
                appearance,
            );
            col = col.with_child(self.row(&label, vec![input, apply, browse], appearance));
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

        let history_controls = self.folder_history_controls(app, appearance);
        if self.host != HostChoice::Unselected
            && (!self.folder_history.entries(&self.history_host()).is_empty()
                || self.folder_navigation.can_back()
                || self.folder_navigation.can_forward())
        {
            col = col.with_child(self.row(
                &crate::t!("cockpit-spawn-card-recent-folders"),
                history_controls,
                appearance,
            ));
        }

        if let Some(preview) = self.bulk_preview_element(app, appearance) {
            col = col.with_child(preview);
        }
        for failure in self.bulk_failure_elements(appearance) {
            col = col.with_child(failure);
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
        let can_launch = self.selected_agent_is_available()
            && self.host_is_ready()
            && self.model_is_ready()
            && self.managed_mode_is_valid()
            && self.account_is_ready()
            && self.selected_remote_cwd(app).is_ok()
            && (self.managed_mode == ManagedLaunchMode::Ordinary
                || self.selected_directory(app).is_some())
            && (self.selected_directory(app).is_none() || self.folder_validation.is_valid());
        let confirm: Box<dyn Element> = if can_launch {
            let label = self
                .bulk_launch
                .as_ref()
                .filter(|ledger| !ledger.failed().is_empty())
                .map(|_| crate::t!("cockpit-spawn-card-retry-failed"))
                .unwrap_or_else(|| {
                    crate::t!(
                        "cockpit-spawn-card-launch",
                        agent = self.agent.display_name()
                    )
                    .to_string()
                });
            self.chip("confirm", label, true, SpawnCardAction::Confirm, appearance)
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
                    self.normalize_managed_mode();
                    // Account indices are provider-specific; fall back to freest.
                    self.account = AccountChoice::Freest;
                    self.batch_accounts.clear();
                    self.select_all_accounts = false;
                    self.invalidate_bulk_plan();
                    match self.host {
                        HostChoice::Unselected => self.invalidate_model_for_target_change(),
                        HostChoice::Local => self.request_model_discovery(ctx),
                        HostChoice::Remote(_) => self.request_remote_account_discovery(ctx),
                    }
                }
            }
            SpawnCardAction::SetModel(m) => {
                self.model = m.clone();
                self.effort = self
                    .selected_model_capability()
                    .and_then(|model| model.default_effort.clone())
                    .unwrap_or_default();
                self.invalidate_bulk_plan();
                ctx.notify();
            }
            SpawnCardAction::SetEffort(e) => {
                self.effort = e.clone();
                self.invalidate_bulk_plan();
                ctx.notify();
            }
            SpawnCardAction::SetManagedMode(mode) => {
                self.managed_mode = *mode;
                self.normalize_managed_mode();
                self.invalidate_bulk_plan();
                if self.managed_mode == ManagedLaunchMode::Ordinary {
                    self.request_model_discovery(ctx);
                } else {
                    ctx.notify();
                }
            }
            SpawnCardAction::ToggleAccountList => {
                self.show_accounts = !self.show_accounts;
                ctx.notify();
            }
            SpawnCardAction::SetAccountFreest => {
                // Chosen — fold back to the calm line. Leaving the list open
                // after a pick would keep asking a question already answered.
                self.select_account_for_launch(AccountChoice::Freest);
                self.request_model_discovery(ctx);
            }
            SpawnCardAction::SetAccount(i) => {
                self.select_account_for_launch(AccountChoice::Specific(*i));
                self.request_model_discovery(ctx);
            }
            SpawnCardAction::SetAllAccounts => {
                self.select_all_accounts = true;
                self.batch_accounts.clear();
                self.invalidate_bulk_plan();
                self.show_accounts = false;
                if self.managed_mode == ManagedLaunchMode::Ordinary {
                    self.request_model_discovery(ctx);
                } else {
                    ctx.notify();
                }
            }
            SpawnCardAction::ToggleBatchAccount(index) => {
                let id = match self.host {
                    HostChoice::Unselected => None,
                    HostChoice::Local => self.provider_options().and_then(|options| {
                        options
                            .accounts
                            .get(*index)
                            .map(|account| LaunchAccountId::local(self.agent, &account.config_dir))
                    }),
                    HostChoice::Remote(_) => {
                        self.remote_options_for_selected_host().and_then(|options| {
                            options.remote_accounts.get(*index).map(|account| {
                                LaunchAccountId::remote(self.agent, &account.route.account_id)
                            })
                        })
                    }
                };
                if let Some(id) = id {
                    self.select_all_accounts = false;
                    if !self.batch_accounts.insert(id.clone()) {
                        self.batch_accounts.remove(&id);
                    }
                    // A one-account batch is a valid explicit selection; keep
                    // the list open so a second account remains one click away.
                    self.invalidate_bulk_plan();
                    if self.managed_mode == ManagedLaunchMode::Ordinary {
                        self.request_model_discovery(ctx);
                    } else {
                        ctx.notify();
                    }
                }
            }
            SpawnCardAction::SetHostLocal => {
                if self.select_host_for_launch(HostChoice::Local) {
                    self.folder_navigation.reset(self.project.clone());
                    self.begin_selected_directory_validation(ctx);
                    self.request_model_discovery(ctx);
                }
            }
            SpawnCardAction::SetHost(i) => {
                if self.select_host_for_launch(HostChoice::Remote(*i)) {
                    if let Some(editor) = self.remote_dir_editor.clone() {
                        editor.update(ctx, |editor, ctx| {
                            editor.set_buffer_text_with_base_buffer("", ctx);
                        });
                    }
                    self.folder_navigation.reset(None);
                    self.begin_selected_directory_validation(ctx);
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
                    self.set_selected_directory(PathBuf::from(path), ctx);
                }
                Err(err) => {
                    log::warn!("Spawn card directory picker error: {err}");
                }
            },
            SpawnCardAction::ClearDirectory => {
                self.project = None;
                self.folder_navigation.reset(None);
                self.folder_validation.clear();
                self.invalidate_bulk_plan();
                self.invalidate_model_for_target_change();
                self.request_model_discovery(ctx);
            }
            SpawnCardAction::ToggleFolderHistory => {
                self.folder_history_open = !self.folder_history_open;
                ctx.notify();
            }
            SpawnCardAction::SelectHistoryPath(path) => {
                self.set_selected_directory(path.clone(), ctx);
                self.folder_history_open = false;
            }
            SpawnCardAction::FolderBack => {
                if let Some(path) = self.folder_navigation.back().map(Path::to_path_buf) {
                    self.set_selected_directory(path, ctx);
                }
            }
            SpawnCardAction::FolderForward => {
                if let Some(path) = self.folder_navigation.forward().map(Path::to_path_buf) {
                    self.set_selected_directory(path, ctx);
                }
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
            SpawnCardAction::ApplyRemoteDirectory => {
                if matches!(self.host, HostChoice::Remote(_))
                    && self.selected_remote_cwd(ctx).is_ok()
                {
                    self.request_model_discovery(ctx);
                }
            }
            SpawnCardAction::Confirm => {
                // Guard here too (not just in the chip's on_click), so the
                // "enter" keybinding can't launch an uninstalled CLI either:
                // `launch_payload` is `None` when nothing is installed.
                if let Some(launch) = self.launch_attempt(ctx) {
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
        account_id: LaunchAccountId,
        discovery_target: ModelDiscoveryTarget,
        config_dir: Option<PathBuf>,
        agent_launch_route: Option<remote_server::proto::AgentLaunchRoute>,
        expected_provider_account_id: Option<String>,
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
    ValidateDirectory(DirectoryValidationRequest),
    LaunchBatch {
        plan_id: BulkLaunchPlanId,
        targets: Vec<(BulkLaunchTargetId, BulkLaunchTarget)>,
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
        managed_mode: ManagedLaunchMode,
        managed_launch_id: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub enum SpawnCardAction {
    SetAgent(CLIAgent),
    SetModel(String),
    SetEffort(String),
    SetManagedMode(ManagedLaunchMode),
    SetAccountFreest,
    SetAccount(usize),
    SetAllAccounts,
    ToggleBatchAccount(usize),
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
    ToggleFolderHistory,
    SelectHistoryPath(PathBuf),
    FolderBack,
    FolderForward,
    /// Open the remote host's SFTP browser to pick the launch directory (#105).
    BrowseRemoteDir,
    /// Confirm the typed remote cwd and rediscover target-specific models.
    ApplyRemoteDirectory,
    Confirm,
    Close,
}

#[cfg(test)]
#[path = "spawn_card_tests.rs"]
mod tests;
