use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;

use crate::elements::ConstrainedBox;
use crate::elements::Rect;
use crate::platform::WindowStyle;
use crate::Entity;
use crate::View;
use crate::WindowId;
use crate::{App, TypedActionView};

use super::*;

struct TestRootView {
    parent_size: Vector2F,
    children_sizes: Vec<Vector2F>,
    axis: Axis,
    run_spacing: f32,
}

impl TestRootView {
    pub fn new(
        parent_size: Vector2F,
        children_sizes: Vec<Vector2F>,
        axis: Axis,
        run_spacing: f32,
    ) -> Self {
        Self {
            parent_size,
            children_sizes,
            axis,
            run_spacing,
        }
    }
}

impl Entity for TestRootView {
    type Event = ();
}

impl View for TestRootView {
    fn ui_name() -> &'static str {
        "Wrap::tests::TestRootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let mut wrap = Wrap::new(self.axis).with_run_spacing(self.run_spacing);
        wrap.extend(self.children_sizes.iter().map(|size| {
            ConstrainedBox::new(Rect::new().finish())
                .with_height(size.y())
                .with_width(size.x())
                .finish()
        }));

        ConstrainedBox::new(wrap.finish())
            .with_width(self.parent_size.x())
            .with_height(self.parent_size.y())
            .finish()
    }
}

impl TypedActionView for TestRootView {
    type Action = ();
}

type RenderFn = dyn Fn(&AppContext) -> Box<dyn Element> + 'static;

struct TestDynamicView {
    render: Box<RenderFn>,
}

impl TestDynamicView {
    fn new(render: impl Fn(&AppContext) -> Box<dyn Element> + 'static) -> Self {
        Self {
            render: Box::new(render),
        }
    }
}

impl Entity for TestDynamicView {
    type Event = ();
}

impl View for TestDynamicView {
    fn ui_name() -> &'static str {
        "Wrap::tests::TestDynamicView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        (self.render)(app)
    }
}

impl TypedActionView for TestDynamicView {
    type Action = ();
}

/// Asserts that the bounds of all the painted rects match that of `rects`.
fn assert_bounds_of_rects(
    app: &mut App,
    window_id: WindowId,
    rects: impl IntoIterator<Item = RectF>,
) {
    let presenter_ref = app
        .presenter(window_id)
        .expect("Test window should have a presenter since first frame is rendered.");
    let presenter = presenter_ref.borrow();
    let scene = presenter
        .scene()
        .expect("Presenter should have rendered a scene after the test_view was updated.");

    let actual_rects = scene
        .layers()
        .flat_map(|layer| layer.rects.iter())
        .map(|rect| rect.bounds);

    itertools::assert_equal(actual_rects, rects);
}

#[test]
fn test_wrap_column_spacing() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test spacing in column wrap
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                let child_size = vec2f(50., 80.);
                let mut wrap = Wrap::column().with_spacing(12.);
                wrap.extend((0..4).map(|_| {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(child_size.y())
                        .with_width(child_size.x())
                        .finish()
                }));

                ConstrainedBox::new(wrap.finish())
                    .with_width(200.)
                    .with_height(185.) // Can fit 2 children per column: 80 + 12 + 80 = 172
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        let child_size = vec2f(50., 80.);
        // Two columns with 2 children each
        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), child_size),
                RectF::new(vec2f(0., 92.), child_size), // 80 + 12
                RectF::new(vec2f(50., 0.), child_size), // New column
                RectF::new(vec2f(50., 92.), child_size), // 80 + 12 in second column
            ],
        );
    })
}
