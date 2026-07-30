//! Dialog rendering component.
//!
//! Provides rendering functionality for dialogs such as delete confirmation, rename, create folder, and file details.
//! author: logic
//! date: 2026-05-26

use std::path::PathBuf;

use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warpui::elements::{
    Border, ChildView, Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Dismiss, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement,
    Radius, SavePosition, Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::Element;
use warpui::ViewHandle;

use crate::editor::EditorView;
use crate::sftp_manager::browser::SftpBrowserAction;
use crate::sftp_manager::types::{format_size, Dialog, FileEntry, TransferDirection};

/// Maximum dialog width.
const DIALOG_MAX_WIDTH: f32 = 360.0;
/// Maximum dialog height.
const DIALOG_MAX_HEIGHT: f32 = 500.0;
/// Dialog inner padding.
const DIALOG_PADDING: f32 = 16.0;
/// Minimum button width.
const BUTTON_MIN_WIDTH: f32 = 80.0;
/// Button height.
const BUTTON_HEIGHT: f32 = 32.0;

/// Dialog shell container.
///
/// Provides uniform background color, corner radius, border, and padding.
fn dialog_shell(content: Box<dyn Element>, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    Container::new(content)
        .with_background(theme.surface_1())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.0)))
        .with_border(Border::all(1.0).with_border_fill(theme.surface_3()))
        .with_uniform_padding(DIALOG_PADDING)
        .finish()
}

/// Render title bar (title + close button).
///
/// Title is wrapped with Shrinkable for adaptive width, X close button is placed on the right.
fn render_title_bar(
    title: &str,
    appearance: &Appearance,
    close_btn_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text_color = theme.active_ui_text_color();
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    let title_el = Shrinkable::new(
        1.0,
        Text::new(title.to_string(), ui_font, ui_font_size)
            .with_color(text_color.into())
            .finish(),
    )
    .finish();

    let close_btn = render_icon_close_button(appearance, close_btn_state);

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(title_el)
        .with_child(close_btn)
        .finish()
}

/// Render X icon close button.
fn render_icon_close_button(
    appearance: &Appearance,
    mouse_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let icon_color = theme.sub_text_color(theme.background());
    let icon_el = ConstrainedBox::new(Icon::X.to_warpui_icon(icon_color).finish())
        .with_width(12.0)
        .with_height(12.0)
        .finish();
    Hoverable::new(mouse_state, move |_| {
        Container::new(icon_el)
            .with_padding_left(4.0)
            .with_padding_right(4.0)
            .with_padding_top(4.0)
            .with_padding_bottom(4.0)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(SftpBrowserAction::CloseDialog);
    })
    .finish()
}

/// Render action button component.
///
/// When is_accent is true, use accent color background; otherwise use surface_2 background.
fn render_button(
    label: &str,
    is_accent: bool,
    appearance: &Appearance,
    action: SftpBrowserAction,
    mouse_state: MouseStateHandle,
    position_id: Option<&str>,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();
    let bg = if is_accent {
        theme.accent()
    } else {
        theme.surface_2()
    };
    let text_color = if is_accent {
        theme.background()
    } else {
        theme.active_ui_text_color()
    };
    let label_owned = label.to_string();

    let btn_el = Hoverable::new(mouse_state, move |_| {
        let text_el = Text::new(label_owned.clone(), ui_font, ui_font_size)
            .with_color(text_color.into())
            .finish();
        let centered = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(text_el)
            .finish();
        Container::new(
            ConstrainedBox::new(centered)
                .with_width(BUTTON_MIN_WIDTH)
                .with_height(BUTTON_HEIGHT)
                .finish(),
        )
        .with_background(bg)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
        .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(action.clone());
    })
    .finish();

    match position_id {
        Some(id) => SavePosition::new(btn_el, id).finish(),
        None => btn_el,
    }
}

