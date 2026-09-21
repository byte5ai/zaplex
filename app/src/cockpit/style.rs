//! Cockpit **style vocabulary** — the shared set of domain-specific visual
//! tokens the cockpit surfaces render with (Step 9, visual design pass).
//!
//! The pane, the sidebar, the Spawn-Karte, the attention inbox and the titlebar
//! pulse all draw the same things: status glyphs, heat-colored percentages,
//! muted-at-rest verb clusters, modal chrome. Before this module each surface
//! carried its own inline copy (two `heat_coloru` duplicates, eight hand-rolled
//! hover-verb closures, two different modal scrims). Centralizing them here,
//! while using app-wide components such as `CompactRowAction` for generic
//! compact-list controls, makes the cockpit read as *one* calm language:
//!
//! - **Calm by default**: verbs rest in the muted sub-text color and only take
//!   their accent on hover; destructive verbs hover into the single attention
//!   color instead of shouting at rest.
//! - **Exactly one attention role**: [`attention_coloru`] — the theme warning
//!   color every `✋` and every destructive hover shares, everywhere.
//! - **Aligned columns**: [`glyph_cell`] gives every status glyph the same
//!   fixed-width leading cell so row labels line up across pane and sidebar.
//!
//! Pure presentation: these helpers build styled elements/colors only — every
//! click still dispatches the caller's existing action unchanged.

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element,
    Empty, Expanded, Flex, Hoverable, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Rect, Stack, Text,
};
use warpui::geometry::vector::vec2f;
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::Action;
use zaplex_cockpit::{heat_fill, Provider, SessionState};

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

/// Corner radius of a multi-line interactive BLOCK inside a zone-card (an
/// account block in the sidebar, an account card in the pane). The container
/// radius scale is 12 → 8 → 4 — zone-card, block, row/control — one radius per
/// size class, never one per zone (polish audit 2026-07-18, P1.1).
pub const BLOCK_RADIUS: f32 = 8.0;

/// Corner radius of one-LINE interactive rows (host row, session row) and of
/// small controls (inputs, chips, badges). See [`BLOCK_RADIUS`] for the scale.
pub const CONTROL_RADIUS: f32 = 4.0;

/// Vertical padding of a one-line interactive row — what makes its hover fill
/// read as a surface instead of a text-height sliver.
pub const ROW_V_PADDING: f32 = 3.0;
/// Horizontal padding of a one-line interactive row.
pub const ROW_H_PADDING: f32 = 6.0;

/// Wraps one already-assembled row LINE as an interactive row surface: row
/// padding, control radius, and — when `hovered` — the shared `fg_overlay_1`
/// hover fill. The ONE hover grammar for host and session rows; pass
/// `hovered = false` for a non-interactive sibling so its text still aligns
/// with the interactive rows around it. (Polish audit 2026-07-18: a row that
/// hovers as a text-width sliver, or not at all, is exactly the inconsistency
/// this bans.)
pub fn hover_row(
    line: Box<dyn Element>,
    hovered: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let mut c = Container::new(line)
        .with_padding_top(ROW_V_PADDING)
        .with_padding_bottom(ROW_V_PADDING)
        .with_padding_left(ROW_H_PADDING)
        .with_padding_right(ROW_H_PADDING)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CONTROL_RADIUS)));
    if hovered {
        c = c.with_background(internal_colors::fg_overlay_1(theme));
    }
    c.finish()
}

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

/// The leading **provider icon** for a session / account row (spec §2.3, §2.5:
/// the provider icon leads). The icon font carries no brand marks, so this maps
/// each provider to a distinct bundled glyph. Rendered muted (it identifies, it
/// doesn't shout).
pub fn provider_icon(provider: Provider) -> icons::Icon {
    match provider {
        Provider::Claude => icons::Icon::AiAssistant,
        Provider::Codex => icons::Icon::Code2,
        Provider::Antigravity => icons::Icon::AntigravityLogo,
    }
}

/// The human provider name for the Provider slot of an account card (spec §2.4:
/// Provider and Plan are two separate slots). A proper noun, not translated.
pub fn provider_label(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
        Provider::Antigravity => "Antigravity",
    }
}

