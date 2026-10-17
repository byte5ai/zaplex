use super::*;

#[test]
fn persisted_cockpit_selection_maps_to_sessions_view() {
    // Before the accounts became their own sidebar view (#504) the whole
    // Cockpit persisted as "Cockpit". Such a snapshot must keep restoring the
    // sessions tree, and the sessions tree must keep that wire name, while
    // the new accounts view persists under its own name.
    let stored =
        r#"{"left_panel_displayed_tab":"Cockpit","pane_group_id":"side-panel","width":287}"#;
    let snapshot = serde_json::from_str::<LeftPanelSnapshot>(stored)
        .expect("a Cockpit snapshot from before #504 must remain readable");
    assert_eq!(
        snapshot.left_panel_displayed_tab,
        LeftPanelDisplayedTab::Cockpit
    );
    assert_eq!(
        LeftPanelDisplayedTab::from(ToolPanelView::Cockpit),
        LeftPanelDisplayedTab::Cockpit,
        "the sessions tree persists under the pre-#504 name"
    );

    let accounts = LeftPanelSnapshot {
        left_panel_displayed_tab: ToolPanelView::CockpitAccounts.into(),
        pane_group_id: "side-panel".to_string(),
        width: 287,
    };
    assert_eq!(
        serde_json::to_value(&accounts).unwrap()["left_panel_displayed_tab"],
        serde_json::json!("CockpitAccounts")
    );
}