/// Render cancel button.
fn render_cancel_button(
    appearance: &Appearance,
    mouse_state: MouseStateHandle,
) -> Box<dyn Element> {
    render_button(
        &crate::t!("fm-dlg-cancel"),
        false,
        appearance,
        SftpBrowserAction::CloseDialog,
        mouse_state,
        Some("sftp_btn:dialog_cancel"),
    )
}

/// Render descriptive confirmation dialog (title + description + confirm/cancel buttons).
///
/// Applicable to scenarios such as delete confirmation, move confirmation, and overwrite confirmation.
fn render_confirm_dialog(
    title: &str,
    description: &str,
    confirm_label: &str,
    confirm_action: SftpBrowserAction,
    appearance: &Appearance,
    confirm_btn_state: MouseStateHandle,
    cancel_btn_state: MouseStateHandle,
    close_btn_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub_color = theme.sub_text_color(theme.background());
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    let title_bar = render_title_bar(title, appearance, close_btn_state);

    let desc_el = Shrinkable::new(
        1.0,
        Text::new(description.to_string(), ui_font, ui_font_size)
            .with_color(sub_color.into())
            .finish(),
    )
    .finish();

    let confirm_btn = render_button(
        confirm_label,
        true,
        appearance,
        confirm_action,
        confirm_btn_state,
        Some("sftp_btn:dialog_confirm"),
    );
    let cancel_btn = render_cancel_button(appearance, cancel_btn_state);

    let buttons = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::End)
        .with_spacing(8.0)
        .with_child(confirm_btn)
        .with_child(cancel_btn)
        .finish();

    let content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(12.0)
        .with_child(title_bar)
        .with_child(desc_el)
        .with_child(buttons)
        .finish();

    let dialog_body = ConstrainedBox::new(dialog_shell(content, appearance))
        .with_max_width(DIALOG_MAX_WIDTH)
        .with_max_height(DIALOG_MAX_HEIGHT)
        .finish();

    wrap_dismiss(dialog_body)
}

/// Render the cross-connection overwrite prompt: some destination files already
/// exist, so offer to Overwrite them all or Skip them. The title-bar X (and
/// click-outside) skips, since the non-conflicting files already started.
fn render_cross_conn_conflict(
    existing: usize,
    is_move: bool,
    appearance: &Appearance,
    confirm_btn_state: MouseStateHandle,
    cancel_btn_state: MouseStateHandle,
    close_btn_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub_color = theme.sub_text_color(theme.background());
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    let verb = if is_move {
        crate::t!("fm-verb-move")
    } else {
        crate::t!("fm-verb-copy")
    };
    let title_bar = render_title_bar(
        &crate::t!("fm-dlg-crossconn-title", verb = verb, count = existing),
        appearance,
        close_btn_state,
    );
    let desc = if existing == 1 {
        crate::t!("fm-dlg-crossconn-body-one")
    } else {
        crate::t!("fm-dlg-crossconn-body-many", count = existing)
    };
    let desc_el = Shrinkable::new(
        1.0,
        Text::new(desc, ui_font, ui_font_size)
            .with_color(sub_color.into())
            .finish(),
    )
    .finish();

    let overwrite_btn = render_button(
        &crate::t!("fm-dlg-overwrite"),
        true,
        appearance,
        SftpBrowserAction::ResolveCrossConnConflict { overwrite: true },
        confirm_btn_state,
        Some("sftp_btn:dialog_confirm"),
    );
    let skip_btn = render_button(
        &crate::t!("fm-dlg-skip"),
        false,
        appearance,
        SftpBrowserAction::ResolveCrossConnConflict { overwrite: false },
        cancel_btn_state,
        Some("sftp_btn:dialog_cancel"),
    );

    let buttons = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::End)
        .with_spacing(8.0)
        .with_child(overwrite_btn)
        .with_child(skip_btn)
        .finish();

    let content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(12.0)
        .with_child(title_bar)
        .with_child(desc_el)
        .with_child(buttons)
        .finish();

    let dialog_body = ConstrainedBox::new(dialog_shell(content, appearance))
        .with_max_width(DIALOG_MAX_WIDTH)
        .with_max_height(DIALOG_MAX_HEIGHT)
        .finish();

    wrap_dismiss(dialog_body)
}

