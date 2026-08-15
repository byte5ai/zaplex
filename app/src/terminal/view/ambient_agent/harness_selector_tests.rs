use super::*;

#[test]
fn new_harness_picker_does_not_offer_legacy_gemini() {
    assert!(!SELECTABLE_HARNESSES.contains(&Harness::Gemini));
    assert!(SELECTABLE_HARNESSES.contains(&Harness::Claude));
}
