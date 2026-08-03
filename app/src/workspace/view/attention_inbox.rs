//! The **"Offene Punkte" inbox** — the calm, in-app counterpart to the ambient
//! Dock badge. Where the badge answers *"is there anything for me?"* with a
//! single number, this modal answers *"what, exactly?"*: a prioritized,
//! human-friendly to-do list of every agent across the whole fleet that is
//! **waiting on you** (`SessionState::Waiting`), waiting-first, grouped by host
//! and project.
//!
//! It is deliberately an *inbox you clear without dread*, not an alarm: neutral
//! copy, no red, the same calm `✋` glyph the cockpit's Conductor uses.
//! Selecting a row **jumps to that agent** through the cockpit's shared,
//! state-aware [`WorkspaceAction::AttachFleetSession`] route. A known live pane
//! is focused, a dormant session is resumed, and an unlocated live session is
//! reported honestly instead of being duplicated. The same rule applies to
//! local and remote rows.
//!
//! Surfaced from the workspace via `WorkspaceAction::OpenAttentionInbox`
//! (default binding `cmd/ctrl-shift-o`); closed with `escape` or its close
//! button.

use std::collections::HashMap;

use pathfinder_color::ColorU;
use warp_core::ui::theme::{Fill, phenomenon::PhenomenonStyle};
use warpui::elements::{
    ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Fill as ElementFill, Flex, Hoverable, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, ScrollbarWidth, Shrinkable, Text,
};
use warpui::fonts::FamilyId;
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::{
    AppContext, Entity, SingletonEntity as _, TypedActionView, View, ViewContext, ViewHandle,
};
use zaplex_cockpit::{SessionState, session_key};

use crate::WorkspaceAction;
use crate::appearance::Appearance;
use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::style::{attention_coloru, glyph_cell};
use crate::ui_components::modal_frame;
use crate::view_components::action_button::ActionButton;

const MODAL_MAX_LIST_HEIGHT: f32 = 420.;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        AttentionInboxAction::Close,
        id!(AttentionInbox::ui_name()),
    )]);
}

#[derive(Clone, Debug)]
pub enum AttentionInboxAction {
    Close,
}

#[derive(Clone, Debug)]
pub enum AttentionInboxEvent {
    Close,
}

pub struct AttentionInbox {
    close_button: ViewHandle<ActionButton>,
    scroll_state: ClippedScrollStateHandle,
    /// Stable hover handle per waiting row, keyed by complete [`session_key`]
    /// identity so copied ids on one host remain separate. Synced against the
    /// cockpit inventory so handles persist across renders.
    row_states: HashMap<String, MouseStateHandle>,
}