/// Render the copy/move conflict dialog: the destination already exists, so
/// offer Overwrite / Skip and their "…all remaining" variants; the title-bar X
/// (and click-outside) cancels the whole batch.
#[allow(clippy::too_many_arguments)]
fn render_copy_move_conflict(
    name: &str,
    remaining: usize,
    is_move: bool,
    appearance: &Appearance,
    overwrite_btn_state: MouseStateHandle,
    skip_btn_state: MouseStateHandle,
    overwrite_all_btn_state: MouseStateHandle,
    skip_all_btn_state: MouseStateHandle,
    close_btn_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub_color = theme.sub_text_color(theme.background());
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    let verb = if is_move {
        crate::t!("fm-verb-move")
    } else {
        crate::t!("fm-verb-copy")
    };
    let title_bar = render_title_bar(
        &crate::t!("fm-dlg-conflict-title", verb = verb),
        appearance,
        close_btn_state,
    );

    let desc = if remaining > 1 {
        crate::t!(
            "fm-dlg-conflict-body-remaining",
            name = name,
            remaining = remaining
        )
    } else {
        crate::t!("fm-dlg-conflict-body", name = name)
    };
    let desc_el = Shrinkable::new(
        1.0,
        Text::new(desc, ui_font, ui_font_size)
            .with_color(sub_color.into())
            .finish(),
    )
    .finish();

    let overwrite_btn = render_button(
        &crate::t!("fm-dlg-overwrite"),
        true,
        appearance,
        SftpBrowserAction::OverwriteConflict { all: false },
        overwrite_btn_state,
        Some("sftp_btn:dialog_confirm"),
    );
    let skip_btn = render_button(
        &crate::t!("fm-dlg-skip"),
        false,
        appearance,
        SftpBrowserAction::SkipConflict { all: false },
        skip_btn_state,
        Some("sftp_btn:dialog_cancel"),
    );
    let overwrite_all_btn = render_button(
        &crate::t!("fm-dlg-overwrite-all"),
        false,
        appearance,
        SftpBrowserAction::OverwriteConflict { all: true },
        overwrite_all_btn_state,
        Some("sftp_btn:dialog_overwrite_all"),
    );
    let skip_all_btn = render_button(
        &crate::t!("fm-dlg-skip-all"),
        false,
        appearance,
        SftpBrowserAction::SkipConflict { all: true },
        skip_all_btn_state,
        Some("sftp_btn:dialog_skip_all"),
    );

    let buttons = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::End)
        .with_spacing(8.0)
        .with_child(overwrite_btn)
        .with_child(skip_btn)
        .with_child(overwrite_all_btn)
        .with_child(skip_all_btn)
        .finish();

    let content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(12.0)
        .with_child(title_bar)
        .with_child(desc_el)
        .with_child(buttons)
        .finish();

    let dialog_body = ConstrainedBox::new(dialog_shell(content, appearance))
        .with_max_width(DIALOG_MAX_WIDTH)
        .with_max_height(DIALOG_MAX_HEIGHT)
        .finish();

    wrap_dismiss(dialog_body)
}

