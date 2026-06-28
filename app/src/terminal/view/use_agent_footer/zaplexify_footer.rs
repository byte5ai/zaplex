use std::sync::Arc;

use parking_lot::FairMutex;
use warpui::prelude::Empty;
use warpui::{
    elements::{
        ChildView, Container, CrossAxisAlignment, Expanded, Flex, MainAxisSize, ParentElement,
    },
    AppContext, Element, Entity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    terminal::view::{TerminalModel, PADDING_LEFT},
    ui_components::icons::Icon,
    view_components::action_button::{ActionButton, ButtonSize, KeystrokeSource, TooltipAlignment},
};

use super::{AgentFooterButtonTheme, USE_AGENT_KEYSTROKE};
use crate::terminal::view::block_banner::ZaplexificationMode;

/// Footer view rendered for detected subshell/SSH commands, offering both
/// "Zaplexify" and "Use agent" buttons in a horizontal row.
pub(super) struct ZaplexifyFooterView {
    terminal_model: Arc<FairMutex<TerminalModel>>,
    zaplexify_button: ViewHandle<ActionButton>,
    use_agent_button: ViewHandle<ActionButton>,
    dismiss_button: ViewHandle<ActionButton>,
    mode: Option<ZaplexificationMode>,
}

impl ZaplexifyFooterView {
    pub fn new(terminal_model: Arc<FairMutex<TerminalModel>>, ctx: &mut ViewContext<Self>) -> Self {
        let button_size = ButtonSize::XSmall;

        let zaplexify_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new(
                crate::t!("terminal-zaplexify-subshell"),
                AgentFooterButtonTheme::new(None),
            )
            .with_icon(Icon::Zap)
            .with_size(button_size)
            .with_tooltip(crate::t!("terminal-zaplexify-subshell-tooltip"))
            .with_tooltip_alignment(TooltipAlignment::Left)
            .on_click(|ctx| {
                ctx.dispatch_typed_action(ZaplexifyFooterViewAction::Zaplexify);
            })
        });

        let use_agent_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(
                crate::t!("terminal-use-agent"),
                AgentFooterButtonTheme::new(None),
            )
            .with_icon(Icon::Oz)
            .with_keybinding(KeystrokeSource::Fixed(USE_AGENT_KEYSTROKE.clone()), ctx)
            .with_size(button_size)
            .with_tooltip(crate::t!("terminal-use-agent-tooltip"))
            .with_tooltip_alignment(TooltipAlignment::Left)
            .on_click(|ctx| {
                ctx.dispatch_typed_action(ZaplexifyFooterViewAction::UseAgent);
            })
        });

        let dismiss_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new(
                crate::t!("common-dismiss"),
                AgentFooterButtonTheme::new(None),
            )
            .with_size(button_size)
            .on_click(|ctx| {
                ctx.dispatch_typed_action(ZaplexifyFooterViewAction::Dismiss);
            })
        });

        Self {
            terminal_model,
            zaplexify_button,
            use_agent_button,
            dismiss_button,
            mode: None,
        }
    }

    /// Updates the zaplexify button label, keybinding, and stores the current zaplexification mode.
    pub fn set_mode(&mut self, mode: ZaplexificationMode, ctx: &mut ViewContext<Self>) {
        let (label, binding_name) = match mode {
            ZaplexificationMode::Ssh { .. } => {
                ("Zaplexify SSH session", "terminal:zaplexify_ssh_session")
            }
            ZaplexificationMode::Subshell { .. } => ("Zaplexify subshell", "terminal:zaplexify_subshell"),
        };
        self.zaplexify_button.update(ctx, |button, ctx| {
            button.set_label(label, ctx);
            button.set_keybinding(Some(KeystrokeSource::Binding(binding_name)), ctx);
        });
        self.mode = Some(mode);
        ctx.notify();
    }

    /// Returns the current zaplexification mode, if set.
    pub fn mode(&self) -> Option<&ZaplexificationMode> {
        self.mode.as_ref()
    }

    /// Clears the zaplexification mode.
    pub fn clear_mode(&mut self, ctx: &mut ViewContext<Self>) {
        self.mode = None;
        self.zaplexify_button.update(ctx, |button, ctx| {
            button.set_keybinding(None, ctx);
        });
        ctx.notify();
    }
}

#[derive(Debug, Clone)]
pub enum ZaplexifyFooterViewAction {
    Zaplexify,
    UseAgent,
    Dismiss,
}

pub enum ZaplexifyFooterViewEvent {
    Zaplexify { mode: ZaplexificationMode },
    UseAgent,
    Dismiss,
}

impl Entity for ZaplexifyFooterView {
    type Event = ZaplexifyFooterViewEvent;
}

impl View for ZaplexifyFooterView {
    fn ui_name() -> &'static str {
        "ZaplexifyFooterView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        let terminal_model = self.terminal_model.lock();

        let button_row = Flex::row()
            .with_spacing(4.)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(ChildView::new(&self.zaplexify_button).finish())
            .with_child(ChildView::new(&self.use_agent_button).finish())
            .with_child(Expanded::new(1., Empty::new().finish()).finish())
            .with_child(ChildView::new(&self.dismiss_button).finish());

        let mut container = Container::new(button_row.finish())
            .with_horizontal_padding(*PADDING_LEFT)
            .with_vertical_padding(4.);

        if terminal_model.is_alt_screen_active() {
            if let Some(bg_color) = terminal_model.alt_screen().inferred_bg_color() {
                container = container.with_background(bg_color);
            }
        }

        container.finish()
    }
}

impl TypedActionView for ZaplexifyFooterView {
    type Action = ZaplexifyFooterViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ZaplexifyFooterViewAction::Zaplexify => {
                if let Some(mode) = self.mode.clone() {
                    self.clear_mode(ctx);
                    ctx.emit(ZaplexifyFooterViewEvent::Zaplexify { mode });
                }
            }
            ZaplexifyFooterViewAction::UseAgent => {
                self.clear_mode(ctx);
                ctx.emit(ZaplexifyFooterViewEvent::UseAgent);
            }
            ZaplexifyFooterViewAction::Dismiss => {
                self.clear_mode(ctx);
                ctx.emit(ZaplexifyFooterViewEvent::Dismiss);
            }
        }
    }
}
