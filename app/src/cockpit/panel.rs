//! `CockpitPanel` — the cockpit **sidebar** (left toolbelt tab): a compact, glanceable
//! list of account cards over the `zaplex_cockpit` data spine. Read-only in C2; the
//! live-session quick-list + quick-launch land in later increments (see the cockpit
//! native-integration design doc). The roomy full dashboard is the main-area pane (C2b).

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Fill as ElementFill, Flex, MainAxisAlignment, MainAxisSize,
    ParentElement, Radius, Rect, ScrollbarWidth, Shrinkable, Text,
};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};
use zaplex_cockpit::{
    format_cost, format_reset, format_tokens, heat_fill, heat_pct_label, AccountUsage, HeatLevel,
};

use crate::cockpit::model::{CockpitEvent, CockpitModel};

const CARD_PADDING: f32 = 8.0;
const CARD_SPACING: f32 = 4.0;
const HEAT_BAR_WIDTH: f32 = 90.0;
const HEAT_BAR_HEIGHT: f32 = 6.0;

/// Maps a heat band to its display colour (reference palette lives in
/// `zaplex_cockpit::HeatLevel::hex`; kept in sync here as `ColorU`).
fn heat_coloru(level: HeatLevel) -> ColorU {
    match level {
        HeatLevel::Ok => ColorU::from_u32(0x22C55EFF),
        HeatLevel::Elevated => ColorU::from_u32(0xEAB308FF),
        HeatLevel::High => ColorU::from_u32(0xFB923CFF),
        HeatLevel::Critical => ColorU::from_u32(0xF97316FF),
        HeatLevel::Over => ColorU::from_u32(0xEF4444FF),
    }
}

pub struct CockpitPanel {
    scroll_state: ClippedScrollStateHandle,
}

impl CockpitPanel {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        // Re-render on theme change and whenever the snapshot updates.
        ctx.subscribe_to_model(&Appearance::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&CockpitModel::handle(ctx), |_, _, event, ctx| {
            let CockpitEvent::Updated = event;
            ctx.notify();
        });
        Self {
            scroll_state: ClippedScrollStateHandle::default(),
        }
    }

    fn text(s: String, family: warpui::fonts::FamilyId, size: f32, color: ColorU) -> Box<dyn Element> {
        Text::new_inline(s, family, size).with_color(color).finish()
    }

    /// A labelled heat bar: `5h [▓▓▓░░] 62%`, coloured by band.
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
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.0)))
                .finish(),
        )
        .with_width(HEAT_BAR_WIDTH)
        .with_height(HEAT_BAR_HEIGHT)
        .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(Self::text(label.to_string(), family, size, muted))
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

    fn render_card(&self, acct: &AccountUsage, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let sub = appearance.ui_font_subheading();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let accent = theme.accent().into_solid();
        let now = chrono::Utc::now();

        // Header: label (bold-ish subheading) + optional plan badge.
        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(6.0)
            .with_child(
                Shrinkable::new(1.0, Self::text(acct.account.label.clone(), family, sub, main))
                    .finish(),
            );
        if let Some(plan) = &acct.account.plan_tier {
            header = header.with_child(
                Container::new(Self::text(plan.clone(), family, body, accent))
                    .with_padding_left(6.0)
                    .with_padding_right(6.0)
                    .with_padding_top(1.0)
                    .with_padding_bottom(1.0)
                    .with_background(internal_colors::fg_overlay_1(theme))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                    .finish(),
            );
        }

        let cost_line = format!(
            "today {} · {}",
            format_cost(acct.today.cost_usd),
            format_tokens(acct.today.total)
        );

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
            .with_child(Self::text(cost_line, family, body, muted));
        if let Some(reset_line) = reset_line {
            col = col.with_child(Self::text(reset_line, family, body, muted));
        }

        Container::new(col.finish())
            .with_uniform_padding(CARD_PADDING)
            .with_margin_bottom(CARD_SPACING)
            .with_background(internal_colors::fg_overlay_1(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.0)))
            .finish()
    }

    fn render_header(&self, snapshot_len: usize, cost5h: f64, cost_wk: f64, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let sub = appearance.ui_font_subheading();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(Self::text(
                format!("{} account{}", snapshot_len, if snapshot_len == 1 { "" } else { "s" }),
                family,
                sub,
                main,
            ))
            .with_child(Self::text(
                format!("{} 5h · {} wk", format_cost(cost5h), format_cost(cost_wk)),
                family,
                body,
                muted,
            ))
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
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        let snapshot = CockpitModel::as_ref(app).snapshot().clone();

        let body_el: Box<dyn Element> = if snapshot.accounts.is_empty() {
            Container::new(Self::text(
                crate::t!("workspace-left-panel-cockpit-empty"),
                family,
                body,
                muted,
            ))
            .with_uniform_padding(CARD_PADDING)
            .finish()
        } else {
            let cost5h: f64 = snapshot.accounts.iter().map(|a| a.block5h.cost_usd).sum();
            let cost_wk: f64 = snapshot.accounts.iter().map(|a| a.week.cost_usd).sum();

            let mut cards = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(
                    Container::new(self.render_header(
                        snapshot.accounts.len(),
                        cost5h,
                        cost_wk,
                        appearance,
                    ))
                    .with_margin_bottom(CARD_SPACING * 2.0)
                    .finish(),
                );
            for acct in &snapshot.accounts {
                cards = cards.with_child(self.render_card(acct, appearance));
            }

            ClippedScrollable::vertical(
                self.scroll_state.clone(),
                cards.finish(),
                ScrollbarWidth::Auto,
                theme.disabled_text_color(theme.background()).into(),
                theme.main_text_color(theme.background()).into(),
                ElementFill::None,
            )
            .with_overlayed_scrollbar()
            .finish()
        };

        Container::new(body_el)
            .with_uniform_padding(CARD_PADDING)
            .finish()
    }
}

impl Entity for CockpitPanel {
    type Event = ();
}

impl TypedActionView for CockpitPanel {
    type Action = ();
}
