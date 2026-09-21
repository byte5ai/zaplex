use super::{remote_terminal_state_survives_detach, DetachType};

#[test]
fn remote_terminal_identity_survives_only_reversible_detaches() {
    assert!(remote_terminal_state_survives_detach(DetachType::Moved));
    assert!(remote_terminal_state_survives_detach(
        DetachType::HiddenForClose
    ));
    assert!(!remote_terminal_state_survives_detach(DetachType::Closed));
}

#[test]
fn temporary_file_manager_sidecar_uses_the_same_permanent_close_boundary() {
    assert!(remote_terminal_state_survives_detach(DetachType::Moved));
    assert!(remote_terminal_state_survives_detach(
        DetachType::HiddenForClose
    ));
    assert!(!remote_terminal_state_survives_detach(DetachType::Closed));

    let source = include_str!("terminal_pane.rs");
    let detach = source.find("fn detach(").unwrap();
    let cleanup = source[detach..]
        .find("remove_temporary_file_manager_replacement(&self.uuid);")
        .map(|offset| detach + offset)
        .unwrap();
    let permanent_close_boundary = source[detach..]
        .find("if !remote_terminal_state_survives_detach(detach_type)")
        .map(|offset| detach + offset)
        .unwrap();
    let unsubscribe = source[detach..]
        .find("// Unsubscribe from all views in the pane stack.")
        .map(|offset| detach + offset)
        .unwrap();

    assert!(permanent_close_boundary < cleanup);
    assert!(cleanup < unsubscribe);
}