/// One selectable row of the target picker: a full-width button showing a
/// candidate pane's `host:/path`, dispatching `PickCopyMoveTarget(index)`.
fn render_target_row(
    label: &str,
    index: usize,
    appearance: &Appearance,
    mouse_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();
    let text_color = theme.active_ui_text_color();
    let bg = theme.surface_2();
    let label_owned = label.to_string();

    Hoverable::new(mouse_state, move |_| {
        let text_el = Shrinkable::new(
            1.0,
            Text::new(label_owned.clone(), ui_font, ui_font_size)
                .with_color(text_color.into())
                .finish(),
        )
        .finish();
        let row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(text_el)
            .finish();
        Container::new(ConstrainedBox::new(row).with_height(BUTTON_HEIGHT).finish())
            .with_background(bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(SftpBrowserAction::PickCopyMoveTarget(index));
    })
    .finish()
}

/// Destination picker (F5/F6 with more than one other pane open): a titled
/// dialog listing every candidate pane as a full-width row; picking one routes
/// the copy/move, Cancel/X aborts. `row_btn_states` holds one mouse-state
/// handle per label, in the same order.
fn render_target_picker(
    is_move: bool,
    labels: &[String],
    appearance: &Appearance,
    cancel_btn_state: MouseStateHandle,
    close_btn_state: MouseStateHandle,
    row_btn_states: &[MouseStateHandle],
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub_color = theme.sub_text_color(theme.background());
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    let verb = if is_move {
        crate::t!("fm-verb-move")
    } else {
        crate::t!("fm-verb-copy")
    };
    let title_bar = render_title_bar(
        &crate::t!("fm-dlg-picker-title", verb = verb),
        appearance,
        close_btn_state,
    );

    let desc_el = Shrinkable::new(
        1.0,
        Text::new(crate::t!("fm-dlg-picker-body"), ui_font, ui_font_size)
            .with_color(sub_color.into())
            .finish(),
    )
    .finish();

    let mut rows = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(6.0);
    for (index, label) in labels.iter().enumerate() {
        let state = row_btn_states.get(index).cloned().unwrap_or_default();
        rows = rows.with_child(render_target_row(label, index, appearance, state));
    }
    let rows = rows.finish();

    let buttons = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::End)
        .with_spacing(8.0)
        .with_child(render_cancel_button(appearance, cancel_btn_state))
        .finish();

    let content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(12.0)
        .with_child(title_bar)
        .with_child(desc_el)
        .with_child(rows)
        .with_child(buttons)
        .finish();

    let dialog_body = ConstrainedBox::new(dialog_shell(content, appearance))
        .with_max_width(DIALOG_MAX_WIDTH)
        .with_max_height(DIALOG_MAX_HEIGHT)
        .finish();

    wrap_dismiss(dialog_body)
}

/// Wrap dialog content in Dismiss + centered container.
fn wrap_dismiss(dialog_content: Box<dyn Element>) -> Box<dyn Element> {
    Dismiss::new(dialog_content)
        .prevent_interaction_with_other_elements()
        .on_dismiss(|ctx, _| {
            ctx.dispatch_typed_action(SftpBrowserAction::CloseDialog);
        })
        .finish()
}

/// Render delete confirmation dialog.
fn render_delete_confirm(
    paths: &[PathBuf],
    appearance: &Appearance,
    confirm_btn_state: MouseStateHandle,
    cancel_btn_state: MouseStateHandle,
    close_btn_state: MouseStateHandle,
) -> Box<dyn Element> {
    let count = paths.len();
    let desc = if count == 1 {
        let name = paths[0]
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| paths[0].display().to_string());
        crate::t!("fm-dlg-delete-body-one", name = name)
    } else {
        crate::t!("fm-dlg-delete-body-many", count = count)
    };

    render_confirm_dialog(
        &crate::t!("fm-dlg-delete-title"),
        &desc,
        &crate::t!("fm-dlg-delete"),
        SftpBrowserAction::ConfirmDelete,
        appearance,
        confirm_btn_state,
        cancel_btn_state,
        close_btn_state,
    )
}

