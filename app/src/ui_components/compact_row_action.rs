use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::elements::{ChildView, Element};
use warpui::{TypedActionView, View, ViewContext, ViewHandle};

use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize, PaneHeaderTheme};

/// An icon-only secondary action for a compact, repeated row.
///
/// The constructor deliberately has no visible-label parameter. The row's
/// primary identity owns the flexible width; secondary actions occupy one
/// fixed square each and expose their text through a tooltip.
#[derive(Clone)]
pub struct CompactRowAction {
    button: ViewHandle<ActionButton>,
}

impl CompactRowAction {
    pub fn new<T>(
        icon: Icon,
        tooltip: impl Into<String>,
        action: T::Action,
        ctx: &mut ViewContext<T>,
    ) -> Self
    where
        T: TypedActionView + View,
        T::Action: Clone,
    {
        Self::new_with_icon_ansi_color(icon, None, tooltip, action, ctx)
    }

    /// Creates a compact action whose icon uses one existing semantic ANSI
    /// color. The button theme and all hover/focus behavior remain shared.
    pub fn new_with_icon_ansi_color<T>(
        icon: Icon,
        icon_color: Option<AnsiColorIdentifier>,
        tooltip: impl Into<String>,
        action: T::Action,
        ctx: &mut ViewContext<T>,
    ) -> Self
    where
        T: TypedActionView + View,
        T::Action: Clone,
    {
        let tooltip = tooltip.into();
        let button = ctx.add_typed_action_view(move |_| {
            let mut button = ActionButton::new("", PaneHeaderTheme)
                .with_size(ButtonSize::XSmall)
                .with_icon(icon)
                .with_tooltip(tooltip)
                .on_click(move |ctx| ctx.dispatch_typed_action(action.clone()));
            if let Some(icon_color) = icon_color {
                button = button.with_icon_ansi_color(icon_color);
            }
            button
        });
        Self { button }
    }

    pub fn render(&self) -> Box<dyn Element> {
        ChildView::new(&self.button).finish()
    }
}
