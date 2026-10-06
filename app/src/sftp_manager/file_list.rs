//! File list rendering component
//!
//! Provides rendering functionality for file list headers and file rows.
//! author: logic
//! date: 2026-05-26

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::{Fill, WarpTheme};
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Expanded,
    Fill as ElementFill, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Point, Radius, SavePosition, Text, ZIndex,
};
use warpui::event::{DispatchedEvent, ModifiersState};
use warpui::fonts::{Properties, Weight};
use warpui::geometry::vector::Vector2F;
use warpui::platform::Cursor;
use warpui::{
    AfterLayoutContext, AppContext, Element, Event, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

use crate::sftp_manager::browser::{SftpBrowserAction, SortColumn, SortState};
use crate::sftp_manager::types::{
    format_size, EntryIdentity, EntryReference, FileEntry, FileEntryType,
};
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
/// Width of the cursor outline. Every row lays this border out (transparent
/// unless it is the cursor row) and pads 1 px less, so rows keep the header's
/// 8 × 4 px content inset and moving the cursor never moves any geometry.
const ROW_OUTLINE: f32 = 1.0;
/// Horizontal / vertical row padding inside the outline.
const ROW_PADDING_X: f32 = 8.0 - ROW_OUTLINE;
const ROW_PADDING_Y: f32 = 4.0 - ROW_OUTLINE;

/// Return appropriate icon based on file entry type
pub fn file_icon(entry_type: &FileEntryType) -> Icon {
    match entry_type {
        FileEntryType::Directory => Icon::Folder,
        FileEntryType::Symlink => Icon::LinkHorizontal,
        FileEntryType::File | FileEntryType::Other => Icon::File,
    }
}

/// How one list row looks for a given state.
///
/// Mark and cursor are independent channels so both stay readable when they
/// meet on one row: the **mark** owns the text (accent-tinted, bold name) and
/// an accent tint behind the row, the **cursor** owns a 1 px outline (accent
/// in the focused pane) plus a neutral fill when the row is not marked. An
/// inactive pane keeps its marks — on a lighter tint — and shows its cursor
/// as a neutral outline only. Hover never changes anything but the fill of an
/// otherwise plain row (spec A4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RowLook {
    /// Row fill; `None` leaves the pane background.
    pub background: Option<Fill>,
    /// Cursor outline colour; `None` keeps the always-present border clear.
    pub outline: Option<Fill>,
    /// Colour of the name.
    pub name: Fill,
    /// Colour of the size and date columns.
    pub detail: Fill,
    /// Whether the name is set in bold (marked rows).
    pub bold: bool,
}

/// Resolve [`RowLook`] from theme roles only (spec A6): marked rows use the
/// accent-tinted foreground for text and the accent overlays for fill, the
/// cursor uses the accent (focused) or a neutral overlay (inactive pane).
pub(crate) fn row_look(
    theme: &WarpTheme,
    is_marked: bool,
    is_cursor: bool,
    pane_focused: bool,
    is_hovered: bool,
) -> RowLook {
    let background = if is_marked {
        Some(if pane_focused {
            internal_colors::accent_overlay_2(theme)
        } else {
            internal_colors::accent_overlay_1(theme)
        })
    } else if is_cursor && pane_focused {
        Some(internal_colors::fg_overlay_2(theme))
    } else if is_hovered {
        Some(internal_colors::fg_overlay_1(theme))
    } else {
        None
    };
    let outline = is_cursor.then(|| {
        if pane_focused {
            theme.accent()
        } else {
            internal_colors::fg_overlay_3(theme)
        }
    });
    let marked_text = internal_colors::accent_fg_strong(theme);
    RowLook {
        background,
        outline,
        name: if is_marked {
            marked_text
        } else {
            theme.active_ui_text_color()
        },
        detail: if is_marked {
            marked_text
        } else {
            theme.sub_text_color(theme.background())
        },
        bold: is_marked,
    }
}

/// The row container: content inset inside an always-present 1 px border
/// that only the cursor colours.
fn row_container(content: Box<dyn Element>, look: &RowLook) -> Box<dyn Element> {
    let outline: ElementFill = match look.outline {
        Some(fill) => fill.into(),
        None => ElementFill::None,
    };
    let mut container = Container::new(content)
        .with_padding_left(ROW_PADDING_X)
        .with_padding_right(ROW_PADDING_X)
        .with_padding_top(ROW_PADDING_Y)
        .with_padding_bottom(ROW_PADDING_Y)
        .with_border(Border::all(ROW_OUTLINE).with_border_fill(outline));
    if let Some(background) = look.background {
        container = container.with_background(background);
    }
    container.finish()
}