/// Render rename dialog.
fn render_rename(
    original_name: &str,
    editor: &ViewHandle<EditorView>,
    appearance: &Appearance,
    confirm_btn_state: MouseStateHandle,
    cancel_btn_state: MouseStateHandle,
    close_btn_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub_color = theme.sub_text_color(theme.background());
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    // Title bar
    let title_bar = render_title_bar(
        &crate::t!("fm-dlg-rename-title"),
        appearance,
        close_btn_state,
    );

    // Current name hint
    let hint = crate::t!("fm-dlg-rename-hint", name = original_name);
    let hint_el = Shrinkable::new(
        1.0,
        Text::new(hint, ui_font, ui_font_size)
            .with_color(sub_color.into())
            .finish(),
    )
    .finish();

    // Editor — Shrinkable + Clipped to prevent long filename overflow
    let editor_el = Container::new(
        Shrinkable::new(1.0, Clipped::new(ChildView::new(editor).finish()).finish()).finish(),
    )
    .with_padding_left(8.0)
    .with_padding_right(8.0)
    .with_padding_top(4.0)
    .with_padding_bottom(4.0)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
    .with_background(theme.surface_2())
    .finish();

    // Buttons
    let confirm_btn = render_button(
        &crate::t!("fm-dlg-confirm"),
        true,
        appearance,
        SftpBrowserAction::ConfirmRename,
        confirm_btn_state,
        Some("sftp_btn:dialog_confirm"),
    );
    let cancel_btn = render_cancel_button(appearance, cancel_btn_state);

    let buttons = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::End)
        .with_spacing(8.0)
        .with_child(confirm_btn)
        .with_child(cancel_btn)
        .finish();

    let content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(12.0)
        .with_child(title_bar)
        .with_child(hint_el)
        .with_child(editor_el)
        .with_child(buttons)
        .finish();

    let dialog_body = ConstrainedBox::new(dialog_shell(content, appearance))
        .with_max_width(DIALOG_MAX_WIDTH)
        .with_max_height(DIALOG_MAX_HEIGHT)
        .finish();

    wrap_dismiss(dialog_body)
}

/// Render create folder dialog.
fn render_create_folder(
    editor: &ViewHandle<EditorView>,
    appearance: &Appearance,
    confirm_btn_state: MouseStateHandle,
    cancel_btn_state: MouseStateHandle,
    close_btn_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    // Title bar
    let title_bar = render_title_bar(
        &crate::t!("fm-dlg-newfolder-title"),
        appearance,
        close_btn_state,
    );

    // Editor
    let editor_el = Container::new(
        Shrinkable::new(1.0, Clipped::new(ChildView::new(editor).finish()).finish()).finish(),
    )
    .with_padding_left(8.0)
    .with_padding_right(8.0)
    .with_padding_top(4.0)
    .with_padding_bottom(4.0)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
    .with_background(theme.surface_2())
    .finish();

    // Buttons
    let confirm_btn = render_button(
        &crate::t!("fm-dlg-create"),
        true,
        appearance,
        SftpBrowserAction::ConfirmNewFolder,
        confirm_btn_state,
        Some("sftp_btn:dialog_confirm"),
    );
    let cancel_btn = render_cancel_button(appearance, cancel_btn_state);

    let buttons = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::End)
        .with_spacing(8.0)
        .with_child(confirm_btn)
        .with_child(cancel_btn)
        .finish();

    let content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(12.0)
        .with_child(title_bar)
        .with_child(editor_el)
        .with_child(buttons)
        .finish();

    let dialog_body = ConstrainedBox::new(dialog_shell(content, appearance))
        .with_max_width(DIALOG_MAX_WIDTH)
        .with_max_height(DIALOG_MAX_HEIGHT)
        .finish();

    wrap_dismiss(dialog_body)
}

