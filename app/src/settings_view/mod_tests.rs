use super::editor_context_menu_visibility;

#[test]
fn editor_context_menu_respects_selection_editability_and_password_safety() {
    let cases = [
        ((true, true, false), (true, true, true)),
        ((false, true, false), (false, false, true)),
        ((true, false, false), (false, true, false)),
        ((true, true, true), (false, false, true)),
        ((true, false, true), (false, false, false)),
    ];

    for ((has_selection, can_edit, is_password), expected) in cases {
        assert_eq!(
            editor_context_menu_visibility(has_selection, can_edit, is_password),
            expected
        );
    }
}
