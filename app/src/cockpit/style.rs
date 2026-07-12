//! Cockpit **style vocabulary** — the one shared set of visual tokens the
//! cockpit surfaces render with (Step 9, visual design pass).
//!
//! The pane, the sidebar, the Spawn-Karte, the attention inbox and the titlebar
//! pulse all draw the same things: status glyphs, heat-colored percentages,
//! muted-at-rest verb clusters, modal chrome. Before this module each surface
//! carried its own inline copy (two `heat_coloru` duplicates, eight hand-rolled
//! hover-verb closures, two different modal scrims). Centralizing them here is
//! what makes the cockpit read as *one* calm language:
//!
//! - **Calm by default**: verbs rest in the muted sub-text color and only take
//!   their accent on hover; destructive verbs hover into the single attention
//!   color instead of shouting at rest.
//! - **Exactly one attention accent**: [`attention_coloru`] — the Critical
//!   amber every `✋` and every destructive hover shares, everywhere.
//! - **Aligned columns**: [`glyph_cell`] gives every status glyph the same
//!   fixed-width leading cell so row labels line up across pane and sidebar.
//!
//! Pure presentation: these helpers build styled elements/colors only — every
//! click still dispatches the caller's existing action unchanged.

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    ConstrainedBox, CrossAxisAlignment, Element, Flex, Hoverable, MouseStateHandle, ParentElement,
    Rect, Text,
};
use warpui::platform::Cursor;
use warpui::Action;
use zaplex_cockpit::HeatLevel;

use crate::ui_components::icons;

/// Fixed width of the leading status-glyph cell on a session row, so labels
/// align into a clean column regardless of glyph metrics (`●` vs `✋` vs `◦`).
pub const GLYPH_COL_WIDTH: f32 = 14.0;

/// Spacing between verbs inside one cluster (review / guardrail / lever) —
/// tight enough to read as one toolbelt segment.
pub const VERB_SPACING: f32 = 8.0;

/// Gap between a row's info span and its trailing verb toolbelt — a touch
/// wider than the in-cluster spacing so content and controls stay distinct.
pub const INFO_VERBS_GAP: f32 = 12.0;

// The modal corner radius and scrim now live in the app-wide modal contract
// [`crate::ui_components::modal_frame`] — the single source shared by cockpit
// modals, `Modal<T>` and `Dialog` alike. The cockpit's Spawn-Karte and inbox
// consume them from there directly.

/// Heat band → display color (`ColorU` form of the reference palette in
/// `zaplex_cockpit::HeatLevel::hex`). The single source for every heat-colored
/// bar, percentage and badge in the cockpit; the hues hold up on both light
/// and dark backgrounds.
pub fn heat_coloru(level: HeatLevel) -> ColorU {
    match level {
        HeatLevel::Ok => ColorU::from_u32(0x22C55EFF),
        HeatLevel::Elevated => ColorU::from_u32(0xEAB308FF),
        HeatLevel::High => ColorU::from_u32(0xFB923CFF),
        HeatLevel::Critical => ColorU::from_u32(0xF97316FF),
        HeatLevel::Over => ColorU::from_u32(0xEF4444FF),
    }
}

/// The **one** attention accent — the Critical amber. Every `✋` (conductor
/// rows, host/project badges, inbox rows, titlebar pulse) and every destructive
/// hover uses exactly this color; everything else stays quiet so this is the
/// only thing that draws the eye.
pub fn attention_coloru() -> ColorU {
    heat_coloru(HeatLevel::Critical)
}

/// How a verb colors on hover. At rest every verb is equally muted — the
/// difference only shows when the user reaches for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerbKind {
    /// Constructive verbs (attach, fork, review, `+`…): muted → theme accent.
    Constructive,
    /// Destructive verbs (stop, kill, stop-all): muted → attention amber.
    Destructive,
}

/// One text verb: muted at rest, `kind`-colored on hover, pointing-hand cursor,
/// dispatching `action` on click. The single building block of every verb
/// cluster on pane and sidebar (identical rest/hover behavior everywhere).
pub fn verb_button<A: Action + Clone>(
    state: MouseStateHandle,
    label: impl Into<String>,
    kind: VerbKind,
    appearance: &Appearance,
    action: A,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let rest = theme.sub_text_color(theme.background()).into_solid();
    let hover = match kind {
        VerbKind::Constructive => theme.accent().into_solid(),
        VerbKind::Destructive => attention_coloru(),
    };
    verb_button_colored(state, label, rest, hover, appearance, action)
}

