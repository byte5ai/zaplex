use crate::ai::llms::{DisableReason, LLMId, LLMInfo};
use crate::menu::{MenuItem, MenuItemFields, MenuTooltipPosition};
use itertools::Itertools;
use std::sync::Arc;
use warp_core::ui::Icon;
use warpui::{
    elements::{
        CrossAxisAlignment, Flex, ParentElement, SavePosition, Shrinkable, Text,
    },
    fonts::{Properties, Style},
    Action, Element,
};

pub fn is_auto(llm: &LLMInfo) -> bool {
    llm.display_name.to_lowercase().contains("auto")
        || llm.id.to_string().to_lowercase().contains("auto")
}

/// Returns true if the given model has other variants with different reasoning levels.
pub fn has_reasoning_variants(llm: &LLMInfo, all_models: &[&LLMInfo]) -> bool {
    if !llm.has_reasoning_level() {
        return false;
    }
    all_models
        .iter()
        .filter(|other| other.base_model_name() == llm.base_model_name() && other.id != llm.id)
        .any(|other| other.has_reasoning_level())
}

fn with_cost_and_profile_info<A: Action + Clone>(
    item: MenuItemFields<A>,
    llm: &LLMInfo,
    profile_default_model: Option<&LLMId>,
) -> MenuItemFields<A> {
    let mut label = String::new();

    if Some(&llm.id) == profile_default_model {
        label.push_str("Profile default");
    }

    match llm.usage_metadata.credit_multiplier {
        Some(mult) if mult != 1. => {
            let mut formatted_cost = format!("~{mult:.1}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string();
            formatted_cost.push('x');
            if label.is_empty() {
                label.push_str(&formatted_cost);
            } else {
                label.push_str(&format!(" ({formatted_cost})"));
            }
        }
        _ => {}
    }

    if label.is_empty() {
        item
    } else {
        // Using the key shortcut label to display extra info is a hack.
        item.with_key_shortcut_label(Some(label))
    }
}

fn make_item_fields<A: Action + Clone>(
    llm: &LLMInfo,
    action: impl Fn(&LLMInfo) -> A,
    position_id_fn: Option<&dyn Fn(&LLMId) -> String>,
    model_id_to_add_profile_default_label_to: Option<&LLMId>,
    collapse_auto: bool,
    collapse_reasoning_variants: bool,
) -> MenuItem<A> {
    let label = if collapse_auto && is_auto(llm) {
        "auto".to_string()
    } else if collapse_reasoning_variants && llm.has_reasoning_level() {
        llm.base_model_name().to_string()
    } else {
        llm.menu_display_name()
    };
    let mut item = if let Some(position_id_fn) = position_id_fn {
        let position_id = position_id_fn(&llm.id);
        MenuItemFields::new_with_custom_label(
            Arc::new(move |_, _, appearance, _| {
                let mut item_row =
                    Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

                let text = Text::new(
                    label.clone(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().background())
                        .into(),
                )
                .finish();
                item_row.add_child(Shrinkable::new(4., text).finish());
                SavePosition::new(item_row.finish(), &position_id).finish()
            }),
            None,
        )
    } else {
        let provider_icon = llm.provider.icon().unwrap_or(Icon::Oz);
        MenuItemFields::new(label).with_icon(provider_icon)
    };

    item = item
        .with_on_select_action(action(llm))
        .with_disabled(llm.disable_reason.is_some());

    if let Some(reason) = &llm.disable_reason {
        item = item
            .with_tooltip(reason.tooltip_text())
            .with_tooltip_position(MenuTooltipPosition::Above);

        if matches!(reason, DisableReason::RequiresUpgrade) {
            item =
                item.with_right_side_label("disabled", Properties::default().style(Style::Italic));
        }
    }

    with_cost_and_profile_info(item, llm, model_id_to_add_profile_default_label_to).into_item()
}

pub fn available_model_menu_items<A: Action + Clone>(
    choices: Vec<&LLMInfo>,
    action: impl Fn(&LLMInfo) -> A,
    model_id_to_add_profile_default_label_to: Option<&LLMId>,
    position_id_fn: Option<&dyn Fn(&LLMId) -> String>,
    collapse_auto: bool,
    collapse_reasoning_variants: bool,
) -> Vec<MenuItem<A>> {
    choices
        .into_iter()
        .map(|llm| {
            make_item_fields(
                llm,
                &action,
                position_id_fn,
                model_id_to_add_profile_default_label_to,
                collapse_auto,
                collapse_reasoning_variants,
            )
        })
        .collect_vec()
}
