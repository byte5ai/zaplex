use super::did_preedit_text_change;

#[test]
fn changed_or_initial_preedit_text_repositions() {
    let previous = ("old".to_string(), Some((1, 1)));

    assert!(did_preedit_text_change(Some(&previous), "new"));
    assert!(did_preedit_text_change(None, "new"));
}
