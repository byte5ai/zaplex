//! Breadcrumb navigation rendering component
//!
//! Renders a clickable breadcrumb navigation based on the current path, supporting
//! step-by-step navigation to parent directories.
//! author: logic
//! date: 2026-05-26

use std::path::{Component, PathBuf};

use warp_core::ui::appearance::Appearance;
use warpui::elements::{ConstrainedBox, Container, Hoverable, SavePosition, Text};
use warpui::platform::Cursor;
use warpui::Element;

use crate::sftp_manager::browser::SftpBrowserAction;
use crate::ui_components::icons::Icon;

/// Render the path breadcrumb navigation
///
/// Traverse each component of the path; each segment is clickable and triggers a NavigateTo action.
/// Segments are separated by ChevronRight icons; empty paths display "/".
pub fn render_breadcrumb(current_path: &PathBuf, appearance: &Appearance) -> Vec<Box<dyn Element>> {
    let theme = appearance.theme();
    let text_color = theme.active_ui_text_color();
    let sub_color = theme.sub_text_color(theme.background());
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    let components: Vec<_> = current_path
        .components()
        .filter(|c| !matches!(c, Component::RootDir))
        .collect();

    // When path is empty or only has root, display only "/"
    if components.is_empty() {
        let root_el = Text::new_inline(String::from("/"), ui_font, ui_font_size)
            .with_color(text_color.into())
            .finish();
        return vec![Container::new(root_el).finish()];
    }

    let mut elements: Vec<Box<dyn Element>> = Vec::new();
    let mut accumulated = PathBuf::new();

    for (i, comp) in components.iter().enumerate() {
        accumulated.push(comp);
        let is_last = i == components.len() - 1;

        // Separator (added after the first segment)
        if i > 0 {
            let sep_icon = ConstrainedBox::new(
                Icon::ChevronRight
                    .to_warpui_icon(sub_color.into())
                    .finish(),
            )
            .with_width(12.0)
            .with_height(12.0)
            .finish();
            elements.push(
                Container::new(sep_icon)
                    .with_padding_left(2.0)
                    .with_padding_right(2.0)
                    .finish(),
            );
        }

        let segment_label = comp.as_os_str().to_string_lossy().to_string();
        let target_path = accumulated.clone();

        if is_last {
            // Last segment uses highlight color, not clickable
            let text_el = Text::new_inline(segment_label, ui_font, ui_font_size)
                .with_color(text_color.into())
                .finish();
            elements.push(Container::new(text_el).finish());
        } else {
            // Non-last segments are clickable for navigation
            let label_for_closure = segment_label.clone();
            let path = accumulated.display();
            let position_id = format!("sftp_breadcrumb:{path}");
            let hoverable = Hoverable::new(Default::default(), move |_| {
                let text_el = Text::new_inline(label_for_closure.clone(), ui_font, ui_font_size)
                    .with_color(sub_color.into())
                    .finish();
                Container::new(text_el).finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SftpBrowserAction::NavigateTo(target_path.clone()));
            })
            .finish();
            elements.push(SavePosition::new(hoverable, &position_id).finish());
        }
    }

    elements
}
