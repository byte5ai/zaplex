//! `CockpitAccountsPanel` — the „KI-Konten" sidebar view: provider-explicit
//! account cards with stacked 5-hour and weekly meters plus the fleet total.
//!
//! It is its own toolbelt entry beside the live session tree (#504), so a long
//! session inventory can never push the accounts out of reach. Both views read
//! the same `CockpitModel`; neither owns a second copy of the data.

use std::collections::HashMap;

use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, DispatchEventResult, Element, EventHandler,
    Fill as ElementFill, Flex, Hoverable, MainAxisSize, MouseStateHandle, ParentElement, Radius,
    ScrollbarWidth, Shrinkable, Text,
};
use warpui::{
    accessibility::{AccessibilityContent, WarpA11yRole},
    AppContext, BlurContext, Entity, FocusContext, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};
use zaplex_cockpit::{
    heat_bar_fraction, heat_pct_label_with_provenance, AccountUsage, ScanHealth, UsageProvenance,
};

use crate::cockpit::account_identity;
use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::panel::{account_count_presentation, CockpitPanel, CARD_PADDING, CARD_SPACING};
use crate::cockpit::style::{
    account_identity_subline, provider_mark, utilisation_coloru, utilisation_track, zone_card,
    BLOCK_RADIUS, METER_LABEL_WIDTH,
};
use crate::view_components::action_button::{ActionButton, ButtonSize, SecondaryTheme};

const HEAT_BAR_HEIGHT: f32 = 6.0;
/// Size of the provider mark leading an account card's headline.
const PROVIDER_MARK_SIZE: f32 = 12.0;
/// Gap between the provider mark and the provider name.
const PROVIDER_MARK_GAP: f32 = 6.0;

/// What the „KI-KONTEN" header shows: the account count and the label and
/// tooltip of its „Alle Konten" control. Spend lives in the fleet pane that
/// control opens, not in the header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AccountsHeaderPresentation {
    pub count: Option<usize>,
    pub fleet_label: String,
    pub fleet_tooltip: String,
}

pub(super) fn accounts_header_presentation(
    health: &ScanHealth,
    account_count: usize,
) -> AccountsHeaderPresentation {
    AccountsHeaderPresentation {
        count: account_count_presentation(health, account_count),
        fleet_label: crate::t!("cockpit-header-all-accounts"),
        fleet_tooltip: crate::t!("cockpit-header-all-accounts-tooltip"),
    }
}

/// Events the accounts view emits toward the workspace (via the left panel).
pub enum CockpitAccountsPanelEvent {
    /// Open the cockpit pane in the main area: `None` = the fleet dashboard
    /// (every account), `Some(account.key)` = that account's own pane.
    ///
    /// The key travels with the request because the pane IS the account —
    /// opening dedupes on it, so two accounts open two panes rather than one
    /// dashboard that can only look at whichever was clicked last.
    OpenCockpitPane(Option<String>),
}

enum FleetOverviewButtonEvent {
    Activated,
}

#[derive(Clone, Debug)]
enum FleetOverviewButtonAction {
    Activate,
}

fn is_fleet_overview_activation_keystroke(keystroke: &warpui::keymap::Keystroke) -> bool {
    !keystroke.cmd
        && !keystroke.ctrl
        && !keystroke.alt
        && !keystroke.shift
        && !keystroke.meta
        && matches!(keystroke.key.as_str(), "enter" | "numpadenter" | " ")
}

/// The „Alle Konten" control in the „KI-KONTEN" header: a small bordered
/// button that opens the fleet pane (spec v3 §S1), focusable and activated by
/// Enter or Space like any button.
struct FleetOverviewButton {
    button: ViewHandle<ActionButton>,
    label: String,
    #[cfg(test)]
    activation_count: usize,
}

impl FleetOverviewButton {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        let button = ctx.add_typed_action_view(|_| {
            ActionButton::new("", SecondaryTheme)
                .with_size(ButtonSize::XSmall)
                .on_click(|ctx| ctx.dispatch_typed_action(FleetOverviewButtonAction::Activate))
        });
        Self {
            button,
            label: String::new(),
            #[cfg(test)]
            activation_count: 0,
        }
    }

    fn set_label(&mut self, label: String, tooltip: String, ctx: &mut ViewContext<Self>) {
        self.label.clone_from(&label);
        self.button.update(ctx, |button, ctx| {
            button.set_label(label, ctx);
            button.set_tooltip(Some(tooltip), ctx);
        });
        ctx.notify();
    }
}

