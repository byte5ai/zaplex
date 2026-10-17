use crate::render::model::RenderedSelection;

use super::*;

fn fake_reversible_actions() -> ReversibleEditorActions {
    ReversibleEditorActions {
        actions: vec![],
        selections: ReversibleSelectionState {
            next: RenderedSelectionSet::new(RenderedSelection::new(0.into(), 0.into())),
            reverse: RenderedSelectionSet::new(RenderedSelection::new(0.into(), 0.into())),
        },
        replacement_range: ReplacementRange {
            new_range: 0.into()..0.into(),
            old_range: 0.into()..0.into(),
        },
    }
}

#[test]
fn test_version_match_after_nonatomic_undo_redo() {
    // Make sure that after a series of nonatomic operations, the undo stack takes the newest version.
    let version1 = ContentVersion::new();
    let mut stack = UndoStack::new(5, version1);

    assert!(stack.version_match(&version1));

    let version2 = ContentVersion::new();
    stack.push_non_atomic_edit(fake_reversible_actions(), NonAtomicType::Insert, version2);
    let version3 = ContentVersion::new();
    stack.push_non_atomic_edit(fake_reversible_actions(), NonAtomicType::Insert, version3);

    assert!(stack.version_match(&version3));

    stack.undo();

    assert!(stack.version_match(&version1));

    stack.redo();

    assert!(stack.version_match(&version3));
}