/// Render single property row (label + value).
fn detail_row(label: &str, value: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub_color = theme.sub_text_color(theme.background());
    let text_color = theme.active_ui_text_color();
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    let label_el = ConstrainedBox::new(
        Text::new(label.to_string(), ui_font, ui_font_size)
            .with_color(sub_color.into())
            .finish(),
    )
    .with_width(80.0)
    .finish();

    let value_el = Shrinkable::new(
        1.0,
        Text::new(value.to_string(), ui_font, ui_font_size)
            .with_color(text_color.into())
            .finish(),
    )
    .finish();

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(8.0)
        .with_child(label_el)
        .with_child(value_el)
        .finish()
}

/// Render file details dialog.
fn render_file_details(
    entry: &FileEntry,
    appearance: &Appearance,
    cancel_btn_state: MouseStateHandle,
    close_btn_state: MouseStateHandle,
) -> Box<dyn Element> {
    // Title bar
    let title_bar = render_title_bar(
        &crate::t!("fm-dlg-details-title"),
        appearance,
        close_btn_state,
    );

    // Type
    let type_str = match entry.file_type {
        crate::sftp_manager::types::FileEntryType::File => crate::t!("fm-dlg-type-file"),
        crate::sftp_manager::types::FileEntryType::Directory => crate::t!("fm-dlg-type-dir"),
        crate::sftp_manager::types::FileEntryType::Symlink => crate::t!("fm-dlg-type-symlink"),
        crate::sftp_manager::types::FileEntryType::Other => crate::t!("fm-dlg-type-other"),
    };

    // Build property rows
    let mut rows = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(8.0);

    rows.add_child(detail_row(
        &crate::t!("fm-dlg-detail-type"),
        &type_str,
        appearance,
    ));
    rows.add_child(detail_row(
        &crate::t!("fm-dlg-detail-size"),
        &format_size(entry.size),
        appearance,
    ));
    let modified = entry.modified.as_deref().unwrap_or("--");
    rows.add_child(detail_row(
        &crate::t!("fm-dlg-detail-modified"),
        modified,
        appearance,
    ));
    let permissions = entry.permissions.as_deref().unwrap_or("--");
    rows.add_child(detail_row(
        &crate::t!("fm-dlg-detail-permissions"),
        permissions,
        appearance,
    ));
    rows.add_child(detail_row(
        &crate::t!("fm-dlg-detail-path"),
        &entry.path.display().to_string(),
        appearance,
    ));

    // Close button
    let close_btn = render_cancel_button(appearance, cancel_btn_state);

    let content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(12.0)
        .with_child(title_bar)
        .with_child(
            ConstrainedBox::new(rows.finish())
                .with_max_height(250.0)
                .finish(),
        )
        .with_child(close_btn)
        .finish();

    let dialog_body = ConstrainedBox::new(dialog_shell(content, appearance))
        .with_max_width(DIALOG_MAX_WIDTH)
        .with_max_height(DIALOG_MAX_HEIGHT)
        .finish();

    wrap_dismiss(dialog_body)
}

/// Render move dialog.
fn render_move_dialog(
    source: &PathBuf,
    target_dir: &PathBuf,
    appearance: &Appearance,
    confirm_btn_state: MouseStateHandle,
    cancel_btn_state: MouseStateHandle,
    close_btn_state: MouseStateHandle,
) -> Box<dyn Element> {
    let source_name = source
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let target_display = target_dir.display().to_string();
    let desc = crate::t!(
        "fm-dlg-move-body",
        name = source_name,
        target = target_display
    );

    render_confirm_dialog(
        &crate::t!("fm-dlg-move-title"),
        &desc,
        &crate::t!("fm-verb-move"),
        SftpBrowserAction::ConfirmMove,
        appearance,
        confirm_btn_state,
        cancel_btn_state,
        close_btn_state,
    )
}