impl Entity for FleetOverviewButton {
    type Event = FleetOverviewButtonEvent;
}

impl View for FleetOverviewButton {
    fn ui_name() -> &'static str {
        "FleetOverviewButton"
    }

    fn accessibility_contents(&self, _ctx: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new_without_help(
            self.label.clone(),
            WarpA11yRole::ButtonRole,
        ))
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.button
                .update(ctx, |button, ctx| button.set_active(true, ctx));
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.button
                .update(ctx, |button, ctx| button.set_active(false, ctx));
        }
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        EventHandler::new(ChildView::new(&self.button).finish())
            .on_keydown(|ctx, _, keystroke| {
                if is_fleet_overview_activation_keystroke(keystroke) {
                    ctx.dispatch_typed_action(FleetOverviewButtonAction::Activate);
                    DispatchEventResult::StopPropagation
                } else {
                    DispatchEventResult::PropagateToParent
                }
            })
            .finish()
    }
}

impl TypedActionView for FleetOverviewButton {
    type Action = FleetOverviewButtonAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FleetOverviewButtonAction::Activate => {
                #[cfg(test)]
                {
                    self.activation_count += 1;
                }
                ctx.focus_self();
                ctx.emit(FleetOverviewButtonEvent::Activated);
            }
        }
    }
}

pub struct CockpitAccountsPanel {
    scroll_state: ClippedScrollStateHandle,
    /// Hover state per account card (key = account `key`). The whole card is a
    /// click target that opens the roomy dashboard pane.
    card_states: HashMap<String, MouseStateHandle>,
    /// Tooltip state per card's account slot (key = account `key`), which names
    /// the email behind an alias.
    identity_tooltip_states: HashMap<String, MouseStateHandle>,
    /// The „Alle Konten" control in the „KI-KONTEN" header, the entry point to
    /// the fleet pane with the cross-account spend (spec v3 §S1).
    fleet_overview_button: ViewHandle<FleetOverviewButton>,
    /// Hover/click state for the "try again" retry (the loading / scan-failed /
    /// empty placeholder). A **stable** handle: `Hoverable` tracks mouse-down in
    /// it, so a fresh one each render would drop the click.
    rescan_btn: MouseStateHandle,
}

