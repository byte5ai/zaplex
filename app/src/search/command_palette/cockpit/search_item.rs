use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warpui::elements::{Align, ConstrainedBox, Flex, ParentElement, Shrinkable, Text};
use warpui::{AppContext, Element, SingletonEntity};

use super::data_source::WAITING_SCORE_BONUS;
use crate::appearance::Appearance;
use crate::cockpit::palette::{CockpitPaletteKind, CockpitPaletteRecord, CockpitPaletteTarget};
use crate::search::action::search_item::styles;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util::render_search_item_icon;
use crate::search::item::SearchItem as SearchItemTrait;
use crate::search::result_renderer::ItemHighlightState;
use crate::ui_components::icons::Icon;

pub struct SearchItem {
    record: CockpitPaletteRecord,
    match_result: FuzzyMatchResult,
}

impl SearchItem {
    pub fn new(record: CockpitPaletteRecord, match_result: FuzzyMatchResult) -> Self {
        Self {
            record,
            match_result,
        }
    }

    fn icon(&self) -> Icon {
        match self.record.kind {
            CockpitPaletteKind::Account => Icon::User,
            CockpitPaletteKind::Session => Icon::TerminalInput,
            CockpitPaletteKind::Host => Icon::Server01,
            CockpitPaletteKind::Project => Icon::Folder,
            CockpitPaletteKind::GitHubFlow => Icon::Github,
        }
    }

    fn target(&self) -> CockpitPaletteTarget {
        self.record.target.clone()
    }
}

impl SearchItemTrait for SearchItem {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        render_search_item_icon(
            appearance,
            self.icon(),
            appearance.theme().foreground().into_solid(),
            highlight_state,
        )
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let primary = Text::new_inline(
            self.record.primary.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid())
        .finish();
        let secondary = Text::new_inline(
            self.record.secondary.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
        .finish();
        let row = Flex::row()
            .with_spacing(8.)
            .with_child(Shrinkable::new(1., Align::new(primary).left().finish()).finish())
            .with_child(secondary)
            .finish();
        ConstrainedBox::new(row)
            .with_height(styles::SEARCH_ITEM_HEIGHT)
            .finish()
    }

    fn score(&self) -> OrderedFloat<f64> {
        // A one-point bonus only breaks equal fuzzy matches. It does not let an
        // unrelated waiting result outrank a materially better textual match.
        OrderedFloat::from(
            self.match_result.score as f64
                + if self.record.waiting {
                    WAITING_SCORE_BONUS as f64
                } else {
                    0.
                },
        )
    }

    fn accept_result(&self) -> Self::Action {
        CommandPaletteItemAction::RunCockpitTarget {
            target: self.target(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        self.record.accessibility_label()
    }

    fn accessibility_help_message(&self) -> Option<String> {
        Some("Press enter to open this Cockpit target.".to_string())
    }

    fn dedup_key(&self) -> Option<String> {
        Some(format!("cockpit:{}", self.record.stable_key()))
    }
}
