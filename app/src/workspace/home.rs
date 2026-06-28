//! Zaplex Home
//!
//! This is the landing page for new tabs if session creation isn't supported (e.g. on the web).
//! It's intentionally small and local-only.

use warpui::ViewContext;

use super::view::Workspace;
use crate::pane_group::{AnyPaneContent, FilePane};

const ZAPLEX_HOME_TITLE: &str = "Welcome to Zaplex";
const ZAPLEX_HOME_CONTENT: &str = r#"
Welcome to Zaplex.

Use this local workspace to:
* Create, view, and edit Zaplex Drive objects
* Manage local settings
* Work with local agent sessions, notebooks, and workflows"#;

/// Create a static "home page" pane.
pub fn create_home_pane(ctx: &mut ViewContext<Workspace>) -> Box<dyn AnyPaneContent> {
    let pane = FilePane::new(
        None,
        None,
        #[cfg(feature = "local_fs")]
        None,
        ctx,
    );
    pane.file_view(ctx).update(ctx, |pane, ctx| {
        pane.open_static(ZAPLEX_HOME_TITLE, ZAPLEX_HOME_CONTENT, ctx);
    });
    Box::new(pane)
}
