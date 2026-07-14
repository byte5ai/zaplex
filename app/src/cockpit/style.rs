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
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Flex,
    Hoverable, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds,
    Radius, Rect, Stack, Text,
};
use warpui::geometry::vector::vec2f;
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::Action;
use zaplex_cockpit::{HeatLevel, Provider, SessionState};

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

/// Corner radius of a flat sidebar **zone-card** (Hosts / AI-Accounts). One
/// calm container radius per the redesign spec §2.1 — rounder than the old 6px
/// per-account cards, so the two zones read as single surfaces.
pub const CARD_RADIUS: f32 = 12.0;

/// Hairline zone-card border width (spec §2.1: `0.5px border`, no shadow).
pub const CARD_BORDER: f32 = 0.5;

/// Fixed width of a session row's right-hand **metric column** (provider icon +
/// ctx%), so the metrics align into a clean column across rows and never shift
/// as the branch label grows or shrinks (spec §2.3). Narrow because it no longer
/// carries the model — that overflowed and broke the alignment.
pub const METRIC_COL_WIDTH: f32 = 58.0;

/// A flat sidebar **zone-card**: `surface_1`, a hairline border, radius 12 and
/// **no** shadow (spec §2.1). Emphasis comes from content + spacing, never from
/// heavy container chrome. Returns the still-open [`Container`] so the caller
/// adds its own padding / margin before `finish()`.
pub fn zone_card(child: Box<dyn Element>, appearance: &Appearance) -> Container {
    let theme = appearance.theme();
    Container::new(child)
        .with_background(theme.surface_1())
        .with_border(Border::all(CARD_BORDER).with_border_fill(theme.split_pane_border_color()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CARD_RADIUS)))
}

// The modal corner radius and scrim now live in the app-wide modal contract
// [`crate::ui_components::modal_frame`] — the single source shared by cockpit
// modals, `Modal<T>` and `Dialog` alike. The cockpit's Spawn-Karte and inbox
// consume them from there directly.

/// Heat band index into the two palettes (green→red order).
fn heat_index(level: HeatLevel) -> usize {
    match level {
        HeatLevel::Ok => 0,
        HeatLevel::Elevated => 1,
        HeatLevel::High => 2,
        HeatLevel::Critical => 3,
        HeatLevel::Over => 4,
    }
}

/// Heat palette tuned for a **dark** background — the bright Tailwind 400/500
/// tones that read on near-black (the shipped default theme).
const HEAT_ON_DARK: [u32; 5] = [
    0x22C55EFF, // Ok       — green-500
    0xEAB308FF, // Elevated — yellow-500
    0xFB923CFF, // High     — orange-400
    0xF97316FF, // Critical — orange-500
    0xEF4444FF, // Over     — red-500
];

/// Heat palette tuned for a **light** background — darker Tailwind 700/800
/// tones, because the bright dark-theme hues (yellow especially) wash out on
/// white. Same green→red semantics, kept legible (L1: a contrast-tested palette).
const HEAT_ON_LIGHT: [u32; 5] = [
    0x15803DFF, // Ok       — green-700
    0xA16207FF, // Elevated — yellow-700
    0xC2410CFF, // High     — orange-700
    0x9A3412FF, // Critical — orange-800
    0xB91C1CFF, // Over     — red-700
];

/// Heat band → display color for the **dark** default theme. The single source
/// for every heat-colored bar, percentage and badge; prefer
/// [`heat_coloru_on`] where the surface's lightness is known so the palette
/// stays legible on a light theme too.
pub fn heat_coloru(level: HeatLevel) -> ColorU {
    ColorU::from_u32(HEAT_ON_DARK[heat_index(level)])
}

