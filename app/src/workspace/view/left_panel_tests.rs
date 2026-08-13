use super::{secondary_return_target, ToolPanelView};

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
fn primary_panel_never_exposes_secondary_back_target() {
    assert_eq!(
        secondary_return_target(
            ToolPanelView::Cockpit,
            &[ToolPanelView::Cockpit, ToolPanelView::ProjectExplorer],
        ),
        None
    );
}