/// What a left press with modifiers means on a list row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModifiedClick {
    /// Cmd-click (macOS) or Ctrl-click: toggle this row's mark. (macOS turns
    /// Ctrl-click into a right click before it gets here.)
    Toggle,
    /// Shift-click: mark the range from the keyboard cursor to this row.
    Range,
}

/// Classify a left press by its modifiers; a plain press (`None`) is left to
/// the row's ordinary click handling (cursor move / open on double click).
pub(crate) fn modified_click(modifiers: &ModifiersState) -> Option<ModifiedClick> {
    if modifiers.alt {
        None
    } else if modifiers.cmd || modifiers.ctrl {
        Some(ModifiedClick::Toggle)
    } else if modifiers.shift {
        Some(ModifiedClick::Range)
    } else {
        None
    }
}

/// Routes Cmd/Ctrl- and Shift-presses on a row to the marking actions before
/// the row's own click handling sees them.
///
/// `Hoverable` click callbacks carry no modifier state, but the raw
/// `LeftMouseDown` does. This wrapper sits *inside* the row's `Hoverable`
/// (which defers to its children), reads the press, dispatches the action and
/// consumes the event: the row then never arms a click, so the same gesture
/// cannot also move the cursor or open the entry on its mouse-up. Plain
/// presses pass through untouched. A row that is not markable (`..`) passes
/// `None` for both actions and simply swallows modified presses.
struct ModifiedClickTarget {
    child: Box<dyn Element>,
    toggle: Option<SftpBrowserAction>,
    range: Option<SftpBrowserAction>,
    origin: Option<Point>,
    child_max_z_index: Option<ZIndex>,
}

impl ModifiedClickTarget {
    fn new(
        child: Box<dyn Element>,
        toggle: Option<SftpBrowserAction>,
        range: Option<SftpBrowserAction>,
    ) -> Self {
        Self {
            child,
            toggle,
            range,
            origin: None,
            child_max_z_index: None,
        }
    }

    /// Whether `position` hits the visible, uncovered part of this row.
    fn is_over(&self, position: Vector2F, ctx: &EventContext) -> bool {
        let (Some(origin), Some(size), Some(z_index)) =
            (self.origin, self.child.size(), self.child_max_z_index)
        else {
            return false;
        };
        ctx.visible_rect(origin, size)
            .is_some_and(|rect| rect.contains_point(position))
            && !ctx.is_covered(Point::from_vec2f(position, z_index))
    }
}

impl Element for ModifiedClickTarget {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        self.child.paint(origin, ctx, app);
        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn parent_data(&self) -> Option<&dyn Any> {
        self.child.parent_data()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if let Event::LeftMouseDown {
            position,
            modifiers,
            ..
        } = event.raw_event()
        {
            if let Some(click) = modified_click(modifiers) {
                if self.is_over(*position, ctx) {
                    let action = match click {
                        ModifiedClick::Toggle => self.toggle.clone(),
                        ModifiedClick::Range => self.range.clone(),
                    };
                    if let Some(action) = action {
                        ctx.dispatch_typed_action(action);
                    }
                    return true;
                }
            }
        }
        self.child.dispatch_event(event, ctx, app)
    }
}

/// The leading cell of a row: the type icon at rest, and the **mark zone** on
/// approach — hovering swaps in a check outline, clicking toggles the
/// multi-selection. Mouse parity for MC's Insert/Space for anyone who does not
/// reach for Cmd/Ctrl-click (see [`ModifiedClickTarget`]).
///
/// A *marked* row keeps its file-type icon — recoloured in the accent, like
/// the rest of its line (NC/MC's colour-marking; RC acceptance 2026-07-21:
/// marks are colour, not a checkmark). The check glyph appears only under the
/// pointer, as the toggle affordance.
fn mark_cell(
    entry: EntryReference,
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
        ctx.dispatch_typed_action(SftpBrowserAction::ToggleMark(entry.clone()));
    })
    .finish()
}

