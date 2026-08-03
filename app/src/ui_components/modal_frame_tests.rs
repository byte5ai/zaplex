use super::unsaved_input_dismiss_action;

#[test]
fn cockpit_and_ssh_dialogs_use_shared_state_contract() {
    assert!(unsaved_input_dismiss_action::<u8>().is_none());
}