impl AttentionInbox {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&Appearance::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&CockpitModel::handle(ctx), |me, _, event, ctx| {
            if matches!(event, CockpitEvent::Updated) {
                me.sync_rows(ctx);
                ctx.notify();
            }
        });

        let close_button =
            ctx.add_view(|_ctx| modal_frame::close_button(AttentionInboxAction::Close));

        let mut me = Self {
            close_button,
            scroll_state: ClippedScrollStateHandle::default(),
            row_states: HashMap::new(),
        };
        me.sync_rows(ctx);
        me
    }

    /// Keep one stable hover handle per currently-waiting row. Dropping stale
    /// handles keeps the map bounded as the fleet churns.
    fn sync_rows(&mut self, ctx: &mut ViewContext<Self>) {
        let model = CockpitModel::as_ref(ctx);

        // Hover handles for every waiting row across the whole fleet.
        let waiting_keys: Vec<String> = model
            .inventory()
            .hosts
            .iter()
            .flat_map(|h| {
                h.projects.iter().flat_map(move |p| {
                    p.sessions
                        .iter()
                        .filter(|s| s.state == SessionState::Waiting)
                        .map(move |s| session_key(h.is_local, h.host_id.as_deref(), s))
                })
            })
            .collect();
        let live: std::collections::HashSet<&String> = waiting_keys.iter().collect();
        self.row_states.retain(|k, _| live.contains(k));
        for key in waiting_keys {
            self.row_states.entry(key).or_default();
        }
    }

    fn render_header(&self, waiting: usize, appearance: &Appearance) -> Box<dyn Element> {
        // The one shared modal header (title · count · close ✕) — identical
        // grammar to the Spawn-Karte and every migrated dialog.
        modal_frame::modal_header(
            crate::t!("cockpit-attention-inbox-title"),
            Some(crate::t!(
                "cockpit-attention-inbox-count",
                count = (waiting as i64)
            )),
            warpui::elements::ChildView::new(&self.close_button).finish(),
            appearance,
        )
    }

    /// One waiting-agent row. Every row uses the shared state-aware attach route:
    /// it focuses a known pane, resumes only a dormant session, and never starts
    /// a second copy of a live agent.
    fn render_row(
        &self,
        host: &str,
        host_id: Option<&str>,
        is_local: bool,
        project: &str,
        session: &zaplex_cockpit::SessionSnapshot,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let main = theme.main_text_color(theme.background()).into_solid();
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let accent = theme.accent().into_solid();

        // Display copies owned by the (repeatedly-called) content builder. The
        // model shows in the Conductor's compact vocabulary ("Opus·High"), not
        // as a raw model id — one attribute language on every surface.
        let disp_project = project.to_string();
        let disp_host = host.to_string();
        let disp_model =
            zaplex_cockpit::model_effort_label(&session.model, session.effort.as_deref());

        let key = session_key(is_local, host_id, session);

        let build_content = move |hovered: bool| -> Box<dyn Element> {
            let name_color = if hovered { accent } else { main };
            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(8.0)
                // The one attention amber every ✋ shares — calm, never
                // alarm-red — in the shared fixed-width glyph column.
                .with_child(glyph_cell(
                    zaplex_cockpit::GLYPH_WAITING,
                    attention_coloru(appearance),
                    appearance,
                ))
                .with_child(
                    Shrinkable::new(
                        1.0,
                        text_inline(disp_project.clone(), family, body, name_color),
                    )
                    .finish(),
                )
                .with_child(text_inline(disp_host.clone(), family, body, muted));
            if !disp_model.is_empty() {
                row = row.with_child(text_inline(disp_model.clone(), family, body, muted));
            }
            row.with_main_axis_size(MainAxisSize::Max).finish()
        };

        let plain_row = |content: Box<dyn Element>| -> Box<dyn Element> {
            Container::new(content)
                .with_vertical_padding(6.)
                .with_horizontal_padding(8.)
                .finish()
        };

        let Some(state) = self.row_states.get(&key).cloned() else {
            return plain_row(build_content(false));
        };

        let action = WorkspaceAction::AttachFleetSession {
            host: host.to_string(),
            host_id: host_id.map(str::to_string),
            session_id: session.session_id.clone(),
            provider: session.provider,
            config_dir: session.config_dir.clone(),
            account_email: session.account_email.clone(),
            is_local,
        };
        Hoverable::new(state, move |mouse| {
            let hovered = mouse.is_hovered();
            let mut container = Container::new(build_content(hovered))
                .with_vertical_padding(6.)
                .with_horizontal_padding(8.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));
            if hovered {
                // Subtle hover fill — calm, reusing the modal chrome tone.
                container = container
                    .with_background(Fill::Solid(PhenomenonStyle::modal_close_button_hover()));
            }
            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        // Reuses the cockpit's state-aware "open = focus" verb; the workspace
        // handler also closes this inbox.
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
    }

    fn render_body(&self, app: &AppContext, appearance: &Appearance) -> Box<dyn Element> {
        let family = appearance.ui_font_family();
        let body = appearance.ui_font_body();
        let theme = appearance.theme();
        let muted = theme.sub_text_color(theme.background()).into_solid();

        let model = CockpitModel::as_ref(app);
        let waiting = model.needs_me();

        let mut list = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(2.);

        if waiting == 0 {
            // Calm "cleared" state — the tone of an empty inbox, not an alert.
            list = list.with_child(
                Container::new(text_inline(
                    crate::t!("cockpit-attention-inbox-empty"),
                    family,
                    body,
                    muted,
                ))
                .with_vertical_padding(8.)
                .with_horizontal_padding(8.)
                .finish(),
            );
        } else {
            // Inventory is already ordered waiting-first, needs-me-heavy hosts
            // and projects on top — just render its waiting leaves in order.
            for host in &model.inventory().hosts {
                for project in &host.projects {
                    for session in &project.sessions {
                        if session.state != SessionState::Waiting {
                            continue;
                        }
                        list = list.with_child(self.render_row(
                            &host.host,
                            host.host_id.as_deref(),
                            host.is_local,
                            &project.name,
                            session,
                            appearance,
                        ));
                    }
                }
            }
        }

        let scroll = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            list.finish(),
            ScrollbarWidth::Auto,
            theme.disabled_text_color(theme.background()).into(),
            theme.main_text_color(theme.background()).into(),
            ElementFill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        ConstrainedBox::new(scroll)
            .with_max_height(MODAL_MAX_LIST_HEIGHT)
            .finish()
    }
}

fn text_inline(s: String, family: FamilyId, size: f32, color: ColorU) -> Box<dyn Element> {
    Text::new_inline(s, family, size).with_color(color).finish()
}

impl Entity for AttentionInbox {
    type Event = AttentionInboxEvent;
}

impl View for AttentionInbox {
    fn ui_name() -> &'static str {
        "AttentionInbox"
    }

    fn on_focus(&mut self, _focus_ctx: &warpui::FocusContext, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let waiting = CockpitModel::as_ref(app).needs_me();

        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.render_header(waiting, appearance))
            .with_child(
                Container::new(self.render_body(app, appearance))
                    .with_margin_top(12.)
                    .finish(),
            )
            .finish();

        // The one shared modal card + scrim. The inbox is read-only (a list you
        // clear), so a click on the scrim dismisses it — the unified backdrop
        // policy for modals that hold no unsaved input.
        let card = modal_frame::modal_card(content, modal_frame::MODAL_WIDTH_WIDE, appearance);
        modal_frame::modal_overlay(card, Some(AttentionInboxAction::Close), app)
    }
}

impl TypedActionView for AttentionInbox {
    type Action = AttentionInboxAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AttentionInboxAction::Close => {
                ctx.emit(AttentionInboxEvent::Close);
            }
        }
    }
}
