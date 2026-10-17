use super::LocalUndoStack;
use crate::editor::{
    view::model::{
        buffer::{
            time::{Lamport, LamportValue},
            ReplicaId,
        },
        Anchor, LocalSelection, LocalSelections,
    },
    PlainTextEditorViewAction,
};
use vec1::vec1;

fn local_selections(start: Anchor, end: Anchor) -> LocalSelections {
    LocalSelections {
        pending: None,
        selections: vec1![LocalSelection::new_for_test(start, end)],
        marked_text_state: Default::default(),
    }
}

fn lamport(value: impl Into<LamportValue>) -> Lamport {
    Lamport {
        replica_id: ReplicaId::new(1),
        value: value.into(),
    }
}

#[test]
fn test_undo_stack_undo_fork() {
    let mut undo_stack = LocalUndoStack::new(local_selections(Anchor::Start, Anchor::Start));

    // [`Action::ReplaceBuffer`] is an atomic action so these edits won't be coalesced.
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [0.into()].into_iter(),
        local_selections(Anchor::Start, Anchor::End),
    );
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [1.into()].into_iter(),
        local_selections(Anchor::End, Anchor::Start),
    );
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [2.into()].into_iter(),
        local_selections(Anchor::End, Anchor::End),
    );

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, vec![2.into()]);
    assert_eq!(selections, local_selections(Anchor::End, Anchor::Start));

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, vec![1.into()]);
    assert_eq!(selections, local_selections(Anchor::Start, Anchor::End));

    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [4.into()].into_iter(),
        local_selections(Anchor::End, Anchor::End),
    );

    assert!(undo_stack.redo().is_none());

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, vec![4.into()]);
    assert_eq!(selections, local_selections(Anchor::Start, Anchor::End));

    let (ops, selections) = undo_stack.redo().unwrap();
    assert_eq!(ops, vec![4.into()]);
    assert_eq!(selections, local_selections(Anchor::End, Anchor::End));

    assert!(undo_stack.redo().is_none());
}
