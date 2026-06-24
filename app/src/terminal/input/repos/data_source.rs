//! Async data source for the inline repos menu.
//!
//! Historically, this pulled a list of "previously opened git repositories" from `PersistedWorkspace`.
//! After LSP + workspace sunset, this candidate source no longer exists, so this data source
//! only retains trait and view wiring, always returns empty results — meaning the menu can still be
//! invoked but never has candidates. This avoids major changes to upper-layer view / suggestions mode
//! wiring; if future implementation wants to connect "current pane group real-time cwd", we can add data source back then.

use warpui::{AppContext, Entity};

use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{AsyncDataSource, BoxFuture, DataSourceRunErrorWrapper};
use crate::terminal::input::repos::AcceptRepo;

pub struct RepoMenuDataSource;

impl RepoMenuDataSource {
    pub fn new() -> Self {
        Self
    }
}

impl AsyncDataSource for RepoMenuDataSource {
    type Action = AcceptRepo;

    fn run_query(
        &self,
        _query: &Query,
        _app: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>> {
        Box::pin(async move { Ok(Vec::new()) })
    }
}

impl Entity for RepoMenuDataSource {
    type Event = ();
}
