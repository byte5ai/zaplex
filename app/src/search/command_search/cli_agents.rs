use ordered_float::OrderedFloat;
use warp_core::ui::builder;
use warpui::{
    elements::{ConstrainedBox, Container, Text},
    AppContext, Element,
};

use crate::{
    appearance::Appearance,
    search::{
        command_search::searcher::{AcceptedHistoryItem, CommandSearchItemAction},
        data_source::{Query, QueryResult},
        item::SearchItem,
        mixer::{DataSourceRunErrorWrapper, SyncDataSource},
        result_renderer::ItemHighlightState,
    },
    terminal::CLIAgent,
};

const SEARCHABLE_AGENTS: &[CLIAgent] = &[CLIAgent::Antigravity, CLIAgent::Grok];

fn match_agent(agent: CLIAgent, query: &str) -> Option<fuzzy_match::FuzzyMatchResult> {
    let query = query.trim();
    let normalized = query.to_ascii_lowercase();
    if normalized.len() < 2 {
        return None;
    }

    let terms = std::iter::once(agent.display_name())
        .chain(std::iter::once(agent.command_prefix()))
        .chain(agent.command_search_aliases().iter().copied());
    if !terms
        .clone()
        .any(|term| term.to_ascii_lowercase().starts_with(&normalized))
    {
        return None;
    }

    let searchable = terms.collect::<Vec<_>>().join(" ");
    fuzzy_match::match_indices_case_insensitive(&searchable, query)
}

#[derive(Clone, Debug)]
struct CliAgentSearchItem {
    agent: CLIAgent,
    score: f64,
}

impl CliAgentSearchItem {
    fn result_text(&self) -> String {
        format!(
            "{} ({})",
            self.agent.display_name(),
            self.agent.command_prefix()
        )
    }
}

impl SearchItem for CliAgentSearchItem {
    type Action = CommandSearchItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon = self
            .agent
            .icon()
            .expect("searchable CLI agents have bundled icons")
            .to_warpui_icon(highlight_state.icon_fill(appearance))
            .finish();
        Container::new(
            ConstrainedBox::new(icon)
                .with_width(appearance.monospace_font_size())
                .with_height(appearance.monospace_font_size())
                .finish(),
        )
        .with_margin_right(8.)
        .finish()
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        Text::new_inline(
            self.result_text(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .autosize_text(builder::MIN_FONT_SIZE)
        .with_color(highlight_state.main_text_fill(appearance).into_solid())
        .finish()
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.score)
    }

    fn priority_tier(&self) -> u8 {
        // A canonical first-class result must beat a legacy history row. This
        // is especially important for the `gemini` alias, which must launch agy.
        1
    }

    fn accept_result(&self) -> CommandSearchItemAction {
        CommandSearchItemAction::AcceptHistory(AcceptedHistoryItem {
            command: self.agent.command_prefix().to_string(),
            linked_workflow_data: None,
        })
    }

    fn execute_result(&self) -> CommandSearchItemAction {
        CommandSearchItemAction::ExecuteHistory(self.agent.command_prefix().to_string())
    }

    fn accessibility_label(&self) -> String {
        format!(
            "{}: run {}",
            self.result_text(),
            self.agent.command_prefix()
        )
    }
}

pub struct CliAgentsDataSource;

impl SyncDataSource for CliAgentsDataSource {
    type Action = CommandSearchItemAction;

    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        Ok(SEARCHABLE_AGENTS
            .iter()
            .filter_map(|agent| {
                match_agent(*agent, &query.text).map(|matched| {
                    CliAgentSearchItem {
                        agent: *agent,
                        score: matched.score as f64,
                    }
                    .into()
                })
            })
            .collect())
    }
}

#[cfg(test)]
#[path = "cli_agents_tests.rs"]
mod tests;
