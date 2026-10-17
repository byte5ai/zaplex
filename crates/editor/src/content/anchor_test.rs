use std::cmp::Ordering;

use warpui::App;

use super::AnchorSide;

use crate::content::{
    anchor::Anchor, buffer::Buffer, selection_model::BufferSelectionModel, text::IndentBehavior,
};
use string_offset::CharOffset;

#[test]
fn test_anchor_comparison() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::zero()..CharOffset::zero(),
                "some text",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let first = selection.update(ctx, |selection, _| {
                selection.create_anchor(CharOffset::from(1), AnchorSide::Right)
            });

            // Anchors should be equal to themselves.
            assert_eq!(
                first.cmp(&first.clone(), selection.as_ref(ctx)),
                Some(Ordering::Equal)
            );

            // Anchors should be equal to other anchors with the same offset.
            let first2 = selection.update(ctx, |selection, _| {
                selection.create_anchor(CharOffset::from(1), AnchorSide::Right)
            });
            assert_ne!(first.id, first2.id);
            assert_eq!(
                first.cmp(&first2, selection.as_ref(ctx)),
                Some(Ordering::Equal)
            );

            // Unequal anchors compare by offset.
            let second = selection.update(ctx, |selection, _| {
                selection.create_anchor(CharOffset::from(5), AnchorSide::Right)
            });
            assert_eq!(
                first.cmp(&second, selection.as_ref(ctx)),
                Some(Ordering::Less)
            );
            assert_eq!(
                second.cmp(&first, selection.as_ref(ctx)),
                Some(Ordering::Greater)
            );

            // Invalid anchors do not compare - delete the range containing `second`.
            buffer.edit_internal_first_selection(
                CharOffset::from(3)..CharOffset::from(7),
                "",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(second.cmp(&first, selection.as_ref(ctx)), None);
            assert_eq!(first.cmp(&second, selection.as_ref(ctx)), None);
        });
    });
}

impl Anchor {
    pub fn cmp(&self, other: &Anchor, selection: &BufferSelectionModel) -> Option<Ordering> {
        if self.id == other.id {
            Some(Ordering::Equal)
        } else {
            Some(
                selection
                    .resolve_anchor(self)?
                    .cmp(&selection.resolve_anchor(other)?),
            )
        }
    }
}