/// The status-dot color for a session state on the given surface (spec §2.3:
/// shape carries the state and the existing semantic theme roles reinforce it.
pub fn status_dot_coloru(state: SessionState, appearance: &Appearance) -> ColorU {
    let theme = appearance.theme();
    match state {
        SessionState::Waiting => theme.ui_warning_color(),
        SessionState::Active | SessionState::Monitor => theme.ui_green_color(),
        SessionState::Idle => theme.sub_text_color(theme.background()).into_solid(),
    }
}

/// The **one** attention accent — the theme warning role. Every waiting mark
/// (conductor rows, host badges, inbox rows, titlebar pulse) and every
/// destructive hover uses exactly this color; everything else stays quiet so
/// this is the only thing that draws the eye.
///
/// **Never use this for utilisation** — see [`utilisation_coloru`]. Amber is
/// reserved for "needs you" (spec v3 §1.3).
///
pub fn attention_coloru(appearance: &Appearance) -> ColorU {
    appearance.theme().ui_warning_color()
}

/// The translucent companion of [`attention_coloru`] used for the static
/// pane/tab halo. It deliberately keeps the same theme-adapted hue while
/// remaining quiet enough that only the solid border carries contrast.
pub fn attention_halo_coloru(appearance: &Appearance) -> ColorU {
    let mut color = attention_coloru(appearance);
    color.a = 38;
    color
}

/// The **one** utilisation threshold — "fast voll" (spec v3 §1.2).
/// This is a *visual* threshold: the plexing router (`zaplex_cockpit::routing`)
/// deprioritises fuller/working accounts by its own binding-window score rather
/// than hard-skipping at this number — the UI shows "fast voll" here; routing is
/// its own contract (spec v3 §5/X1).
pub const NEARLY_FULL: f64 = 0.85;

/// Colour for a **utilisation** readout (context fill, 5h/week meters): the
/// theme's muted text below [`NEARLY_FULL`] and its semantic error role at or
/// above it. No product-specific palette is introduced.
pub fn utilisation_coloru(fraction: f64, appearance: &Appearance) -> ColorU {
    let theme = appearance.theme();
    if fraction >= NEARLY_FULL {
        theme.ui_error_color()
    } else {
        theme.sub_text_color(theme.background()).into_solid()
    }
}

/// A full-width utilisation track for account cards. The track is the flexible
/// slot between its label and percentage; only its painted fill is proportional.
/// This keeps both stacked windows aligned at every panel width without a fixed
/// 90/160 px meter contract.
pub fn utilisation_track(
    fraction: f64,
    height: f32,
    color: ColorU,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let fill = heat_fill(fraction) as f32;
    let inner: Box<dyn Element> = if fill <= 0.0 {
        Empty::new().finish()
    } else if fill >= 1.0 {
        Rect::new().with_background_color(color).finish()
    } else {
        Flex::row()
            .with_child(
                Expanded::new(
                    fill,
                    Rect::new()
                        .with_background_color(color)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(height * 0.5)))
                        .finish(),
                )
                .finish(),
            )
            .with_child(Expanded::new(1.0 - fill, Empty::new().finish()).finish())
            .finish()
    };
    Expanded::new(
        1.0,
        ConstrainedBox::new(
            Container::new(inner)
                .with_background(internal_colors::fg_overlay_1(appearance.theme()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(height * 0.5)))
                .finish(),
        )
        .with_height(height)
        .finish(),
    )
    .finish()
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
        VerbKind::Destructive => attention_coloru(appearance),
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

/// A Cockpit-domain icon verb ([`icon_verb_button`]) **with a hover tooltip**.
/// Use it for pane and zone affordances that follow the 14 px glyph grid;
/// compact repeated rows use the app-wide `CompactRowAction` and its fixed
/// 20 px action slot. The tooltip is an overlay and never re-lays out the row.
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
        VerbKind::Destructive => attention_coloru(appearance),
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
    // Utilisation is NOT an attention signal — one shared rule decides the colour
    // (spec v3 §1.2): muted grey, true red only when nearly full. See
    // `utilisation_coloru` for why this must not be the Critical amber.
    let color = utilisation_coloru(fill, appearance);
    // German typography: a narrow no-break space before the percent sign
    // (spec v3 §7). U+202F keeps "42 %" from wrapping between number and sign.
    let label = if verbose {
        format!("· {pct}\u{202f}% ctx")
    } else {
        format!("· {pct}\u{202f}%")
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
#[path = "style_tests.rs"]
mod tests;
