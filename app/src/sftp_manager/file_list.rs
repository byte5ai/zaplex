//! File list rendering component
//!
//! Provides rendering functionality for file list headers and file rows.
//! author: logic
//! date: 2026-05-26

use std::collections::HashSet;

use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ConstrainedBox, Container, CrossAxisAlignment, Expanded, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, SavePosition, Text,
};
use warpui::platform::Cursor;
use warpui::Element;

use crate::sftp_manager::browser::SftpBrowserAction;
use crate::sftp_manager::types::{format_size, FileEntry, FileEntryType};
use crate::ui_components::icons::Icon;

/// File size column width
const FILE_SIZE_WIDTH: f32 = 80.0;
/// File date column width
const FILE_DATE_WIDTH: f32 = 120.0;
/// Leading icon cell (16 px icon + 8 px row spacing) — the header mirrors it
/// so the Name header sits exactly over the names, not 24 px beside them.
const ICON_CELL: f32 = 16.0;
/// Row spacing between icon / name / columns.
const ROW_SPACING: f32 = 8.0;

/// Return appropriate icon based on file entry type
pub fn file_icon(entry_type: &FileEntryType) -> Icon {
    match entry_type {
        FileEntryType::Directory | FileEntryType::Symlink => Icon::Folder,
        FileEntryType::File | FileEntryType::Other => Icon::File,
    }
}

/// Render a single file row.
///
/// Two independent highlights, MC-style in SEMANTICS only: `is_cursor` is the
/// single keyboard cursor, `is_selected` the multi-selection set; the cursor
/// wins when a row is both, selection wins over hover. The LOOK is the app's
/// own vocabulary (polish audit FM.1) — an accent *tint* for the cursor, the
/// shared neutral overlays for selection and hover, text colours untouched —
/// not MC's full accent slab with inverted text, which made the cursor row a
/// foreign object in the theme.
pub fn render_file_row(
    entry: &FileEntry,
    index: usize,
    is_selected: bool,
    is_cursor: bool,
    mouse_handle: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let is_dir = matches!(entry.file_type, FileEntryType::Directory | FileEntryType::Symlink);
    let icon_color = if is_dir {
        theme.accent().into_solid()
    } else {
        theme.sub_text_color(theme.background()).into_solid()
    };
    let text_color = theme.active_ui_text_color();
    let sub_color = theme.sub_text_color(theme.background());

    let name = entry.name.clone();
    let file_type = entry.file_type;
    let size = entry.size;
    let modified = entry.modified.clone();
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    Hoverable::new(mouse_handle, move |mouse| {
        let bg: Option<Fill> = if is_cursor {
            Some(theme.accent().with_opacity(22))
        } else if is_selected {
            Some(internal_colors::fg_overlay_2(theme))
        } else if mouse.is_hovered() {
            Some(internal_colors::fg_overlay_1(theme))
        } else {
            None
        };

        // Icon
        let icon_el = ConstrainedBox::new(
            file_icon(&file_type).to_warpui_icon(icon_color.into()).finish(),
        )
        .with_width(ICON_CELL)
        .with_height(ICON_CELL)
        .finish();

        // Name — `Expanded` (tight fit), not `Shrinkable` (loose fit): only a
        // tight-fit child actually consumes the remaining width, and consuming
        // it is what pins Size/Modified into stable right columns.
        let name_el = Expanded::new(
            1.0,
            Text::new_inline(name.clone(), ui_font, ui_font_size)
                .with_color(text_color.into())
                .finish(),
        )
        .finish();

        // Size — right-aligned in its fixed column, like every numeric column.
        let size_text = if matches!(file_type, FileEntryType::Directory | FileEntryType::Symlink) {
            String::from("--")
        } else {
            format_size(size)
        };
        let size_el = ConstrainedBox::new(
            Flex::row()
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(
                    Text::new_inline(size_text, ui_font, ui_font_size)
                        .with_color(sub_color.into())
                        .finish(),
                )
                .finish(),
        )
        .with_width(FILE_SIZE_WIDTH)
        .finish();

        // Modified date
        let date_text = modified.clone().unwrap_or_else(|| String::from("--"));
        let date_el = ConstrainedBox::new(
            Text::new_inline(date_text, ui_font, ui_font_size)
                .with_color(sub_color.into())
                .finish(),
        )
        .with_width(FILE_DATE_WIDTH)
        .finish();

        // Assemble row content. `Max` is what pins Size/Modified into stable
        // columns — without it the row hugged its content and the columns
        // wandered with the name length (polish audit FM.2).
        let row_content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(ROW_SPACING)
            .with_child(icon_el)
            .with_child(name_el)
            .with_child(size_el)
            .with_child(date_el)
            .finish();

        let mut container = Container::new(row_content)
            .with_padding_left(8.0)
            .with_padding_right(8.0)
            .with_padding_top(4.0)
            .with_padding_bottom(4.0);
        if let Some(bg) = bg {
            container = container.with_background(bg);
        }
        container.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(SftpBrowserAction::SelectEntry(index));
    })
    .on_double_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(SftpBrowserAction::OpenEntry(index));
    })
    .on_right_click(move |ctx, _, position| {
        use super::browser::SFTP_PANEL_POSITION_ID;
        let offset = match ctx.element_position_by_id(SFTP_PANEL_POSITION_ID) {
            Some(bounds) => position - bounds.origin(),
            None => position,
        };
        ctx.dispatch_typed_action(SftpBrowserAction::ContextMenu {
            index,
            position: offset,
        });
    })
    .finish()
}

