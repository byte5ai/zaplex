//! SFTP file browser pane (a central pane, opened via the SSH manager tree).
//!
//! Mirrors the minimal structure of `ssh_server_pane.rs`. The pane is not persisted (
//! `LeafContents::Sftp { .. }` returns false from `is_persisted()`);
//! its data flows through SFTP connection operations.
//! author: logic
//! date: 2026-05-26

use warpui::{AppContext, ModelHandle, View, ViewContext, ViewHandle};

use crate::app_state::LeafContents;
use crate::pane_group::{BackingView, PaneConfiguration, PaneContent, PaneGroup, PaneView};
use crate::sftp_manager::browser::SftpBrowserView;

use super::{DetachType, PaneId, ShareableLink, ShareableLinkError};

/// SFTP file browser pane contents
pub struct SftpPane {
    view: ViewHandle<PaneView<SftpBrowserView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    /// Business node id (not the pane view id), used for snapshot serialization.
    node_id: String,
}

impl SftpPane {
    #[cfg(test)]
    pub(crate) fn browser_view(&self, ctx: &warpui::AppContext) -> ViewHandle<SftpBrowserView> {
        self.view.as_ref(ctx).child(ctx)
    }

    /// Creates a new SFTP browser pane, rooted at `start_path` (the remote
    /// shell's cwd) when known, else the host root `/`.
    pub fn new<V: View>(
        node_id: String,
        start_path: Option<std::path::PathBuf>,
        ctx: &mut ViewContext<V>,
    ) -> Self {
        let id_for_view = node_id.clone();
        let browser_view = ctx.add_typed_action_view(move |ctx| {
            SftpBrowserView::new(id_for_view.clone(), start_path, ctx)
        });
        let pane_configuration = browser_view.as_ref(ctx).pane_configuration();
        let pane_view = ctx.add_typed_action_view(|ctx| {
            let pane_id = PaneId::from_sftp_pane_ctx(ctx);
            PaneView::new(pane_id, browser_view, (), pane_configuration.clone(), ctx)
        });
        Self {
            view: pane_view,
            pane_configuration,
            node_id,
        }
    }

    /// Creates an SFTP browser pane in **pick mode** (the spawn card's "Browse…",
    /// #105): the browser shows a "Use this folder" pick bar that returns the
    /// chosen directory to the spawn card via `WorkspaceAction::RemoteSpawnDirPicked`.
    pub fn new_for_pick<V: View>(
        node_id: String,
        start_path: Option<std::path::PathBuf>,
        ctx: &mut ViewContext<V>,
    ) -> Self {
        let id_for_view = node_id.clone();
        let browser_view = ctx.add_typed_action_view(move |ctx| {
            SftpBrowserView::new(id_for_view.clone(), start_path, ctx).with_pick_mode()
        });
        let pane_configuration = browser_view.as_ref(ctx).pane_configuration();
        let pane_view = ctx.add_typed_action_view(|ctx| {
            let pane_id = PaneId::from_sftp_pane_ctx(ctx);
            PaneView::new(pane_id, browser_view, (), pane_configuration.clone(), ctx)
        });
        Self {
            view: pane_view,
            pane_configuration,
            node_id,
        }
    }

    /// Creates a file-manager pane over the **local** filesystem (FM pane-mode
    /// P1), rooted at `start_path`. Snapshots as `LeafContents::Sftp` with an
    /// empty node id — irrelevant in practice since SFTP/FM panes are not
    /// persisted (`is_persisted()` is false).
    pub fn new_local<V: View>(start_path: std::path::PathBuf, ctx: &mut ViewContext<V>) -> Self {
        let browser_view =
            ctx.add_typed_action_view(move |ctx| SftpBrowserView::new_local(start_path, ctx));
        let pane_configuration = browser_view.as_ref(ctx).pane_configuration();
        let pane_view = ctx.add_typed_action_view(|ctx| {
            let pane_id = PaneId::from_sftp_pane_ctx(ctx);
            PaneView::new(pane_id, browser_view, (), pane_configuration.clone(), ctx)
        });
        Self {
            view: pane_view,
            pane_configuration,
            node_id: String::new(),
        }
    }
}

impl PaneContent for SftpPane {
    fn id(&self) -> PaneId {
        PaneId::from_sftp_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));
        let child = self.view.as_ref(ctx).child(ctx);

        let pane_id = self.id();
        ctx.subscribe_to_view(&child, move |pane_group, _, event, ctx| {
            pane_group.handle_pane_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let child = self.view.as_ref(ctx).child(ctx);
        ctx.unsubscribe_to_view(&child);
    }

    fn snapshot(&self, _ctx: &AppContext) -> LeafContents {
        LeafContents::Sftp {
            node_id: self.node_id.clone(),
        }
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
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
