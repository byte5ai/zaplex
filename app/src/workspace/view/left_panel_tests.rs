use super::{
    secondary_return_target, sessions_entry_shows_attention, view_remains_available, ToolPanelView,
};

#[test]
fn waiting_signal_visible_on_sessions_entry_while_accounts_active() {
    // #504: with the accounts view open, a waiting agent must stay visible on
    // the Sessions entry; inside the tree its own header carries the count.
    assert!(sessions_entry_shows_attention(
        ToolPanelView::CockpitAccounts,
        true
    ));
    assert!(sessions_entry_shows_attention(
        ToolPanelView::SshManager,
        true
    ));
    assert!(!sessions_entry_shows_attention(
        ToolPanelView::Cockpit,
        true
    ));
    assert!(!sessions_entry_shows_attention(
        ToolPanelView::CockpitAccounts,
        false
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
