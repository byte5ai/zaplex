use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use itertools::Itertools;
use warpui::{AppContext, Entity, SingletonEntity};

use super::search_item::SearchItem;
use crate::cockpit::github_flows::RepositoryContext;
use crate::cockpit::model::CockpitModel;
use crate::cockpit::palette::{build_palette_index, CockpitPaletteKind};
use crate::cockpit::settings::CockpitSettings;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::files::model::FileSearchModel;
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
use crate::terminal::cli_agent::CLIAgentInstallModel;

const MAX_COCKPIT_RESULTS: usize = 200;
const EXACT_CLASS_OR_LABEL_BONUS: i64 = 2;

fn class_label(kind: CockpitPaletteKind) -> &'static str {
    match kind {
        CockpitPaletteKind::Account => "account",
        CockpitPaletteKind::Session => "session",
        CockpitPaletteKind::Host => "host",
        CockpitPaletteKind::Project => "project",
        CockpitPaletteKind::GitHubFlow => "github workflow",
    }
}

fn search_records(
    records: Vec<crate::cockpit::palette::CockpitPaletteRecord>,
    query: &str,
) -> Vec<QueryResult<CommandPaletteItemAction>> {
    let query = query.trim();
    let mut matches = records
        .into_iter()
        .filter_map(|record| {
            let mut match_result = if query.is_empty() {
                FuzzyMatchResult::no_match()
            } else {
                [
                    match_indices_case_insensitive(&record.search_text, query),
                    match_indices_case_insensitive(class_label(record.kind), query),
                ]
                .into_iter()
                .flatten()
                .max_by_key(|result| result.score)?
            };
            if !query.is_empty()
                && (record.primary.trim().eq_ignore_ascii_case(query)
                    || class_label(record.kind).eq_ignore_ascii_case(query))
            {
                match_result.score = match_result
                    .score
                    .saturating_add(EXACT_CLASS_OR_LABEL_BONUS);
            }
            let score = match_result.score as f64 + if record.waiting { 1. } else { 0. };
            Some((score, record, match_result))
        })
        .collect_vec();
    matches.sort_by(|(score_a, record_a, _), (score_b, record_b, _)| {
        score_b
            .partial_cmp(score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| record_a.stable_key().cmp(record_b.stable_key()))
    });
    matches
        .into_iter()
        .take(MAX_COCKPIT_RESULTS)
        .map(|(_, record, match_result)| SearchItem::new(record, match_result).into())
        .collect_vec()
}

#[derive(Default)]
pub struct DataSource;

impl DataSource {
    pub fn new() -> Self {
        Self
    }

    fn records(&self, app: &AppContext) -> Vec<crate::cockpit::palette::CockpitPaletteRecord> {
        if !*CockpitSettings::as_ref(app).enabled {
            return Vec::new();
        }
        let model = CockpitModel::as_ref(app);
        let installed_agents = CLIAgentInstallModel::as_ref(app);
        let has_analysis_account = [
            zaplex_cockpit::Provider::Claude,
            zaplex_cockpit::Provider::Codex,
        ]
        .into_iter()
        .any(|provider| {
            installed_agents.is_cli_agent_installed(crate::cockpit::agent_of(provider))
                && zaplex_cockpit::pick_freest_checked(provider, model.snapshot()).is_some()
        });
        let github_cli_available = cfg!(not(target_family = "wasm"))
            && crate::util::path::resolve_executable("gh").is_some();
        let repository = (has_analysis_account && github_cli_available)
            .then(|| FileSearchModel::as_ref(app).repo_root(app))
            .flatten()
            .as_deref()
            .and_then(|root| RepositoryContext::discover(root).ok());
        build_palette_index(model.snapshot(), model.inventory(), repository.as_ref())
    }

    pub fn query_result(
        &self,
        stable_key: &str,
        app: &AppContext,
    ) -> Option<QueryResult<CommandPaletteItemAction>> {
        self.records(app)
            .into_iter()
            .find(|record| record.stable_key() == stable_key)
            .map(|record| SearchItem::new(record, FuzzyMatchResult::no_match()).into())
    }
}

impl Entity for DataSource {
    type Event = ();
}

impl SyncDataSource for DataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        Ok(search_records(self.records(app), &query.text))
    }
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
