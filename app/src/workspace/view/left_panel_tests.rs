use super::{secondary_return_target, view_remains_available, ToolPanelView};

#[test]
fn ssh_manager_drill_in_returns_to_cockpit() {
    assert_eq!(
        secondary_return_target(
            ToolPanelView::SshManager,
            &[ToolPanelView::Cockpit, ToolPanelView::ProjectExplorer],
        ),
        Some(ToolPanelView::Cockpit)
    );
}

#[test]
fn primary_ssh_manager_has_no_back_target() {
    assert_eq!(
        secondary_return_target(ToolPanelView::SshManager, &[ToolPanelView::SshManager]),
        None
    );
}

#[test]
fn toolbelt_ssh_manager_has_no_back_target_even_with_cockpit_available() {
    assert_eq!(
        secondary_return_target(
            ToolPanelView::SshManager,
            &[ToolPanelView::Cockpit, ToolPanelView::SshManager],
        ),
        None
    );
}

#[test]
fn ssh_manager_drill_in_survives_available_view_updates() {
    assert!(view_remains_available(
        ToolPanelView::SshManager,
        &[ToolPanelView::Cockpit, ToolPanelView::ProjectExplorer],
    ));
}

#[test]
fn removed_primary_view_does_not_remain_available() {
    assert!(!view_remains_available(
        ToolPanelView::ProjectExplorer,
        &[ToolPanelView::Cockpit],
    ));
}

#[test]
fn primary_panel_never_exposes_secondary_back_target() {
    assert_eq!(
        secondary_return_target(
            ToolPanelView::Cockpit,
            &[ToolPanelView::Cockpit, ToolPanelView::ProjectExplorer],
        ),
        None
    );
}
