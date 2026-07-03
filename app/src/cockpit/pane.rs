//! Cockpit **main-area pane** (C2b) — the roomy dashboard over the
//! `zaplex_cockpit` data spine: aggregate header, per-account cards with the
//! full cost/token matrix (today / 5h block / week), both heat bars and reset
//! timers. The compact glanceable variant is the sidebar (`CockpitPanel`);
//! this pane is a first-class zaplex pane (tab/split/promotable,
//! multi-instance), opened from the sidebar's expand action. See
//! docs/superpowers/specs/2026-07-01-cockpit-native-integration-design.md §3.3.

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Fill as ElementFill, Flex, MainAxisAlignment, MainAxisSize,
    ParentElement, Radius, Rect, ScrollbarWidth, Shrinkable, Text,
};
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View, ViewContext,
};
use zaplex_cockpit::{
    format_cost, format_reset, format_tokens, heat_fill, heat_pct_label, AccountUsage, HeatLevel,
    SessionState, WindowTotals,
};

use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};

const PANE_PADDING: f32 = 16.0;
const CARD_PADDING: f32 = 12.0;
const CARD_SPACING: f32 = 8.0;
const HEAT_BAR_WIDTH: f32 = 160.0;
const HEAT_BAR_HEIGHT: f32 = 8.0;
/// Fixed column width for the cost/token matrix cells.
const MATRIX_COL_WIDTH: f32 = 110.0;

/// Heat band → display colour (kept in sync with `CockpitPanel::heat_coloru`;
/// reference palette lives in `zaplex_cockpit::HeatLevel::hex`).
fn heat_coloru(level: HeatLevel) -> ColorU {
    match level {
        HeatLevel::Ok => ColorU::from_u32(0x22C55EFF),
        HeatLevel::Elevated => ColorU::from_u32(0xEAB308FF),
        HeatLevel::High => ColorU::from_u32(0xFB923CFF),
        HeatLevel::Critical => ColorU::from_u32(0xF97316FF),
        HeatLevel::Over => ColorU::from_u32(0xEF4444FF),
    }
}

/// The dashboard view backing the cockpit pane.
pub struct CockpitPaneView {
    scroll_state: ClippedScrollStateHandle,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
}

impl CockpitPaneView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&Appearance::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&CockpitModel::handle(ctx), |_, _, event, ctx| {
            if matches!(event, CockpitEvent::Updated) {
                ctx.notify();
            }
        });
        let pane_configuration =
            ctx.add_model(|_ctx| PaneConfiguration::new(crate::t!("cockpit-pane-title")));
        Self {
            scroll_state: ClippedScrollStateHandle::default(),
            pane_configuration,
            focus_handle: None,
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

    /// A labelled heat bar (roomier than the sidebar variant).
    fn heat_bar(&self, label: &str, fraction: f64, appearance: &Appearance) -> Box<dyn Element> {
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
            .with_child(ConstrainedBox::new(Self::text(
                label.to_string(),
                family,
                size,
                muted,
            ))
            .with_width(24.0)
            .finish())
            .with_child(track)
            .with_child(Self::text(
                heat_pct_label(fraction),
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
                .with_child(Self::text(
                    format_tokens(totals.total),
                    family,
                    body,
                    muted,
                ))
                .finish(),
        )
        .with_width(MATRIX_COL_WIDTH)
        .finish()
    }

    /// A full account card: header (label + plan), both heat bars, the
    /// today/5h/week cost+token matrix, and the reset line.
    fn render_card(&self, acct: &AccountUsage, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let heading = appearance.ui_font_heading_3();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let accent = theme.accent().into_solid();
        let now = chrono::Utc::now();

        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.0)
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
            .with_child(self.heat_bar("5h", acct.heat, appearance))
            .with_child(self.heat_bar("wk", acct.heat_week, appearance))
            .with_child(matrix);
        // Live sessions (C3a), waiting-first (the spine pre-sorts): the
        // dashboard's job is surfacing what needs YOU.
        for session in acct.sessions.iter().take(4) {
            let (glyph, color) = match session.state {
                SessionState::Waiting => ("✋", heat_coloru(HeatLevel::Critical)),
                SessionState::Active => ("●", heat_coloru(HeatLevel::Ok)),
                SessionState::Monitor => ("◌", muted),
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
            col = col.with_child(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.0)
                    .with_child(Self::text(glyph.to_string(), family, body, color))
                    .with_child(
                        Shrinkable::new(1.0, Self::text(label, family, body, main)).finish(),
                    )
                    .with_main_axis_size(MainAxisSize::Max)
                    .finish(),
            );
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

    /// Aggregate header: account count + summed today/5h/week cost.
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

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(Self::text(
                if waiting > 0 {
                    format!(
                        "{} account{} · ✋ {waiting} waiting on you",
                        accounts.len(),
                        if accounts.len() == 1 { "" } else { "s" }
                    )
                } else {
                    format!(
                        "{} account{}",
                        accounts.len(),
                        if accounts.len() == 1 { "" } else { "s" }
                    )
                },
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
            for acct in &snapshot.accounts {
                col = col.with_child(self.render_card(acct, appearance));
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
    type Action = ();
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
