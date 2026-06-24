use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use itertools::Itertools;
use warpui::{AppContext, Entity};

use super::SshServerSearchItem;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};

use warp_ssh_manager::{NodeKind, SshRepository};

/// Upper limit. SSH servers are typically a handful to a few dozen, won't explode.
const MAX_SSH_SERVERS_CONSIDERED: usize = 200;

#[derive(Default)]
pub struct SshServersDataSource;

impl SshServersDataSource {
    pub fn new() -> Self {
        Self
    }
}

impl Entity for SshServersDataSource {
    type Event = ();
}

impl SyncDataSource for SshServersDataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        // Use our own with_conn (separate write connection), don't pollute PaneGroup's main write thread.
        // DataSourceRunErrorWrapper is a Box<dyn DataSourceRunError> custom trait; wrapping cost
        // is too high — on failure, log and return empty results (SSH won't show in palette,
        // but other sources are unaffected).
        let nodes = match warp_ssh_manager::with_conn(|c| Ok(SshRepository::list_nodes(c)?)) {
            Ok(n) => n,
            Err(e) => {
                log::warn!("command palette ssh: failed to load nodes: {e}");
                return Ok(Vec::new());
            }
        };

        // Only show server nodes. Fetch details for each node once, skip failures
        // (folders have no details and return None).
        let server_nodes: Vec<_> = nodes
            .into_iter()
            .filter(|n| matches!(n.kind, NodeKind::Server))
            .take(MAX_SSH_SERVERS_CONSIDERED)
            .collect();

        let query_str = query.text.as_str();
        let results = server_nodes
            .into_iter()
            .filter_map(|node| {
                let server =
                    warp_ssh_manager::with_conn(|c| Ok(SshRepository::get_server(c, &node.id)?))
                        .ok()
                        .flatten()?;

                // Use name + " " + host as search text; matching either name or host is fine.
                let display_name = node.name.clone();
                let host_user = if server.username.is_empty() {
                    server.host.clone()
                } else {
                    format!("{}@{}", server.username, server.host)
                };
                let haystack = format!("{display_name} {host_user}");

                let match_result = if query_str.is_empty() {
                    Some(FuzzyMatchResult::no_match())
                } else {
                    match_indices_case_insensitive(&haystack, query_str)
                }?;

                let mut item = SshServerSearchItem::new(node, server, host_user, display_name);
                let mut mr = match_result;
                // Similar to RepoDataSource, slightly boost score to make SSH results competitive in the mixed panel.
                mr.score *= 4;
                item.match_result = mr;
                Some(item.into())
            })
            .collect_vec();

        Ok(results)
    }
}