/// Render a single file row.
///
/// Two independent highlights, MC-style in semantics: `is_cursor` is the
/// single keyboard cursor, `is_selected` the multi-selection set (marks), and
/// both stay visible when they meet (see [`RowLook`]). A marked row carries
/// its whole line — icon, bold name, size, date — in the accent colour on an
/// accent tint: NC/MC's colour-marking made theme-native (RC acceptance
/// 2026-07-21: marks are colour, not a checkmark), so a marked set of files
/// and folders reads at a glance without replacing the file-type icon.
///
/// Mouse: a plain click moves the cursor, a double click opens, Cmd/Ctrl-click
/// toggles the mark, Shift-click marks the range from the cursor.
#[allow(clippy::too_many_arguments)]
pub fn render_file_row(
    entry: &FileEntry,
    index: usize,
    listing_generation: u64,
    position_prefix: &str,
    panel_position_id: &str,
    is_selected: bool,
    is_cursor: bool,
    pane_focused: bool,
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
    let muted = theme.sub_text_color(theme.background());

    let name = entry.name.clone();
    let file_type = entry.file_type;
    let entry_reference = entry.entry_reference(listing_generation);
    let mark_reference = entry_reference.clone();
    let open_reference = entry_reference.clone();
    let context_reference = entry_reference.clone();
    let toggle_reference = entry_reference.clone();
    let range_reference = entry_reference.clone();
    let size = entry.size;
    let modified = entry.modified.clone();
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();
    let accent_fill: Fill = theme.accent();
    let row_position_prefix = position_prefix.to_string();
    let panel_position_id = panel_position_id.to_string();
    // The mark-affordance hover colour stays genuinely muted — the detail
    // colour turns accent on marked rows and must not bleed into it.
    let muted_fill: Fill = muted;
    let icon_fill: Fill = icon_color.into();

    Hoverable::new(mouse_handle, move |mouse| {
        let look = row_look(
            theme,
            is_selected,
            is_cursor,
            pane_focused,
            mouse.is_hovered(),
        );

        // Name — `Expanded` (tight fit), not `Shrinkable` (loose fit): only a
        // tight-fit child actually consumes the remaining width, and consuming
        // it is what pins Size/Modified into stable right columns.
        let mut name_text =
            Text::new_inline(name.clone(), ui_font, ui_font_size).with_color(look.name.into());
        if look.bold {
            name_text = name_text.with_style(Properties::default().weight(Weight::Bold));
        }
        let name_el = Expanded::new(1.0, name_text.finish()).finish();

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
                        .with_color(look.detail.into())
                        .finish(),
                )
                .finish(),
        )
        .with_width(FILE_SIZE_WIDTH)
        .finish();
        let size_position_id = format!("{row_position_prefix}:{index}:size-column");
        let size_el = SavePosition::new(size_el, &size_position_id)
            .for_single_frame()
            .finish();

        // Modified date
        let date_text = modified.clone().unwrap_or_else(|| String::from("--"));
        let date_el = ConstrainedBox::new(
            Text::new_inline(date_text, ui_font, ui_font_size)
                .with_color(look.detail.into())
                .finish(),
        )
        .with_width(FILE_DATE_WIDTH)
        .finish();
        let date_position_id = format!("{row_position_prefix}:{index}:date-column");
        let date_el = SavePosition::new(date_el, &date_position_id)
            .for_single_frame()
            .finish();

        // Assemble row content. `Max` is what pins Size/Modified into stable
        // columns — without it the row hugged its content and the columns
        // wandered with the name length (polish audit FM.2).
        let row_content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(ROW_SPACING)
            .with_child(mark_cell(
                mark_reference.clone(),
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

        ModifiedClickTarget::new(
            row_container(row_content, &look),
            Some(SftpBrowserAction::ToggleMark(toggle_reference.clone())),
            Some(SftpBrowserAction::MarkRangeTo(range_reference.clone())),
        )
        .finish()
    })
    // The mark zone and the modifier-click target live inside this row and
    // handle their own presses; deferring to children is what keeps a mark
    // gesture from ALSO running SelectEntry (a cursor move) on mouse-up.
    .with_defer_events_to_children()
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(SftpBrowserAction::SelectEntry(entry_reference.clone()));
    })
    .on_double_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(SftpBrowserAction::OpenEntry(open_reference.clone()));
    })
    .on_right_click(move |ctx, _, position| {
        let offset = match ctx.element_position_by_id(&panel_position_id) {
            Some(bounds) => position - bounds.origin(),
            None => position,
        };
        ctx.dispatch_typed_action(SftpBrowserAction::ContextMenu {
            entry: context_reference.clone(),
            position: offset,
        });
    })
    .finish()
}

