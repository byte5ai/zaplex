//! File list rendering component
//!
//! Provides rendering functionality for file list headers and file rows.
//! author: logic
//! date: 2026-05-26

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Expanded, Flex,
    Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius,
    SavePosition, Text,
};
use warpui::platform::Cursor;
use warpui::Element;

use crate::sftp_manager::browser::{SftpBrowserAction, SortColumn, SortState};
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
        FileEntryType::Directory => Icon::Folder,
        FileEntryType::Symlink => Icon::LinkHorizontal,
        FileEntryType::File | FileEntryType::Other => Icon::File,
    }
}

/// The leading cell of a row: the type icon at rest, and the **mark zone** on
/// approach — hovering swaps in a check outline, clicking toggles the
/// multi-selection. Mouse parity for MC's Insert/Space, which is the only way
/// to offer marking here at all: the framework's click handlers carry no
/// modifier state, so ctrl/shift-click cannot be implemented (2026-07-19).
///
/// A *marked* row keeps its file-type icon — recoloured in the accent, like
/// the rest of its line (NC/MC's colour-marking; RC acceptance 2026-07-21:
/// marks are colour, not a checkmark). The check glyph appears only under the
/// pointer, as the toggle affordance.
fn mark_cell(
    index: usize,
    entry_type: FileEntryType,
    is_marked: bool,
    icon_color: Fill,
    accent: Fill,
    muted: Fill,
    handle: MouseStateHandle,
) -> Box<dyn Element> {
    Hoverable::new(handle, move |mouse| {
        let (icon, color) = if mouse.is_hovered() {
            // The toggle affordance; accent when it would unmark, muted when
            // it would mark.
            (Icon::Check, if is_marked { accent } else { muted })
        } else if is_marked {
            (file_icon(&entry_type), accent)
        } else {
            (file_icon(&entry_type), icon_color)
        };
        ConstrainedBox::new(icon.to_warpui_icon(color).finish())
            .with_width(ICON_CELL)
            .with_height(ICON_CELL)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(SftpBrowserAction::ToggleMark(index));
    })
    .finish()
}

