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
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DropShadow,
    Element, Flex, FormattedTextElement, Hoverable, MainAxisSize, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::terminal::CLIAgent;

const MODAL_WIDTH: f32 = 480.;

/// One account option offered in the card.
#[derive(Clone, Debug)]
pub struct AccountOption {
    pub label: String,
    pub config_dir: PathBuf,
    pub heat_label: String,
}

/// The account options for one provider, plus its precomputed freest pick.
#[derive(Clone, Debug, Default)]
pub struct ProviderOptions {
    pub installed: bool,
    /// Display label of the freest account (heat included), if any.
    pub freest_label: Option<String>,
    /// Config dir of the freest account (`None` = only the default login).
    pub freest_dir: Option<PathBuf>,
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
    /// Pre-scoped host id (from a Conductor host-header `+`); `None` = local.
    pub scoped_host: Option<String>,
    /// Pre-scoped project dir (from a Conductor project-header `+` / context).
    pub project: Option<PathBuf>,
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
    host: HostChoice,
    project: Option<PathBuf>,
    chip_states: std::cell::RefCell<std::collections::HashMap<String, MouseStateHandle>>,
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

/// The thinking-effort presets. `low`/`medium`/`high` are exactly Codex's
/// accepted `model_reasoning_effort` values; for Claude they are recorded for
/// the Conductor (Claude has no effort CLI flag).
const EFFORTS: &[&str] = &["low", "medium", "high"];

impl SpawnCard {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        SpawnCard {
            cfg: SpawnCardConfig::default(),
            agent: CLIAgent::Claude,
            model: "opus".to_string(),
            effort: "high".to_string(),
            account: AccountChoice::Freest,
            host: HostChoice::Local,
            project: None,
            chip_states: Default::default(),
        }
    }

    /// (Re)initialize the card from a fresh config + optional pre-scoping. Picks
    /// smart defaults so the common case is a single confirm.
    pub fn configure(&mut self, cfg: SpawnCardConfig) {
        // Default agent: Claude if available, else Codex, else Claude.
        self.agent = if cfg.claude.installed || !cfg.codex.installed {
            CLIAgent::Claude
        } else {
            CLIAgent::Codex
        };
        self.model = models_for(self.agent)
            .first()
            .copied()
            .unwrap_or("opus")
            .to_string();
        self.effort = "high".to_string();
        self.account = AccountChoice::Freest;
        // Pre-scope host from a Conductor host `+`, else local.
        self.host = cfg
            .scoped_host
            .as_ref()
            .and_then(|scoped| {
                cfg.hosts
                    .iter()
                    .position(|h| &h.id == scoped || &h.name == scoped)
            })
            .map(HostChoice::Remote)
            .unwrap_or(HostChoice::Local);
        self.project = cfg.project.clone();
        self.cfg = cfg;
    }

