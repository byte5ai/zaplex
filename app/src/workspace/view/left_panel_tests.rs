use super::{
    secondary_return_target, sessions_entry_shows_attention, view_remains_available, ToolPanelView,
};

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

#[test]
fn waiting_signal_visible_on_sessions_entry_while_accounts_active() {
    // #504: with the accounts view open, a waiting agent must stay visible on
    // the Sessions entry; inside the tree its own header carries the count.
    assert!(sessions_entry_shows_attention(
        ToolPanelView::CockpitAccounts,
        1
    ));
    assert!(sessions_entry_shows_attention(ToolPanelView::SshManager, 2));
    assert!(!sessions_entry_shows_attention(ToolPanelView::Cockpit, 3));
    assert!(!sessions_entry_shows_attention(
        ToolPanelView::CockpitAccounts,
        0
    ));
}

#[test]
fn accounts_view_is_a_primary_view_without_back_target() {
    assert_eq!(
        secondary_return_target(
            ToolPanelView::CockpitAccounts,
            &[
                ToolPanelView::Cockpit,
                ToolPanelView::CockpitAccounts,
                ToolPanelView::SshManager,
            ],
        ),
        None
    );
    assert!(view_remains_available(
        ToolPanelView::CockpitAccounts,
        &[ToolPanelView::Cockpit, ToolPanelView::CockpitAccounts],
    ));
}
