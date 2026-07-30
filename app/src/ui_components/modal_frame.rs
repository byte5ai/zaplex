//! # The one modal contract
//!
//! Single source of truth for modal/dialog **chrome**: the scrim behind every
//! modal, the geometry band (radius / width / padding / title typography) and
//! the reusable frame builders (card, centered overlay, header row, close ✕).
//!
//! Before this module the app carried four parallel modal stacks with divergent
//! chrome — three different scrims (`ColorU(0,0,0,179)`, `ColorU(18,18,18,128)`,
//! a blurred overlay), two radii (8 vs 10) and four widths (420 / 440 / 460 /
//! 480). The cockpit's Spawn-Karte and Attention-Inbox each hand-rolled their
//! own overlay, header and close button; the tab-config modals rebuilt a header
//! inside the generic `Modal<T>` shell. That is exactly the fragmentation the RC
//! master plan (WS1) closes: **one** scrim, **one** radius, **one** width band,
//! **one** header/close/footer grammar — defined here and consumed everywhere.
//!
//! Pure presentation: these helpers build styled elements only. Each modal keeps
//! its own state and dispatches its own actions; the frame just gives them the
//! identical premium shell.

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::{phenomenon::PhenomenonStyle, Fill};
use warpui::elements::{
    Align, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Dismiss, DropShadow, Element, Flex, MainAxisSize, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Shrinkable, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{Action, AppContext};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ActionButtonTheme, ButtonSize};

/// The one corner radius for every modal card. Unified from the old split
/// (`Modal`/`Dialog` = 8, cockpit = 10) to the premium cockpit value.
pub const MODAL_RADIUS: f32 = 10.0;

/// Standard modal width — dialogs, confirmations, tab-config modals.
pub const MODAL_WIDTH_STANDARD: f32 = 440.0;

/// Wide modal width — the content-rich cockpit modals (Spawn-Karte, inbox).
pub const MODAL_WIDTH_WIDE: f32 = 480.0;

/// Uniform interior padding of a modal card.
pub const MODAL_PADDING: f32 = 24.0;

/// Title size in a modal header (unified from spawn 20 / inbox 16).
pub const MODAL_TITLE_SIZE: f32 = 18.0;

/// Subtitle size in a modal header.
pub const MODAL_SUBTITLE_SIZE: f32 = 12.0;

/// The **one** scrim behind every modal — a calm dark veil, identical whether it
/// sits behind a cockpit modal, a `Modal<T>` or a `Dialog`. The single source;
/// `cockpit::style::modal_scrim` re-exports this.
pub fn modal_scrim() -> ColorU {
    ColorU::new(18, 18, 18, 128)
}

/// Shared backdrop policy for every modal that can hold unsaved input. A
/// backdrop click is absorbed, never translated into a close action.
pub fn unsaved_input_dismiss_action<A>() -> Option<A> {
    None
}

/// The one modal close ✕ — an [`ActionButton`] with [`Icon::X`], small, that
/// dispatches `action` on click. Each modal adds this as a child view (so it
/// carries the caller's dispatch) and places it via [`modal_header`], giving
/// every modal the identical corner ✕.
pub fn close_button<A: Action + Clone + 'static>(action: A) -> ActionButton {
    ActionButton::new("", CloseButtonTheme)
        .with_icon(Icon::X)
        .with_size(ButtonSize::Small)
        .on_click(move |ctx| ctx.dispatch_typed_action(action.clone()))
}

/// The shared theming for the modal close ✕: transparent at rest, a calm hover
/// wash, the muted modal-chrome text color. One definition for every modal.
struct CloseButtonTheme;

impl ActionButtonTheme for CloseButtonTheme {
    fn background(&self, hovered: bool, _appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(Fill::Solid(PhenomenonStyle::modal_close_button_hover()))
        } else {
            None
        }
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        _appearance: &Appearance,
    ) -> ColorU {
        PhenomenonStyle::modal_close_button_text()
    }
}

