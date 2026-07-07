//! Guardrails confirmation dialog (cockpit step 7): the single confirm surface
//! shared by the two destructive guardrail verbs — per-agent "⨯ kill"
//! (SIGKILL) and the fleet-wide "Stop all" (SIGINT to every live agent).
//! Modeled on [`super::close_session_confirmation_dialog::CloseSessionConfirmationDialog`]
//! (same shell, dialog component, blur backdrop); [`AgentGuardrailKind`] carries
//! whatever the confirm handler needs to dispatch the actual signal, and the
//! title/body text is built by the pure `zaplex_cockpit::guardrails` message
//! functions so it stays unit-testable outside GPUI.

use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::{
    elements::{
        Align, ChildAnchor, Container, MouseStateHandle, OffsetPositioning, ParentAnchor,
        ParentOffsetBounds, Stack,
    },
    fonts::Weight,
    platform::Cursor,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::{
    appearance::Appearance,
    ui_components::dialog::{dialog_styles, Dialog},
};

/// What the user is confirming — everything the confirm handler needs to
/// dispatch the actual signal back to the workspace. Cheap to clone (small,
/// string-based identity), so the dialog can hand a copy back on confirm
/// without borrowing itself.
#[derive(Clone, Debug)]
pub enum AgentGuardrailKind {
    /// Per-row "⨯ kill" verb (guardrails step 7): SIGKILL a single agent.
    KillAgent {
        host: String,
        session_id: String,
        pid: u32,
        /// Row label (name — dir), for the dialog's message.
        agent_label: String,
        project_name: String,
    },
    /// Fleet-wide "Stop all" control: SIGINT every live agent across the
    /// inventory. `count` is only used for the confirm message; the workspace
    /// re-resolves the live target list at confirm time (never stale).
    StopAll { count: usize },
}

pub struct AgentGuardrailDialog {
    cancel_mouse_state: MouseStateHandle,
    confirm_mouse_state: MouseStateHandle,
    /// `None` if the dialog was never opened.
    kind: Option<AgentGuardrailKind>,
}

impl Default for AgentGuardrailDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentGuardrailDialog {
    pub fn new() -> Self {
        Self {
            cancel_mouse_state: Default::default(),
            confirm_mouse_state: Default::default(),
            kind: None,
        }
    }

    pub fn set_kind(&mut self, kind: AgentGuardrailKind) {
        self.kind = Some(kind);
    }
}

impl Entity for AgentGuardrailDialog {
    type Event = AgentGuardrailDialogEvent;
}

impl View for AgentGuardrailDialog {
    fn ui_name() -> &'static str {
        "AgentGuardrailDialog"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let (title, body) = match &self.kind {
            Some(AgentGuardrailKind::KillAgent {
                agent_label,
                project_name,
                ..
            }) => zaplex_cockpit::kill_confirm_message(agent_label, project_name),
            Some(AgentGuardrailKind::StopAll { count }) => {
                zaplex_cockpit::stop_all_confirm_message(*count)
            }
            // Never rendered while visible (workspace always sets `kind` before
            // opening), but stays inert rather than panicking if it ever is.
            None => (String::new(), String::new()),
        };
        let confirm_label = match &self.kind {
            Some(AgentGuardrailKind::KillAgent { .. }) => "Kill",
            _ => "Stop all",
        };

        let button_style = UiComponentStyles {
            font_size: Some(14.),
            font_weight: Some(Weight::Bold),
            width: Some(160.),
            height: Some(40.),
            ..Default::default()
        };

        let confirm_button = appearance
            .ui_builder()
            .button(ButtonVariant::Error, self.confirm_mouse_state.clone())
            .with_centered_text_label(confirm_label.to_string())
            .with_style(button_style.clone())
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(AgentGuardrailDialogAction::Confirm)
            })
            .finish();

        let cancel_button = appearance
            .ui_builder()
            .button(ButtonVariant::Basic, self.cancel_mouse_state.clone())
            .with_centered_text_label("Cancel".to_string())
            .with_style(button_style)
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(AgentGuardrailDialogAction::Cancel)
            })
            .finish();

        let dialog = Container::new(
            Dialog::new(
                title,
                Some(body),
                UiComponentStyles {
                    width: Some(460.),
                    padding: Some(Coords::uniform(24.)),
                    ..dialog_styles(appearance)
                },
            )
            .with_bottom_row_child(cancel_button)
            .with_bottom_row_child(confirm_button)
            .build()
            .finish(),
        )
        .with_margin_top(35.)
        .finish();

        // Stack needed so that dialog can get bounds information, specifically
        // to ensure no overlap with the window's traffic lights.
        let mut stack = Stack::new();
        stack.add_positioned_child(
            dialog,
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );

        // This blurs the background and makes it uninteractable.
        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(Fill::blur().into())
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }
}

pub enum AgentGuardrailDialogEvent {
    Confirm { kind: AgentGuardrailKind },
    Cancel,
}

#[derive(Debug)]
pub enum AgentGuardrailDialogAction {
    Confirm,
    Cancel,
}

impl TypedActionView for AgentGuardrailDialog {
    type Action = AgentGuardrailDialogAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentGuardrailDialogAction::Confirm => {
                let Some(kind) = self.kind.clone() else {
                    log::error!("Guardrail dialog confirm pressed with no kind set");
                    return;
                };
                ctx.emit(AgentGuardrailDialogEvent::Confirm { kind });
            }
            AgentGuardrailDialogAction::Cancel => {
                ctx.emit(AgentGuardrailDialogEvent::Cancel);
            }
        }
    }
}