impl CockpitAccountsPanel {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let fleet_overview_button = ctx.add_typed_action_view(FleetOverviewButton::new);
        ctx.subscribe_to_view(&fleet_overview_button, |_, _, event, ctx| match event {
            FleetOverviewButtonEvent::Activated => {
                ctx.dispatch_typed_action(&CockpitAccountsPanelAction::OpenDashboardPane)
            }
        });
        // Re-render on theme change and whenever the snapshot updates.
        ctx.subscribe_to_model(&Appearance::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&CockpitModel::handle(ctx), |me, _, event, ctx| {
            if matches!(event, CockpitEvent::Updated) {
                me.sync_card_states(ctx);
                ctx.notify();
            }
        });
        let header = accounts_header_presentation(&ScanHealth::Pending, 0);
        fleet_overview_button.update(ctx, |button, ctx| {
            button.set_label(header.fleet_label, header.fleet_tooltip, ctx)
        });
        let mut me = Self {
            scroll_state: ClippedScrollStateHandle::default(),
            card_states: HashMap::new(),
            identity_tooltip_states: HashMap::new(),
            fleet_overview_button,
            rescan_btn: MouseStateHandle::default(),
        };
        me.sync_card_states(ctx);
        me
    }

    /// Card hover handles, keyed by account `key` (one stable handle per card
    /// across renders); drop handles of accounts that disappeared.
    fn sync_card_states(&mut self, ctx: &mut ViewContext<Self>) {
        let acct_keys: std::collections::HashSet<String> = CockpitModel::as_ref(ctx)
            .snapshot()
            .accounts
            .iter()
            .map(|a| a.account.key.clone())
            .collect();
        self.card_states.retain(|k, _| acct_keys.contains(k));
        self.identity_tooltip_states
            .retain(|k, _| acct_keys.contains(k));
        for key in acct_keys {
            self.card_states.entry(key.clone()).or_default();
            self.identity_tooltip_states.entry(key).or_default();
        }
    }

    /// The placeholder, disambiguated by scan health so an empty account list
    /// no longer reads the same whether the first scan is still running, a
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
        let muted = theme.sub_text_color(theme.surface_2()).into_solid();
        let accent = theme.accent().into_solid();
        // A deliberately-disabled cockpit is neither "empty" nor "loading" — say so,
        // and offer no retry (re-scanning cannot help while it is off).
        if !enabled {
            return CockpitPanel::text(
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
        let msg_el = CockpitPanel::text(msg, family, body, muted);
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
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(CockpitAccountsPanelAction::Rescan))
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
    /// get no extra chrome. An unknown window is an empty track reading `—`.
    fn heat_bar(
        &self,
        label: &str,
        fraction: f64,
        provenance: UsageProvenance,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let fraction = heat_bar_fraction(fraction, provenance);
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let size = appearance.ui_font_body();
        let muted = theme.sub_text_color(theme.surface_1()).into_solid();
        // Utilisation is not attention: one shared rule (spec v3 §1.2) — calm
        // theme text, with the theme error role only at the "fast voll" threshold.
        // The bar's fill carries the level; color only flags "nearly full".
        let bar_color = utilisation_coloru(fraction, appearance);
        let track = utilisation_track(fraction, HEAT_BAR_HEIGHT, bar_color, appearance);

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(
                ConstrainedBox::new(CockpitPanel::text(label.to_string(), family, size, muted))
                    .with_width(METER_LABEL_WIDTH)
                    .finish(),
            )
            .with_child(track)
            .with_child(CockpitPanel::text(
                heat_pct_label_with_provenance(fraction, provenance),
                family,
                size,
                bar_color,
            ))
            .with_main_axis_size(MainAxisSize::Max)
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
        let provider_size = appearance.ui_font_body_large();
        let identity_size = appearance.ui_font_footnote();
        let main = theme.main_text_color(theme.surface_1()).into_solid();
        let muted = theme.sub_text_color(theme.surface_1()).into_solid();
        let identity = account_identity(&acct.account);

        // Provider is the stable headline on every account surface, led by the
        // provider's own mark (icon in its theme color) so Claude and Codex
        // cards differ at a glance.
        let header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(PROVIDER_MARK_GAP)
            .with_child(provider_mark(
                identity.provider,
                PROVIDER_MARK_SIZE,
                appearance,
            ))
            .with_child(
                Shrinkable::new(
                    1.0,
                    CockpitPanel::text(
                        identity.provider_label.to_string(),
                        family,
                        provider_size,
                        main,
                    ),
                )
                .finish(),
            )
            .finish();

        let tooltip_state = self
            .identity_tooltip_states
            .get(&acct.account.key)
            .cloned()
            .unwrap_or_default();
        // The subline starts under the provider name, not under the mark.
        let subline = Container::new(account_identity_subline(
            identity.account,
            identity.plan,
            identity.email_hint,
            tooltip_state,
            identity_size,
            muted,
            appearance,
        ))
        .with_padding_left(PROVIDER_MARK_SIZE + PROVIDER_MARK_GAP)
        .finish();
        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(CARD_SPACING)
            .with_child(header)
            .with_child(subline);
        col = col
            .with_child(self.heat_bar(
                &crate::t!("cockpit-meter-5h"),
                acct.heat,
                acct.provenance_5h,
                appearance,
            ))
            .with_child(self.heat_bar(
                &crate::t!("cockpit-meter-week"),
                acct.heat_week,
                acct.provenance_week,
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
            ctx.dispatch_typed_action(CockpitAccountsPanelAction::SelectAccount(key.clone()))
        })
        .finish()
    }

    /// The „KI-KONTEN" header: label, account count and the „Alle Konten"
    /// control that opens the fleet pane (spec v3 §S1). Cross-account spend
    /// (today, 5 h, week) lives in that pane's summary rather than in the
    /// header, so the header stays one quiet line like the mockup.
    fn render_header(
        &self,
        health: &ScanHealth,
        account_count: usize,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let presentation = accounts_header_presentation(health, account_count);
        CockpitPanel::render_zone_header(
            crate::t!("cockpit-zone-accounts").to_string(),
            presentation.count,
            Some(ChildView::new(&self.fleet_overview_button).finish()),
            appearance.theme().surface_1(),
            appearance,
        )
    }
}

impl View for CockpitAccountsPanel {
    fn ui_name() -> &'static str {
        "CockpitAccountsPanel"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() && !CockpitModel::as_ref(ctx).snapshot().accounts.is_empty()
        {
            ctx.focus(&self.fleet_overview_button);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        // A disabled cockpit clears its snapshot to empty; the placeholder must say
        // "disabled", not "no accounts" (spec: the empty state is only for the real one).
        let enabled = *crate::cockpit::settings::CockpitSettings::as_ref(app).enabled;
        let snapshot = CockpitModel::as_ref(app).snapshot().clone();

        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);

