//! Breadcrumb navigation rendering component
//!
//! Renders a clickable breadcrumb navigation based on the current path, supporting
//! step-by-step navigation to parent directories.
//! author: logic
//! date: 2026-05-26

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    ConstrainedBox, Container, Hoverable, MouseStateHandle, SavePosition, Text,
};
use warpui::platform::Cursor;
use warpui::Element;

use crate::sftp_manager::browser::SftpBrowserAction;
use crate::ui_components::icons::Icon;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BreadcrumbSegment {
    pub(crate) label: String,
    pub(crate) target: PathBuf,
}

/// Split a path into display labels and exact navigation targets.
///
/// The accumulated target starts at the path's own root component, so absolute
/// paths remain absolute while relative paths remain relative.
pub(crate) fn breadcrumb_segments(path: &Path) -> Vec<BreadcrumbSegment> {
    let mut accumulated = PathBuf::new();
    let mut segments = Vec::new();

    for component in path.components() {
        accumulated.push(component.as_os_str());
        let label = match component {
            Component::Prefix(prefix) => prefix.as_os_str().to_string_lossy().into_owned(),
            Component::RootDir => "/".to_string(),
            Component::CurDir => ".".to_string(),
            Component::ParentDir => "..".to_string(),
            Component::Normal(part) => part.to_string_lossy().into_owned(),
        };
        segments.push(BreadcrumbSegment {
            label,
            target: accumulated.clone(),
        });
    }

    if segments.is_empty() {
        segments.push(BreadcrumbSegment {
            label: ".".to_string(),
            target: PathBuf::new(),
        });
    }

    segments
}

/// Render the path breadcrumb navigation
///
/// Traverse each component of the path; each segment is clickable and triggers a NavigateTo action.
/// Segments are separated by ChevronRight icons; empty paths display ".".
pub fn render_breadcrumb(
    current_path: &Path,
    mouse_handles: &HashMap<PathBuf, MouseStateHandle>,
    appearance: &Appearance,
) -> Vec<Box<dyn Element>> {
    let theme = appearance.theme();
    let text_color = theme.active_ui_text_color();
    let sub_color = theme.sub_text_color(theme.background());
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    let segments = breadcrumb_segments(current_path);
    let mut elements: Vec<Box<dyn Element>> = Vec::new();

    for (i, segment) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;

        // Separator (added after the first segment)
        if i > 0 {
            let sep_icon =
                ConstrainedBox::new(Icon::ChevronRight.to_warpui_icon(sub_color).finish())
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

        let segment_label = segment.label.clone();
        let target_path = segment.target.clone();

        if is_last {
            // Last segment uses highlight color, not clickable
            let text_el = Text::new_inline(segment_label, ui_font, ui_font_size)
                .with_color(text_color.into())
                .finish();
            elements.push(Container::new(text_el).finish());
        } else {
            // Non-last segments are clickable for navigation
            let label_for_closure = segment_label.clone();
            let path = segment.target.display();
            let position_id = format!("sftp_breadcrumb:{path}");
            let Some(mouse_handle) = mouse_handles.get(&segment.target).cloned() else {
                continue;
            };
            let hoverable = Hoverable::new(mouse_handle, move |_| {
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
            let hit_target = ConstrainedBox::new(hoverable)
                .with_min_width(1.0)
                .with_min_height(ui_font_size)
                .finish();
            elements.push(SavePosition::new(hit_target, &position_id).finish());
        }
    }

    elements
}

#[cfg(test)]
#[path = "breadcrumb_tests.rs"]
mod tests;
