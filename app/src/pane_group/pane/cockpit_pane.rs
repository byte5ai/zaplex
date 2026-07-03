use warpui::{AppContext, ModelHandle, View, ViewContext, ViewHandle};

use crate::{
    app_state::LeafContents,
    cockpit::CockpitPaneView,
    pane_group::{
        pane::{ShareableLink, ShareableLinkError},
        BackingView, PaneConfiguration, PaneContent, PaneGroup, PaneView,
    },
};

use super::PaneId;

/// The cockpit dashboard as pane content (tab/split/promotable, multi-instance).
/// The dashboard view itself lives in `crate::cockpit::pane`; this wrapper
/// hosts it in the pane system (mirrors `WelcomePane`).
pub struct CockpitPane {
    view: ViewHandle<PaneView<CockpitPaneView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl CockpitPane {
    pub fn new<V: View>(ctx: &mut ViewContext<V>) -> Self {
        let cockpit_view = ctx.add_typed_action_view(CockpitPaneView::new);
        let pane_configuration = cockpit_view.as_ref(ctx).pane_configuration();
        let pane_view = ctx.add_typed_action_view(|ctx| {
            let pane_id = PaneId::from_cockpit_pane_ctx(ctx);
            PaneView::new(pane_id, cockpit_view, (), pane_configuration.clone(), ctx)
        });
        Self {
            view: pane_view,
            pane_configuration,
        }
    }
}

impl PaneContent for CockpitPane {
    fn id(&self) -> PaneId {
        PaneId::from_cockpit_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        let pane_id = self.id();
        let child = self.view.as_ref(ctx).child(ctx);
        ctx.subscribe_to_view(&child, move |pane_group, _, event, ctx| {
            pane_group.handle_pane_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: super::DetachType,
        ctx: &mut warpui::ViewContext<PaneGroup>,
    ) {
        let child = self.view.as_ref(ctx).child(ctx);
        ctx.unsubscribe_to_view(&child);
    }

    fn snapshot(&self, _ctx: &AppContext) -> LeafContents {
        LeafContents::Cockpit
    }

    fn has_application_focus(&self, ctx: &mut warpui::ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut warpui::ViewContext<PaneGroup>) {
        self.view
            .as_ref(ctx)
            .child(ctx)
            .update(ctx, BackingView::focus_contents)
    }

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        Ok(ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}