        // A degraded scan with some accounts present: the list may be missing others
        // (e.g. a broken Codex sign-in). Warn above the accounts; the empty case shows
        // this in its own placeholder instead.
        if !snapshot.accounts.is_empty()
            && matches!(snapshot.health, zaplex_cockpit::ScanHealth::Degraded(_))
        {
            content = content.with_child(
                Container::new(self.render_scan_placeholder(&snapshot.health, enabled, appearance))
                    .with_uniform_padding(CARD_PADDING)
                    .with_margin_bottom(CARD_SPACING * 2.0)
                    .finish(),
            );
        }

        // One flat zone-card holding the fleet-usage header + one flat block per
        // account. No accounts → a calm hint under the header instead.
        if snapshot.accounts.is_empty() {
            // Keep the section header, but show zero only after a successful scan.
            // Pending or degraded discovery is unknown rather than empty.
            let empty = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(
                    Container::new(CockpitPanel::render_zone_header(
                        crate::t!("cockpit-zone-accounts").to_string(),
                        account_count_presentation(&snapshot.health, 0),
                        None,
                        theme.surface_2(),
                        appearance,
                    ))
                    .with_margin_bottom(CARD_SPACING * 2.0)
                    .finish(),
                )
                .with_child(self.render_scan_placeholder(&snapshot.health, enabled, appearance));
            content = content.with_child(
                Container::new(empty.finish())
                    .with_uniform_padding(CARD_PADDING)
                    .finish(),
            );
        } else {
            let selected = CockpitModel::as_ref(app)
                .selected_account()
                .map(str::to_string);
            let mut accounts = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(
                    Container::new(self.render_header(
                        &snapshot.health,
                        snapshot.accounts.len(),
                        appearance,
                    ))
                    .with_margin_bottom(CARD_SPACING * 2.0)
                    .finish(),
                );
            for acct in &snapshot.accounts {
                let is_selected = selected.as_deref() == Some(acct.account.key.as_str());
                accounts = accounts.with_child(self.render_card(acct, is_selected, appearance));
            }
            content = content.with_child(
                zone_card(accounts.finish(), appearance)
                    .with_uniform_padding(CARD_PADDING)
                    .finish(),
            );
        }

        // The accounts view owns the full sidebar height with its own scroll
        // state, independent of how many sessions the tree view holds (#504).
        let scroll = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            content.finish(),
            ScrollbarWidth::Auto,
            theme.disabled_text_color(theme.surface_2()).into(),
            theme.main_text_color(theme.surface_2()).into(),
            ElementFill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        // A column at full height carries the surface to the bottom of the
        // sidebar even when the content is shorter, like the other toolbelt views.
        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(Shrinkable::new(1.0, scroll).finish())
                .finish(),
        )
        .with_uniform_padding(CARD_PADDING)
        .with_background(theme.surface_2())
        .finish()
    }
}

impl Entity for CockpitAccountsPanel {
    type Event = CockpitAccountsPanelEvent;
}

/// Accounts-view actions (routed back into the view by the action system).
#[derive(Clone, Debug)]
pub enum CockpitAccountsPanelAction {
    OpenDashboardPane,
    /// Select an account (its `account.key`) → open (or focus) that account's
    /// own pane and carry a stable highlight in the sidebar. A second click
    /// focuses the pane; it does not de-select.
    SelectAccount(String),
    /// Re-run the account scan — the retry on the loading/scan-failed/empty
    /// placeholder.
    Rescan,
}

impl TypedActionView for CockpitAccountsPanel {
    type Action = CockpitAccountsPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CockpitAccountsPanelAction::OpenDashboardPane => {
                ctx.emit(CockpitAccountsPanelEvent::OpenCockpitPane(None));
            }
            CockpitAccountsPanelAction::SelectAccount(key) => {
                // Mark it selected (the sidebar highlight follows), then open —
                // or focus — that account's own pane. Clicking the same card
                // again lands here too and simply focuses the pane it already
                // has: the selection no longer toggles off underneath it.
                let key = key.clone();
                CockpitModel::handle(ctx)
                    .update(ctx, |model, ctx| model.select_account(key.clone(), ctx));
                ctx.emit(CockpitAccountsPanelEvent::OpenCockpitPane(Some(key)));
            }
            CockpitAccountsPanelAction::Rescan => {
                CockpitModel::handle(ctx).update(ctx, |model, ctx| model.rescan(ctx));
            }
        }
    }
}

#[cfg(test)]
#[path = "accounts_panel_tests.rs"]
mod tests;