/// Render file list header — localized, mirroring the rows' column grid
/// (leading icon cell included) and separated from them by a hairline.
pub fn render_header(appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let header_color = theme.sub_text_color(theme.background());
    let family = appearance.ui_font_family();
    let size = appearance.ui_font_size();

    // The rows lead with a 16 px icon; the header mirrors that cell so "Name"
    // sits over the names (audit FM.2 — the header had no icon cell at all,
    // so "Name" sat a fixed 24 px beside the names, with a magic spacing 24
    // standing in for icon + gap between the other columns).
    let icon_spacer = ConstrainedBox::new(Flex::row().finish())
        .with_width(ICON_CELL)
        .finish();

    let name_el = Expanded::new(
        1.0,
        Text::new_inline(crate::t!("fm-col-name"), family, size)
            .with_color(header_color.into())
            .finish(),
    )
    .finish();

    let size_el = ConstrainedBox::new(
        Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(
                Text::new_inline(crate::t!("fm-col-size"), family, size)
                    .with_color(header_color.into())
                    .finish(),
            )
            .finish(),
    )
    .with_width(FILE_SIZE_WIDTH)
    .finish();

    let date_el = ConstrainedBox::new(
        Text::new_inline(crate::t!("fm-col-modified"), family, size)
            .with_color(header_color.into())
            .finish(),
    )
    .with_width(FILE_DATE_WIDTH)
    .finish();

    let header_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_spacing(ROW_SPACING)
        .with_child(icon_spacer)
        .with_child(name_el)
        .with_child(size_el)
        .with_child(date_el)
        .finish();

    Container::new(header_row)
        .with_padding_left(8.0)
        .with_padding_right(8.0)
        .with_padding_top(4.0)
        .with_padding_bottom(4.0)
        .with_border(Border::bottom(1.0).with_border_fill(theme.split_pane_border_color()))
        .finish()
}

/// Render list of file rows.
///
/// `cursor_entry_index` is the entry index of the MC keyboard cursor (the
/// single accent-highlighted row), or `None` when the pane isn't the active
/// keyboard target.
pub fn render_file_rows(
    entries: &[FileEntry],
    filtered_indices: &[usize],
    selected: &HashSet<usize>,
    cursor_entry_index: Option<usize>,
    mouse_handles: &[MouseStateHandle],
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    for &index in filtered_indices {
        let entry = &entries[index];
        let is_selected = selected.contains(&index);
        let is_cursor = cursor_entry_index == Some(index);
        let mouse_handle = mouse_handles.get(index).cloned().unwrap_or_default();
        let row = render_file_row(entry, index, is_selected, is_cursor, mouse_handle, appearance);
        let position_id = format!("sftp_row:{index}");
        let positioned = SavePosition::new(row, &position_id).finish();
        col.add_child(positioned);
    }

    col.finish()
}