/// Render overwrite confirmation dialog.
fn render_overwrite_confirm(
    _source: &PathBuf,
    target: &PathBuf,
    _file_size: u64,
    direction: TransferDirection,
    appearance: &Appearance,
    confirm_btn_state: MouseStateHandle,
    cancel_btn_state: MouseStateHandle,
    close_btn_state: MouseStateHandle,
) -> Box<dyn Element> {
    let target_name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let desc = match direction {
        TransferDirection::Upload => crate::t!("fm-dlg-overwrite-upload-body", name = target_name),
        TransferDirection::Download => {
            crate::t!("fm-dlg-overwrite-download-body", name = target_name)
        }
        TransferDirection::Copy => {
            unreachable!("copy direction is not used by overwrite dialogs")
        }
    };

    render_confirm_dialog(
        &crate::t!("fm-dlg-overwrite-title"),
        &desc,
        &crate::t!("fm-dlg-overwrite"),
        SftpBrowserAction::ConfirmOverwrite,
        appearance,
        confirm_btn_state,
        cancel_btn_state,
        close_btn_state,
    )
}

/// Render dialog (main entry point).
///
/// Dispatch to the corresponding render function based on dialog type.
#[allow(clippy::too_many_arguments)]
pub fn render_dialog(
    dialog: &Dialog,
    rename_editor: &ViewHandle<EditorView>,
    new_folder_editor: &ViewHandle<EditorView>,
    appearance: &Appearance,
    confirm_btn_state: MouseStateHandle,
    cancel_btn_state: MouseStateHandle,
    close_btn_state: MouseStateHandle,
    overwrite_all_btn_state: MouseStateHandle,
    skip_all_btn_state: MouseStateHandle,
    target_pick_btn_states: &[MouseStateHandle],
) -> Box<dyn Element> {
    match dialog {
        Dialog::CopyMoveTargetPicker { is_move, labels } => render_target_picker(
            *is_move,
            labels,
            appearance,
            cancel_btn_state,
            close_btn_state,
            target_pick_btn_states,
        ),
        Dialog::CopyMoveConflict {
            name,
            remaining,
            is_move,
        } => render_copy_move_conflict(
            name,
            *remaining,
            *is_move,
            appearance,
            confirm_btn_state,
            cancel_btn_state,
            overwrite_all_btn_state,
            skip_all_btn_state,
            close_btn_state,
        ),
        Dialog::CrossConnConflict { existing, is_move } => render_cross_conn_conflict(
            *existing,
            *is_move,
            appearance,
            confirm_btn_state,
            cancel_btn_state,
            close_btn_state,
        ),
        Dialog::DeleteConfirm { paths, .. } => render_delete_confirm(
            paths,
            appearance,
            confirm_btn_state,
            cancel_btn_state,
            close_btn_state,
        ),
        Dialog::Rename { original_name, .. } => render_rename(
            original_name,
            rename_editor,
            appearance,
            confirm_btn_state,
            cancel_btn_state,
            close_btn_state,
        ),
        Dialog::CreateFolder { .. } => render_create_folder(
            new_folder_editor,
            appearance,
            confirm_btn_state,
            cancel_btn_state,
            close_btn_state,
        ),
        Dialog::FileDetails { entry } => {
            render_file_details(entry, appearance, cancel_btn_state, close_btn_state)
        }
        Dialog::Move { source, target_dir } => render_move_dialog(
            source,
            target_dir,
            appearance,
            confirm_btn_state,
            cancel_btn_state,
            close_btn_state,
        ),
        Dialog::OverwriteConfirm {
            source,
            target,
            file_size,
            direction,
        } => render_overwrite_confirm(
            source,
            target,
            *file_size,
            *direction,
            appearance,
            confirm_btn_state,
            cancel_btn_state,
            close_btn_state,
        ),
        Dialog::CloseTransferPanelConfirm => render_confirm_dialog(
            &crate::t!("fm-dlg-close-transfer-title"),
            &crate::t!("fm-dlg-close-transfer-body"),
            &crate::t!("fm-dlg-close"),
            SftpBrowserAction::ConfirmCloseTransferPanel,
            appearance,
            confirm_btn_state,
            cancel_btn_state,
            close_btn_state,
        ),
    }
}