/// Heat band → display color **chosen for the given background** (L1 semantic
/// palette): of the two tuned variants (bright dark-theme hue / darker
/// light-theme hue) it returns whichever has the **higher actual contrast**
/// against `bg`, so a status dot / meter fill stays as legible as possible on
/// either theme — and even on a mid-tone surface it never picks the worse of
/// the two (a naive luminance split could). `bg` is the surface the color
/// renders against (usually `theme.background().into_solid()`).
pub fn heat_coloru_on(level: HeatLevel, bg: ColorU) -> ColorU {
    let i = heat_index(level);
    let dark = ColorU::from_u32(HEAT_ON_DARK[i]);
    let light = ColorU::from_u32(HEAT_ON_LIGHT[i]);
    if contrast_ratio(dark, bg) >= contrast_ratio(light, bg) {
        dark
    } else {
        light
    }
}

/// WCAG relative luminance of a color (alpha ignored), `0.0` (black)..`1.0`
/// (white). The basis of the [`contrast_ratio`] contrast test that keeps the
/// heat palette legible on both themes.
pub fn relative_luminance(c: ColorU) -> f64 {
    fn chan(v: u8) -> f64 {
        let s = v as f64 / 255.0;
        if s <= 0.039_28 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * chan(c.r) + 0.7152 * chan(c.g) + 0.0722 * chan(c.b)
}

/// WCAG contrast ratio between two colors (`>= 1.0`). `3.0` is the AA threshold
/// for graphical objects (status dots, meter fills) — what the palette test
/// asserts the heat colors clear against each theme's background.
pub fn contrast_ratio(a: ColorU, b: ColorU) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The leading **provider icon** for a session / account row (spec §2.3, §2.5:
/// the provider icon leads). The icon font carries no brand marks, so this maps
/// each provider to the closest distinct monochrome glyph — a generic assistant
/// for Claude, the code glyph for Codex — enough to tell the two apart at a
/// glance without a fabricated logo. Rendered muted (it identifies, it doesn't
/// shout).
pub fn provider_icon(provider: Provider) -> icons::Icon {
    match provider {
        Provider::Claude => icons::Icon::AiAssistant,
        Provider::Codex => icons::Icon::Code2,
    }
}

/// The human provider name for the Provider slot of an account card (spec §2.4:
/// Provider and Plan are two separate slots). A proper noun, not translated.
pub fn provider_label(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
    }
}

/// The provider identity colour, shown ONLY on the account-card provider swatch
/// (spec §1: provider colours live in the KI-Konten cards + the pane table, never
/// the tree). Own marks, not the vendors' trademarked logos: Claude = clay,
/// Codex = blue.
pub fn provider_color(provider: Provider) -> ColorU {
    match provider {
        Provider::Claude => ColorU {
            r: 0xC8,
            g: 0x72,
            b: 0x4A,
            a: 0xFF,
        },
        Provider::Codex => ColorU {
            r: 0x3D,
            g: 0x9B,
            b: 0xF0,
            a: 0xFF,
        },
    }
}

/// The status-dot color for a session state on the given surface (spec §2.3:
/// status is carried by **color**, not a chrome glyph). Waiting is the one
/// attention band (amber), working rests in the calm "ok" green, idle fades to
/// the theme's muted sub-text. Uses [`heat_coloru_on`] so the dot stays legible
/// on a light theme.
pub fn status_dot_coloru(state: SessionState, appearance: &Appearance) -> ColorU {
    let theme = appearance.theme();
    let bg = theme.background().into_solid();
    match state {
        SessionState::Waiting => heat_coloru_on(HeatLevel::Critical, bg),
        SessionState::Active | SessionState::Monitor => heat_coloru_on(HeatLevel::Ok, bg),
        SessionState::Idle => theme.sub_text_color(theme.background()).into_solid(),
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

/// An icon verb button ([`icon_verb_button`]) **with a hover tooltip** — every
/// icon-only affordance on the cockpit surfaces should carry one so the meaning
/// is discoverable (a bare clickable glyph without a label is not). The tooltip
/// floats above the icon on hover only (respects the hover rule — it's an
/// overlay, it doesn't re-lay-out the row).
pub fn icon_verb_button_tooltip<A: Action + Clone>(
    state: MouseStateHandle,
    icon: icons::Icon,
    rest: Fill,
    hover: Fill,
    tooltip: impl Into<String>,
    appearance: &Appearance,
    action: A,
) -> Box<dyn Element> {
    let builder = appearance.ui_builder();
    let tooltip = tooltip.into();
    Hoverable::new(state, move |mouse| {
        let color = if mouse.is_hovered() { hover } else { rest };
        let icon_el = ConstrainedBox::new(icon.to_warpui_icon(color).finish())
            .with_width(GLYPH_COL_WIDTH)
            .with_height(GLYPH_COL_WIDTH)
            .finish();
        if !mouse.is_hovered() {
            return icon_el;
        }
        let mut stack = Stack::new();
        stack.add_child(icon_el);
        stack.add_positioned_overlay_child(
            builder.tool_tip(tooltip.clone()).build().finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0.0, -6.0),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::TopMiddle,
                ChildAnchor::BottomMiddle,
            ),
        );
        stack.finish()
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
    // Utilisation is NOT an attention signal: it stays muted grey and only turns
    // red when the window is nearly full (≥ 90 %). This keeps amber exclusive to
    // the waiting state — a full heat gradient here would compete with it (spec
    // §1: one encoding per data type).
    let _ = fill;
    let color = if pct >= 90 {
        heat_coloru(HeatLevel::Critical)
    } else {
        appearance
            .theme()
            .sub_text_color(appearance.theme().background())
            .into_solid()
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: ColorU = ColorU {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
    const BLACK: ColorU = ColorU {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    /// A representative near-black terminal background (the shipped dark theme
    /// sits around here); the heat palette is validated against it.
    const NEAR_BLACK: ColorU = ColorU {
        r: 20,
        g: 20,
        b: 20,
        a: 255,
    };

    const LEVELS: [HeatLevel; 5] = [
        HeatLevel::Ok,
        HeatLevel::Elevated,
        HeatLevel::High,
        HeatLevel::Critical,
        HeatLevel::Over,
    ];

    #[test]
    fn contrast_ratio_is_symmetric_and_bounded() {
        // Black on white is the canonical 21:1 maximum; a color against itself
        // is 1:1. Order must not matter.
        assert!((contrast_ratio(BLACK, WHITE) - 21.0).abs() < 0.05);
        assert!((contrast_ratio(WHITE, BLACK) - 21.0).abs() < 0.05);
        assert!((contrast_ratio(WHITE, WHITE) - 1.0).abs() < 1e-9);
    }

    /// L1 contrast test: every heat band must clear the WCAG 3:1 graphical-object
    /// threshold against BOTH theme backgrounds when picked via [`heat_coloru_on`]
    /// — otherwise a status dot or meter fill washes out on one theme.
    #[test]
    fn heat_palette_legible_on_both_themes() {
        for level in LEVELS {
            let on_light = heat_coloru_on(level, WHITE);
            let cr_light = contrast_ratio(on_light, WHITE);
            assert!(
                cr_light >= 3.0,
                "{level:?} washes out on a light surface (contrast {cr_light:.2})"
            );

            let on_dark = heat_coloru_on(level, NEAR_BLACK);
            let cr_dark = contrast_ratio(on_dark, NEAR_BLACK);
            assert!(
                cr_dark >= 3.0,
                "{level:?} washes out on a dark surface (contrast {cr_dark:.2})"
            );
        }
    }

    /// The dark and light tables are genuinely different (the light variant is
    /// the darker, more saturated tone) — a regression that collapsed them would
    /// silently reintroduce the wash-out.
    #[test]
    fn heat_on_light_differs_from_dark_default() {
        for level in LEVELS {
            assert_ne!(heat_coloru_on(level, WHITE), heat_coloru(level));
            assert_eq!(heat_coloru_on(level, NEAR_BLACK), heat_coloru(level));
        }
    }
}