/// The `..` row: the first row of any non-root directory, and the way one goes
/// up without hunting for the toolbar arrow (MC parity — the acceptance
/// complaint of 2026-07-19). It is a *virtual* row: it exists only here and in
/// the cursor arithmetic, never in `entries`, so no destructive verb can
/// address it — and no marking gesture either: it is never marked, and a
/// Cmd/Ctrl- or Shift-click on it does nothing.
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
    pane_focused: bool,
    mouse_handle: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let accent = theme.accent();
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    Hoverable::new(mouse_handle, move |mouse| {
        let look = row_look(theme, false, is_cursor, pane_focused, mouse.is_hovered());

        let icon_el = ConstrainedBox::new(Icon::ArrowUp.to_warpui_icon(accent).finish())
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
                        .with_color(look.name.into())
                        .finish(),
                )
                .finish(),
            )
            .finish();

        ModifiedClickTarget::new(row_container(row_content, &look), None, None).finish()
    })
    .with_defer_events_to_children()
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

/// "3 Ordner · 5 Dateien", "1 Datei", "2 Ordner": the marked set by kind.
/// Folders are named apart from files because an operation on a folder
/// includes everything inside it.
pub(crate) fn marked_items_label(folders: usize, files: usize) -> String {
    let folder_count = crate::t!("fm-count-folders", count = folders);
    let file_count = crate::t!("fm-count-files", count = files);
    match (folders, files) {
        (0, _) => file_count,
        (_, 0) => folder_count,
        _ => crate::t!(
            "fm-count-folders-and-files",
            folders = folder_count,
            files = file_count
        ),
    }
}

/// The words of MC's selection status line, e.g. "3 Ordner · 5 Dateien
/// markiert · 12.4 MB ohne Ordner". Folder sizes are never computed (that
/// would walk every marked tree), so the byte total covers the marked files
/// and says so whenever folders are marked too.
pub(crate) fn selection_status_text(folders: usize, files: usize, file_bytes: u64) -> String {
    let items = marked_items_label(folders, files);
    match (folders, files) {
        (0, _) => crate::t!(
            "fm-selection-status",
            items = items,
            size = format_size(file_bytes)
        ),
        (_, 0) => crate::t!("fm-selection-status-folders-only", items = items),
        _ => crate::t!(
            "fm-selection-status-with-folders",
            items = items,
            size = format_size(file_bytes)
        ),
    }
}

/// MC's selection status line: what is marked, by kind, and how big. Shown
/// only while something is — an empty line would be furniture. An inactive
/// pane keeps the line, in the secondary text colour.
pub fn render_selection_status(
    folders: usize,
    files: usize,
    file_bytes: u64,
    pane_focused: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let color = if pane_focused {
        internal_colors::accent_fg_strong(theme)
    } else {
        theme.sub_text_color(theme.background())
    };
    Container::new(
        Text::new(
            selection_status_text(folders, files, file_bytes),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(color.into())
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
#[allow(clippy::too_many_arguments)]
pub fn render_file_rows(
    entries: &[FileEntry],
    filtered_indices: &[usize],
    selected: &HashSet<EntryIdentity>,
    listing_generation: u64,
    cursor_row: usize,
    has_parent_row: bool,
    position_prefix: &str,
    panel_position_id: &str,
    mouse_handles: &HashMap<PathBuf, MouseStateHandle>,
    mark_handles: &HashMap<PathBuf, MouseStateHandle>,
    parent_row_handle: MouseStateHandle,
    pane_focused: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    if has_parent_row {
        // The `..` row's handle is a PERSISTENT one from the browser state —
        // a handle minted per render records the mouse-down in a state the
        // next render throws away, so the mouse-up never completes a click
        // (the dead `..` row of the RC acceptance 2026-07-21).
        let row = render_parent_row(cursor_row == 0, pane_focused, parent_row_handle, appearance);
        let position_id = format!("{position_prefix}:parent");
        col.add_child(
            SavePosition::new(row, &position_id)
                .for_single_frame()
                .finish(),
        );
    }

    let offset = usize::from(has_parent_row);
    for (row_position, &index) in filtered_indices.iter().enumerate() {
        let entry = &entries[index];
        let is_selected = selected.contains(&entry.entry_identity());
        let is_cursor = cursor_row == row_position + offset;
        let mouse_handle = mouse_handles.get(&entry.path).cloned().unwrap_or_default();
        let mark_handle = mark_handles.get(&entry.path).cloned().unwrap_or_default();
        let row = render_file_row(
            entry,
            index,
            listing_generation,
            position_prefix,
            panel_position_id,
            is_selected,
            is_cursor,
            pane_focused,
            mouse_handle,
            mark_handle,
            appearance,
        );
        let position_id = format!("{position_prefix}:{index}");
        let positioned = SavePosition::new(row, &position_id)
            .for_single_frame()
            .finish();
        col.add_child(positioned);
    }

    col.finish()
}