/// The one modal header: `title` (+ optional `subtitle`) resting on the left, a
/// close ✕ pinned right. `close` is the caller's ✕ element (built from
/// [`close_button`] and rendered as a `ChildView`) so the header owns the layout
/// while the caller owns the dispatch.
pub fn modal_header(
    title: impl Into<String>,
    subtitle: Option<String>,
    close: Box<dyn Element>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let family = appearance.ui_font_family();
    let theme = appearance.theme();
    let main = theme.main_text_color(theme.background()).into_solid();
    let muted = theme.sub_text_color(theme.background()).into_solid();

    let mut text_col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_spacing(2.)
        .with_child(
            Text::new(title.into(), family, MODAL_TITLE_SIZE)
                .with_color(main)
                .with_style(Properties::default().weight(Weight::Semibold))
                .finish(),
        );
    if let Some(sub) = subtitle {
        // `Text::new` (soft-wrapping) rather than `new_inline`: subtitles can be a
        // full sentence (e.g. the session-config intro) and must wrap beside the
        // ✕ instead of clipping.
        text_col.add_child(
            Text::new(sub, family, MODAL_SUBTITLE_SIZE)
                .with_color(muted)
                .finish(),
        );
    }

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(Shrinkable::new(1.0, text_col.finish()).finish())
        .with_child(close)
        .finish()
}

/// Wrap `body` in the standard modal **card**: uniform padding, themed
/// background, the one corner radius, a hairline border and a drop shadow, sized
/// to `width`. The single card surface behind every migrated modal.
pub fn modal_card(body: Box<dyn Element>, width: f32, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    ConstrainedBox::new(
        Container::new(body)
            .with_uniform_padding(MODAL_PADDING)
            .with_background(theme.background())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(MODAL_RADIUS)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_drop_shadow(DropShadow::default())
            .finish(),
    )
    .with_width(width)
    .finish()
}

/// Center `card` on screen behind the one modal [`modal_scrim`]. When
/// `dismiss_on_click` is `Some(action)`, a click on the scrim (outside the card)
/// dispatches it — the single backdrop-dismiss policy. Read-only modals pass the
/// close action; modals holding unsaved input pass `None` so a stray click can
/// never discard their state.
pub fn modal_overlay<A: Action + Clone + 'static>(
    card: Box<dyn Element>,
    dismiss_on_click: Option<A>,
    app: &AppContext,
) -> Box<dyn Element> {
    // Always wrap the card in a `Dismiss` that blocks interaction with the
    // workspace behind the modal (a stray backdrop click must never reach a
    // control underneath the open modal). Whether that click also *dismisses* is
    // the policy knob: read-only modals dispatch their close action; modals with
    // unsaved input absorb the click and do nothing.
    let dismiss = Dismiss::new(card).prevent_interaction_with_other_elements();
    let inner = match dismiss_on_click {
        Some(action) => dismiss
            .on_dismiss(move |ctx, _app| ctx.dispatch_typed_action(action.clone()))
            .finish(),
        None => dismiss.on_dismiss(|_ctx, _app| {}).finish(),
    };

    // Stack so the card can read its bounds (keeps it clear of the window's
    // traffic lights), then center it and lay the scrim behind everything.
    let mut stack = Stack::new();
    stack.add_positioned_child(
        inner,
        OffsetPositioning::offset_from_parent(
            vec2f(0., 0.),
            ParentOffsetBounds::WindowByPosition,
            ParentAnchor::Center,
            ChildAnchor::Center,
        ),
    );

    Container::new(Align::new(stack.finish()).finish())
        .with_background_color(modal_scrim())
        .with_corner_radius(app.windows().window_corner_radius())
        .finish()
}

#[cfg(test)]
mod tests {
    use super::unsaved_input_dismiss_action;

    #[test]
    fn cockpit_and_ssh_dialogs_use_shared_state_contract() {
        assert!(unsaved_input_dismiss_action::<u8>().is_none());
    }
}