/// [`verb_button`] with explicit rest/hover colors — for the rare verb whose
/// rest state carries meaning (e.g. "✓ reviewed" resting in accent).
pub fn verb_button_colored<A: Action + Clone>(
    state: MouseStateHandle,
    label: impl Into<String>,
    rest: ColorU,
    hover: ColorU,
    appearance: &Appearance,
    action: A,
) -> Box<dyn Element> {
    let family = appearance.ui_font_family();
    let size = appearance.ui_font_body();
    let label = label.into();
    Hoverable::new(state, move |mouse| {
        let color = if mouse.is_hovered() { hover } else { rest };
        Text::new_inline(label, family, size)
            .with_color(color)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
    .finish()
}

/// An **icon-font** verb button (premium icon pass, #107): a monochrome
/// `icons::Icon` in place of a text glyph, resting in `rest` and taking `hover`
/// on hover. Same interaction contract as [`verb_button`]; sized to the shared
/// glyph column so it aligns with the status dots.
pub fn icon_verb_button<A: Action + Clone>(
    state: MouseStateHandle,
    icon: icons::Icon,
    rest: Fill,
    hover: Fill,
    action: A,
) -> Box<dyn Element> {
    Hoverable::new(state, move |mouse| {
        let color = if mouse.is_hovered() { hover } else { rest };
        ConstrainedBox::new(icon.to_warpui_icon(color).finish())
            .with_width(GLYPH_COL_WIDTH)
            .with_height(GLYPH_COL_WIDTH)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
    .finish()
}

/// An **icon-font** verb with a trailing word — the icon-pass form of a labeled
/// text verb like the old `review` / `fork` / `log` (#107 icon pass).
/// A monochrome [`icons::Icon`] followed by `label`, both muted at rest and
/// `kind`-colored on hover, dispatching `action` on click. Same interaction
/// contract as [`verb_button`]; the icon sits in the shared glyph column so it
/// aligns with status dots and the icon-only verbs.
pub fn icon_word_verb<A: Action + Clone>(
    state: MouseStateHandle,
    icon: icons::Icon,
    label: impl Into<String>,
    kind: VerbKind,
    appearance: &Appearance,
    action: A,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let rest = theme.sub_text_color(theme.background()).into_solid();
    let hover = match kind {
        VerbKind::Constructive => theme.accent().into_solid(),
        VerbKind::Destructive => attention_coloru(),
    };
    let family = appearance.ui_font_family();
    let size = appearance.ui_font_body();
    let label = label.into();
    Hoverable::new(state, move |mouse| {
        let color = if mouse.is_hovered() { hover } else { rest };
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.0)
            .with_child(
                ConstrainedBox::new(icon.to_warpui_icon(Fill::Solid(color)).finish())
                    .with_width(GLYPH_COL_WIDTH)
                    .with_height(GLYPH_COL_WIDTH)
                    .finish(),
            )
            .with_child(
                Text::new_inline(label.clone(), family, size)
                    .with_color(color)
                    .finish(),
            )
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
    .finish()
}

/// A hairline divider between verb clusters, so review · guardrail · lever
/// read as tidy toolbelt segments rather than one run of loose glyphs. Drawn
/// from the theme's muted text at low alpha — barely there, theme-correct.
pub fn cluster_divider(appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let mut c = theme.sub_text_color(theme.background()).into_solid();
    c.a = 64;
    ConstrainedBox::new(Rect::new().with_background_color(c).finish())
        .with_width(1.0)
        .with_height(10.0)
        .finish()
}

/// The fixed-width leading status-glyph cell of a session row — one glyph,
/// one color, one column width on every surface, so labels align vertically.
pub fn glyph_cell(glyph: &str, color: ColorU, appearance: &Appearance) -> Box<dyn Element> {
    ConstrainedBox::new(
        Text::new_inline(
            glyph.to_string(),
            appearance.ui_font_family(),
            appearance.ui_font_body(),
        )
        .with_color(color)
        .finish(),
    )
    .with_width(GLYPH_COL_WIDTH)
    .finish()
}

/// The context-window fill readout (`· 42% ctx` roomy / `· 42%` compact),
/// heat-colored by how full the window is — the same small element wherever a
/// context percentage appears.
pub fn ctx_pct_element(
    pct: u32,
    fill: f64,
    verbose: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let color = heat_coloru(HeatLevel::from_fraction(fill));
    let label = if verbose {
        format!("· {pct}% ctx")
    } else {
        format!("· {pct}%")
    };
    Text::new_inline(
        label,
        appearance.ui_font_family(),
        appearance.ui_font_body(),
    )
    .with_color(color)
    .finish()
}
