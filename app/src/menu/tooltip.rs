use pathfinder_geometry::vector::Vector2F;
use warpui::{
    elements::{Element, Point},
    event::DispatchedEvent,
    AfterLayoutContext, AppContext, EventContext, LayoutContext, PaintContext, SizeConstraint,
};

/// A menu hint in its own final Stack overlay, without pointer ownership.
pub(super) struct MenuTooltip {
    child: Box<dyn Element>,
}

impl MenuTooltip {
    pub(super) fn new(child: Box<dyn Element>) -> Self {
        Self { child }
    }
}

impl Element for MenuTooltip {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app);
        ctx.scene.set_active_layer_click_through();
    }

    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
}
