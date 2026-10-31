use super::{initialize_app, mock_workspace, WorkspaceAction};
use crate::tab_configs::new_worktree_modal::NewWorktreeModalAction;
use warpui::{App, TypedActionView};

#[test]
fn worktree_modal_cancel_restores_original_tab_focus() {
    for action in [
        NewWorktreeModalAction::Cancel,
        NewWorktreeModalAction::Escape,
    ] {
        App::test((), |mut app| async move {
            crate::i18n::init(Some("en"));
            initialize_app(&mut app);
            let workspace = mock_workspace(&mut app);
            let original_group = workspace.read(&app, |workspace, _| {
                workspace.active_tab_pane_group().clone()
            });
            let original_tab_count = workspace.read(&app, |workspace, _| workspace.tabs.len());
            let body = workspace.read(&app, |workspace, app| {
                workspace.new_worktree_modal.view.as_ref(app).body().clone()
            });

            workspace.update(&mut app, |workspace, ctx| {
                workspace.focus_active_tab(ctx);
            });
            workspace.update(&mut app, |workspace, ctx| {
                assert!(original_group.is_self_or_child_focused(ctx));
                workspace.handle_action(&WorkspaceAction::OpenNewWorktreeModal, ctx);
                // Cancel from a focused modal, as after editing its fields.
                ctx.focus(&body);
            });
            workspace.update(&mut app, |workspace, ctx| {
                assert!(workspace.current_workspace_state.is_new_worktree_modal_open);
                assert!(body.is_self_or_child_focused(ctx));
                assert!(!original_group.is_self_or_child_focused(ctx));
            });
            body.update(&mut app, |body, ctx| body.handle_action(&action, ctx));

            workspace.update(&mut app, |workspace, ctx| {
                assert!(!workspace.current_workspace_state.is_new_worktree_modal_open);
                assert_eq!(workspace.active_tab_pane_group().id(), original_group.id());
                assert_eq!(workspace.tabs.len(), original_tab_count);
                assert!(
                    original_group.is_self_or_child_focused(ctx),
                    "Expected the original tab to regain keyboard focus after {action:?}"
                );
            });
        });
    }
}
