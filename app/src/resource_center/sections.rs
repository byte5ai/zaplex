use warp_core::{context_flag::ContextFlag, features::FeatureFlag};
use warpui::ViewContext;

use super::{
    FeatureItem, FeatureSection, FeatureSectionData, ResourceCenterMainView, Section, Tip,
    TipAction, TipHint,
};

pub fn sections(ctx: &mut ViewContext<ResourceCenterMainView>) -> Vec<Section> {
    let mut sections = vec![Section::Changelog()];

    if FeatureFlag::AvatarInTabBar.is_enabled() {
        return sections;
    }

    let get_started = FeatureSectionData {
        section_name: FeatureSection::GettingStarted,
        items: vec![
            FeatureItem::new(
                crate::t!("resource-center-create-first-block-title"),
                crate::t!("resource-center-create-first-block-description"),
                Tip::Hint(TipHint::CreateBlock),
                ctx,
            ),
            FeatureItem::new(
                crate::t!("resource-center-navigate-blocks-title"),
                crate::t!("resource-center-navigate-blocks-description"),
                Tip::Hint(TipHint::BlockSelect),
                ctx,
            ),
            FeatureItem::new(
                crate::t!("resource-center-block-action-title"),
                crate::t!("resource-center-block-action-description"),
                Tip::Hint(TipHint::BlockAction),
                ctx,
            ),
            FeatureItem::new(
                crate::t!("resource-center-command-palette-title"),
                crate::t!("resource-center-command-palette-description"),
                Tip::Action(TipAction::CommandPalette),
                ctx,
            ),
            FeatureItem::new(
                crate::t!("resource-center-set-theme-title"),
                crate::t!("resource-center-set-theme-description"),
                Tip::Action(TipAction::ThemePicker),
                ctx,
            ),
        ],
    };
    sections.push(Section::Feature(get_started));

    let maximize_warp = FeatureSectionData {
        section_name: FeatureSection::MaximizeWarp,
        items: maximize_warp_items(ctx),
    };
    sections.push(Section::Feature(maximize_warp));

    // The upstream "Advanced setup" section is dropped: its two "View documentation"
    // items have no Zaplex docs destination, and the third linked warp.dev's blog —
    // foreign branding in a test user's hand. Restore it once Zaplex docs exist.

    sections
}

fn maximize_warp_items(ctx: &mut ViewContext<ResourceCenterMainView>) -> Vec<FeatureItem> {
    let mut maximize_warp_items = vec![];

    maximize_warp_items.push(FeatureItem::new(
        crate::t!("resource-center-command-search-title"),
        crate::t!("resource-center-command-search-description"),
        Tip::Action(TipAction::CommandSearch),
        ctx,
    ));

    maximize_warp_items.push(FeatureItem::new(
        crate::t!("resource-center-ai-command-search-title"),
        crate::t!("resource-center-ai-command-search-description"),
        Tip::Action(TipAction::AiCommandSearch),
        ctx,
    ));

    if ContextFlag::CreateNewSession.is_enabled() {
        maximize_warp_items.push(FeatureItem::new(
            crate::t!("resource-center-split-panes-title"),
            crate::t!("resource-center-split-panes-description"),
            Tip::Action(TipAction::SplitPane),
            ctx,
        ));
    }

    if ContextFlag::LaunchConfigurations.is_enabled() {
        maximize_warp_items.push(FeatureItem::new(
            crate::t!("resource-center-launch-configuration-title"),
            crate::t!("resource-center-launch-configuration-description"),
            Tip::Action(TipAction::SaveNewLaunchConfig),
            ctx,
        ));
    }

    maximize_warp_items
}