/// Render a single file row.
///
/// Two independent highlights, MC-style in SEMANTICS only: `is_cursor` is the
/// single keyboard cursor, `is_selected` the multi-selection set; the cursor
/// wins when a row is both, selection wins over hover. The LOOK is the app's
/// own vocabulary (polish audit FM.1) — an accent *tint* for the cursor, the
/// shared neutral overlays for selection and hover — not MC's full accent slab
/// with inverted text, which made the cursor row a foreign object in the theme.
/// A marked row carries its WHOLE line — icon, name, size, date — in the
/// accent colour: NC/MC's colour-marking made theme-native (RC acceptance
/// 2026-07-21: marks are colour, not a checkmark), so a marked set reads at a
/// glance without replacing the file-type icon.
pub fn render_file_row(
    entry: &FileEntry,
    index: usize,
    is_selected: bool,
    is_cursor: bool,
    mouse_handle: MouseStateHandle,
    mark_handle: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let is_dir = matches!(entry.file_type, FileEntryType::Directory);
    let icon_color = if is_dir {
        theme.accent().into_solid()
    } else {
        theme.sub_text_color(theme.background()).into_solid()
    };
    let text_color = if is_selected {
        theme.accent()
    } else {
        theme.active_ui_text_color()
    };
    let muted = theme.sub_text_color(theme.background());
    // Marked rows colour their WHOLE line (NC/MC), the numeric columns too.
    let sub_color = if is_selected { theme.accent() } else { muted };

    let name = entry.name.clone();
    let file_type = entry.file_type;
    let size = entry.size;
    let modified = entry.modified.clone();
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();
    let accent_fill: Fill = theme.accent().into();
    // The mark-affordance hover colour stays genuinely muted — `sub_color`
    // turns accent on marked rows and must not bleed into it.
    let muted_fill: Fill = muted.into();
    let icon_fill: Fill = icon_color.into();

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
        let size_text = if matches!(file_type, FileEntryType::Directory) {
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
            .with_child(mark_cell(
                index,
                file_type,
                is_selected,
                icon_fill,
                accent_fill,
                muted_fill,
                mark_handle.clone(),
            ))
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
    // The mark zone lives inside this row and handles its own click; deferring
    // to children is what keeps a mark-click from ALSO running SelectEntry,
    // which would clear the very selection the user just made.
    .with_defer_events_to_children()
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

/// The `..` row: the first row of any non-root directory, and the way one goes
/// up without hunting for the toolbar arrow (MC parity — the acceptance
/// complaint of 2026-07-19). It is a *virtual* row: it exists only here and in
/// the cursor arithmetic, never in `entries`, so no destructive verb can
/// address it.
///
/// A single click navigates up (RC acceptance 2026-07-21: "Klick auf `..`
/// wechselt nicht eine Ebene höher") — the row is a pure navigation
/// affordance, there is nothing else clicking it could mean. The double-click
/// handler is a deliberate no-op, NOT missing: the framework fires the click
/// handler on the first mouse-up of a double click and falls back to it on
/// the second when no double-click handler is set — without the no-op, a
/// habitual double click would navigate up TWO levels.
pub fn render_parent_row(
    is_cursor: bool,
    mouse_handle: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let accent = theme.accent();
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    Hoverable::new(mouse_handle, move |mouse| {
        let bg: Option<Fill> = if is_cursor {
            Some(theme.accent().with_opacity(22))
        } else if mouse.is_hovered() {
            Some(internal_colors::fg_overlay_1(theme))
        } else {
            None
        };

        let icon_el = ConstrainedBox::new(Icon::ArrowUp.to_warpui_icon(accent.into()).finish())
            .with_width(ICON_CELL)
            .with_height(ICON_CELL)
            .finish();

        let row_content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(ROW_SPACING)
            .with_child(icon_el)
            .with_child(
                Expanded::new(
                    1.0,
                    Text::new_inline("..".to_string(), ui_font, ui_font_size)
                        .with_color(theme.active_ui_text_color().into())
                        .finish(),
                )
                .finish(),
            )
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
        ctx.dispatch_typed_action(SftpBrowserAction::NavigateUp);
    })
    // No-op on purpose — swallows the second mouse-up of a double click so it
    // cannot navigate a second level (see the function doc).
    .on_double_click(move |_, _, _| {})
    .finish()
}

/// One sortable column header: the label, plus a caret on the column that
/// currently orders the list. Clicking sorts by it (or flips the direction).
fn header_cell(
    label: String,
    column: SortColumn,
    sort: SortState,
    handle: Option<MouseStateHandle>,
    right_aligned: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let family = appearance.ui_font_family();
    let size = appearance.ui_font_size();
    let active = sort.column == column;
    let rest_color = if active {
        theme.main_text_color(theme.background())
    } else {
        theme.sub_text_color(theme.background())
    };
    let text = if active {
        format!("{label} {}", if sort.ascending { "▲" } else { "▼" })
    } else {
        label
    };

    let Some(handle) = handle else {
        return Text::new_inline(text, family, size)
            .with_color(rest_color.into())
            .finish();
    };

    Hoverable::new(handle, move |mouse| {
        let color = if mouse.is_hovered() {
            theme.accent()
        } else {
            rest_color
        };
        let label_el = Text::new_inline(text.clone(), family, size)
            .with_color(color.into())
            .finish();
        let inner: Box<dyn Element> = if right_aligned {
            Flex::row()
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(label_el)
                .finish()
        } else {
            label_el
        };
        Container::new(inner)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(SftpBrowserAction::SortBy(column));
    })
    .finish()
}

/// Render file list header — localized, mirroring the rows' column grid
/// (leading icon cell included), separated from them by a hairline, and
/// sortable: each column header orders the list on click (MC parity).
pub fn render_header(
    sort: SortState,
    handles: &HashMap<SortColumn, MouseStateHandle>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    // The rows lead with a 16 px icon; the header mirrors that cell so "Name"
    // sits over the names (audit FM.2 — the header had no icon cell at all,
    // so "Name" sat a fixed 24 px beside the names, with a magic spacing 24
    // standing in for icon + gap between the other columns).
    let icon_spacer = ConstrainedBox::new(Flex::row().finish())
        .with_width(ICON_CELL)
        .finish();

    let name_el = Expanded::new(
        1.0,
        header_cell(
            crate::t!("fm-col-name"),
            SortColumn::Name,
            sort,
            handles.get(&SortColumn::Name).cloned(),
            false,
            appearance,
        ),
    )
    .finish();

    let size_el = ConstrainedBox::new(header_cell(
        crate::t!("fm-col-size"),
        SortColumn::Size,
        sort,
        handles.get(&SortColumn::Size).cloned(),
        true,
        appearance,
    ))
    .with_width(FILE_SIZE_WIDTH)
    .finish();

    let date_el = ConstrainedBox::new(header_cell(
        crate::t!("fm-col-modified"),
        SortColumn::Modified,
        sort,
        handles.get(&SortColumn::Modified).cloned(),
        false,
        appearance,
    ))
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

/// MC's selection status line: how much is marked. Shown only while something
/// is — an empty line would be furniture.
pub fn render_selection_status(
    count: usize,
    bytes: u64,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text = crate::t!(
        "fm-selection-status",
        count = count.to_string(),
        size = format_size(bytes)
    );
    Container::new(
        Text::new_inline(
            text,
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.accent().into())
        .finish(),
    )
    .with_padding_left(8.0)
    .with_padding_right(8.0)
    .with_padding_top(4.0)
    .with_padding_bottom(4.0)
    .with_border(Border::top(1.0).with_border_fill(theme.split_pane_border_color()))
    .finish()
}

/// Render list of file rows.
///
/// `cursor_row` indexes the *row* space: the `..` row (present when
/// `has_parent_row`) followed by the visible entries — the same space the
/// keyboard cursor moves in, so the highlight and the keys never disagree.
pub fn render_file_rows(
    entries: &[FileEntry],
    filtered_indices: &[usize],
    selected: &HashSet<usize>,
    cursor_row: usize,
    has_parent_row: bool,
    mouse_handles: &HashMap<PathBuf, MouseStateHandle>,
    mark_handles: &HashMap<PathBuf, MouseStateHandle>,
    parent_row_handle: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    if has_parent_row {
        // The `..` row's handle is a PERSISTENT one from the browser state —
        // a handle minted per render records the mouse-down in a state the
        // next render throws away, so the mouse-up never completes a click
        // (the dead `..` row of the RC acceptance 2026-07-21).
        let row = render_parent_row(cursor_row == 0, parent_row_handle, appearance);
        col.add_child(SavePosition::new(row, "sftp_row:parent").finish());
    }

    let offset = usize::from(has_parent_row);
    for (row_position, &index) in filtered_indices.iter().enumerate() {
        let entry = &entries[index];
        let is_selected = selected.contains(&index);
        let is_cursor = cursor_row == row_position + offset;
        let mouse_handle = mouse_handles.get(&entry.path).cloned().unwrap_or_default();
        let mark_handle = mark_handles.get(&entry.path).cloned().unwrap_or_default();
        let row = render_file_row(
            entry,
            index,
            is_selected,
            is_cursor,
            mouse_handle,
            mark_handle,
            appearance,
        );
        let position_id = format!("sftp_row:{index}");
        let positioned = SavePosition::new(row, &position_id).finish();
        col.add_child(positioned);
    }

    col.finish()
}
