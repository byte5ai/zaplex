use super::*;

#[test]
fn agent_guardrail_dialog_participates_in_modal_focus_and_close_all() {
    warpui::App::test((), |app| async move {
        let mut state = WorkspaceState {
            is_agent_guardrail_dialog_open: true,
            ..Default::default()
        };
        app.read(|ctx| {
            assert!(state.is_any_non_palette_modal_open(ctx));
            assert!(state.is_any_non_terminal_view_open(ctx));
        });
        state.close_all_modals();
        assert!(!state.is_agent_guardrail_dialog_open);
    });
}
