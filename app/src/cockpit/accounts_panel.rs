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
    Rect, ScrollbarWidth, Shrinkable, Text,
};
use warpui::{
    accessibility::{AccessibilityContent, WarpA11yRole},
    AppContext, BlurContext, Entity, FocusContext, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};
use zaplex_cockpit::{format_cost, heat_pct_label_with_provenance, AccountUsage, UsageProvenance};

use crate::cockpit::account_identity;
use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::panel::{account_count_presentation, CockpitPanel, CARD_PADDING, CARD_SPACING};
use crate::cockpit::style::{utilisation_coloru, utilisation_track, zone_card, BLOCK_RADIUS};
use crate::view_components::action_button::{ActionButton, ButtonSize, PaneHeaderTheme};

const HEAT_BAR_HEIGHT: f32 = 6.0;

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

enum FleetTotalButtonEvent {
    Activated,
}

#[derive(Clone, Debug)]
enum FleetTotalButtonAction {
    Activate,
}

fn is_fleet_total_activation_keystroke(keystroke: &warpui::keymap::Keystroke) -> bool {
    !keystroke.cmd
        && !keystroke.ctrl
        && !keystroke.alt
        && !keystroke.shift
        && !keystroke.meta
        && matches!(keystroke.key.as_str(), "enter" | "numpadenter" | " ")
}

struct FleetTotalButton {
    button: ViewHandle<ActionButton>,
    label: String,
    #[cfg(test)]
    activation_count: usize,
}

impl FleetTotalButton {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        let button = ctx.add_typed_action_view(|_| {
            ActionButton::new("", PaneHeaderTheme)
                .with_size(ButtonSize::XSmall)
                .on_click(|ctx| ctx.dispatch_typed_action(FleetTotalButtonAction::Activate))
        });
        Self {
            button,
            label: String::new(),
            #[cfg(test)]
            activation_count: 0,
        }
    }

    fn set_label(&mut self, label: String, ctx: &mut ViewContext<Self>) {
        self.label.clone_from(&label);
        self.button
            .update(ctx, |button, ctx| button.set_label(label, ctx));
        ctx.notify();
    }
}

impl Entity for FleetTotalButton {
    type Event = FleetTotalButtonEvent;
}

impl View for FleetTotalButton {
    fn ui_name() -> &'static str {
        "FleetTotalButton"
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
                if is_fleet_total_activation_keystroke(keystroke) {
                    ctx.dispatch_typed_action(FleetTotalButtonAction::Activate);
                    DispatchEventResult::StopPropagation
                } else {
                    DispatchEventResult::PropagateToParent
                }
            })
            .finish()
    }
}

impl TypedActionView for FleetTotalButton {
    type Action = FleetTotalButtonAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FleetTotalButtonAction::Activate => {
                #[cfg(test)]
                {
                    self.activation_count += 1;
                }
                ctx.focus_self();
                ctx.emit(FleetTotalButtonEvent::Activated);
            }
        }
    }
}

pub struct CockpitAccountsPanel {
    scroll_state: ClippedScrollStateHandle,
    /// Hover state per account card (key = account `key`). The whole card is a
    /// click target that opens the roomy dashboard pane.
    card_states: HashMap<String, MouseStateHandle>,
    /// Semantic button for the „KI-KONTEN" header's fleet total — the cross-account
    /// spend figure doubles as the entry point to the fleet pane (spec v3 §S1).
    fleet_total_button: ViewHandle<FleetTotalButton>,
    /// Hover/click state for the "try again" retry (the loading / scan-failed /
    /// empty placeholder). A **stable** handle: `Hoverable` tracks mouse-down in
    /// it, so a fresh one each render would drop the click.
    rescan_btn: MouseStateHandle,
}

impl CockpitAccountsPanel {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let fleet_total_button = ctx.add_typed_action_view(FleetTotalButton::new);
        ctx.subscribe_to_view(&fleet_total_button, |_, _, event, ctx| match event {
            FleetTotalButtonEvent::Activated => {
                ctx.dispatch_typed_action(&CockpitAccountsPanelAction::OpenDashboardPane)
            }
        });
        // Re-render on theme change and whenever the snapshot updates.
        ctx.subscribe_to_model(&Appearance::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&CockpitModel::handle(ctx), |me, _, event, ctx| {
            if matches!(event, CockpitEvent::Updated) {
                me.sync_card_states(ctx);
                me.sync_fleet_total_button(ctx);
                ctx.notify();
            }
        });
        let mut me = Self {
            scroll_state: ClippedScrollStateHandle::default(),
            card_states: HashMap::new(),
            fleet_total_button,
            rescan_btn: MouseStateHandle::default(),
        };
        me.sync_card_states(ctx);
        me.sync_fleet_total_button(ctx);
        me
    }

    fn sync_fleet_total_button(&self, ctx: &mut ViewContext<Self>) {
        let fleet_today = CockpitModel::as_ref(ctx)
            .snapshot()
            .accounts
            .iter()
            .map(|account| account.today.cost_usd)
            .sum::<f64>();
        let label = crate::t!(
            "cockpit-header-today-total",
            today = format_cost(fleet_today)
        );
        self.fleet_total_button
            .update(ctx, |button, ctx| button.set_label(label.to_string(), ctx));
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
        for key in acct_keys {
            self.card_states.entry(key).or_default();
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
        let muted = theme.sub_text_color(theme.surface_1()).into_solid();
        // Utilisation is not attention: one shared rule (spec v3 §1.2) — calm
        // theme text, with the theme error role only at the "fast voll" threshold.
        // The bar's fill carries the level; color only flags "nearly full".
        let bar_color = utilisation_coloru(fraction, appearance);
        let track = utilisation_track(fraction, HEAT_BAR_HEIGHT, bar_color, appearance);

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(CockpitPanel::text(label.to_string(), family, size, muted))
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

        // Provider is the stable headline on every account surface. The themed
        // accent mark is supplementary; the provider name remains visible.
        let header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(6.0)
            .with_child(
                ConstrainedBox::new(
                    Rect::new()
                        .with_background_color(theme.accent().into_solid())
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
                    CockpitPanel::text(identity.provider.to_string(), family, provider_size, main),
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
            col = col.with_child(CockpitPanel::identity_text(
                identity.subline,
                family,
                identity_size,
                muted,
            ));
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
            ctx.dispatch_typed_action(CockpitAccountsPanelAction::SelectAccount(key.clone()))
        })
        .finish()
    }

    /// The „KI-KONTEN" header: label + count, plus the **fleet total** — the one
    /// cross-account number (spec v3 §S1).
    ///
    /// The Maximize icon is gone, but the *fleet view it opened* is not: an
    /// account pane can only ever show its own account, so if this number and its
    /// entry point both vanished, cross-account spend would have no home at all —
    /// a regression, not a decluttering. The total therefore stays visible and
    /// **is itself the affordance**: clicking it opens the fleet pane. One
    /// element, two jobs, no extra chrome.
    fn render_header(&self, snapshot_len: usize, appearance: &Appearance) -> Box<dyn Element> {
        CockpitPanel::render_zone_header(
            crate::t!("cockpit-zone-accounts").to_string(),
            Some(snapshot_len),
            Some(ChildView::new(&self.fleet_total_button).finish()),
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
            ctx.focus(&self.fleet_total_button);
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
                    Container::new(self.render_header(snapshot.accounts.len(), appearance))
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

        Container::new(scroll)
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