    fn provider_options(&self) -> &ProviderOptions {
        match self.agent {
            CLIAgent::Codex => &self.cfg.codex,
            _ => &self.cfg.claude,
        }
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
    fn summary(&self) -> String {
        let account = match self.host {
            HostChoice::Remote(_) => "host account".to_string(),
            HostChoice::Local => match self.account {
                AccountChoice::Freest => "freest".to_string(),
                AccountChoice::Specific(i) => self
                    .provider_options()
                    .accounts
                    .get(i)
                    .map(|a| a.label.clone())
                    .unwrap_or_else(|| "freest".to_string()),
            },
        };
        let host = match self.host {
            HostChoice::Local => "local".to_string(),
            HostChoice::Remote(i) => self
                .cfg
                .hosts
                .get(i)
                .map(|h| h.name.clone())
                .unwrap_or_else(|| "remote".to_string()),
        };
        let mut s = format!(
            "{} · {} · {} · {} · {} · {}",
            self.agent.display_name(),
            self.model,
            Self::cap(&self.effort),
            Self::context_label(&self.model),
            account,
            host,
        );
        if let Some(dir) = &self.project {
            s.push_str(&format!(" · {}", dir.display()));
        }
        s
    }

    fn render_card(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        let title = FormattedTextElement::from_str("New Agent", family, 20.)
            .with_color(main)
            .with_weight(Weight::Bold)
            .finish();
        let subtitle = Text::new_inline(
            "Pick exactly what starts — model and thinking-effort are the launch.",
            family,
            12.,
        )
        .with_color(muted)
        .finish();

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(Container::new(title).with_margin_bottom(4.).finish())
            .with_child(Container::new(subtitle).with_margin_bottom(18.).finish());

        // Agent row (only installed providers).
        let mut agent_chips = Vec::new();
        for agent in [CLIAgent::Claude, CLIAgent::Codex] {
            let installed = match agent {
                CLIAgent::Codex => self.cfg.codex.installed,
                _ => self.cfg.claude.installed,
            };
            if !installed {
                continue;
            }
            agent_chips.push(self.chip(
                &format!("agent-{}", agent.to_serialized_name()),
                agent.display_name().to_string(),
                self.agent == agent,
                SpawnCardAction::SetAgent(agent),
                appearance,
            ));
        }
        if agent_chips.is_empty() {
            // Neither installed: still offer Claude so the card is never empty.
            agent_chips.push(self.chip(
                "agent-claude",
                CLIAgent::Claude.display_name().to_string(),
                true,
                SpawnCardAction::SetAgent(CLIAgent::Claude),
                appearance,
            ));
        }
        col = col.with_child(self.row("Agent", agent_chips, appearance));

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
        col = col.with_child(self.row("Model", model_chips, appearance));

        // Effort row.
        let effort_note = if matches!(self.agent, CLIAgent::Claude) {
            " (tracked)"
        } else {
            ""
        };
        let effort_chips: Vec<Box<dyn Element>> = EFFORTS
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
            .collect();
        col = col.with_child(self.row(&format!("Effort{effort_note}"), effort_chips, appearance));

        // Account row — freest + explicit accounts, unless a remote host owns it.
        if matches!(self.host, HostChoice::Remote(_)) {
            col = col.with_child(self.row(
                "Account",
                vec![Container::new(
                    Text::new_inline(
                        "Uses the host's default account".to_string(),
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
            let opts = self.provider_options();
            let mut acct_chips = Vec::new();
            let freest_label = opts
                .freest_label
                .clone()
                .map(|l| format!("⚡ Freest — {l}"))
                .unwrap_or_else(|| "⚡ Freest".to_string());
            acct_chips.push(self.chip(
                "acct-freest",
                freest_label,
                self.account == AccountChoice::Freest,
                SpawnCardAction::SetAccountFreest,
                appearance,
            ));
            for (i, a) in opts.accounts.iter().enumerate() {
                acct_chips.push(self.chip(
                    &format!("acct-{i}"),
                    format!("{} ({})", a.label, a.heat_label),
                    self.account == AccountChoice::Specific(i),
                    SpawnCardAction::SetAccount(i),
                    appearance,
                ));
            }
            col = col.with_child(self.row("Account", acct_chips, appearance));
        }

        // Host row — local + connected SSH hosts.
        let mut host_chips = vec![self.chip(
            "host-local",
            "Local".to_string(),
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
        col = col.with_child(self.row("Host", host_chips, appearance));

        // Project (read-only display of the launch dir).
        let project_str = self
            .project
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "Default directory".to_string());
        col = col.with_child(self.row(
            "Project",
            vec![Container::new(
                Text::new_inline(project_str, family, 12.).with_color(main).finish(),
            )
            .finish()],
            appearance,
        ));

        // Summary line.
        col = col.with_child(
            Container::new(
                Text::new_inline(format!("Launching: {}", self.summary()), family, 12.)
                    .with_color(muted)
                    .finish(),
            )
            .with_margin_bottom(16.)
            .finish(),
        );

        // Confirm + cancel.
        let confirm = self.chip(
            "confirm",
            format!("Launch {}", self.agent.display_name()),
            true,
            SpawnCardAction::Confirm,
            appearance,
        );
        let cancel = self.chip(
            "cancel",
            "Cancel".to_string(),
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

        Container::new(col.finish())
            .with_uniform_padding(24.)
            .with_background(theme.background())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_drop_shadow(DropShadow::default())
            .finish()
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
        let card = ConstrainedBox::new(self.render_card(app))
            .with_width(MODAL_WIDTH)
            .finish();

        let mut stack = Stack::new();
        stack.add_positioned_child(
            card,
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                warpui::elements::ChildAnchor::Center,
            ),
        );

        Container::new(
            Align::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(stack.finish())
                    .finish(),
            )
            .finish(),
        )
        .with_background(Fill::Solid(ColorU::new(97, 97, 97, 255)).with_opacity(50))
        .finish()
    }
}

impl TypedActionView for SpawnCard {
    type Action = SpawnCardAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SpawnCardAction::SetAgent(agent) => {
                if self.agent != *agent {
                    self.agent = *agent;
                    // Reset model to the new provider's default; keep effort.
                    self.model = models_for(*agent)
                        .first()
                        .copied()
                        .unwrap_or("opus")
                        .to_string();
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
            SpawnCardAction::SetAccountFreest => {
                self.account = AccountChoice::Freest;
                ctx.notify();
            }
            SpawnCardAction::SetAccount(i) => {
                self.account = AccountChoice::Specific(*i);
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
            SpawnCardAction::Confirm => {
                ctx.emit(SpawnCardEvent::Launch {
                    agent: self.agent,
                    config_dir: self.resolved_config_dir(),
                    cwd: self.project.clone(),
                    node_id: self.resolved_node_id(),
                    model: Some(self.model.clone()),
                    effort: Some(self.effort.clone()),
                });
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
    Launch {
        agent: CLIAgent,
        config_dir: Option<PathBuf>,
        cwd: Option<PathBuf>,
        node_id: Option<String>,
        model: Option<String>,
        effort: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub enum SpawnCardAction {
    SetAgent(CLIAgent),
    SetModel(String),
    SetEffort(String),
    SetAccountFreest,
    SetAccount(usize),
    SetHostLocal,
    SetHost(usize),
    Confirm,
    Close,
}
