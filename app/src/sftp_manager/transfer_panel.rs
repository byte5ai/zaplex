//! Transfer panel rendering component
//!
//! Provides rendering for file transfer progress panel, including transfer direction icon, status label, progress bar, and transfer list.
//! author: logic
//! date: 2026-05-26

use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Fill, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, SavePosition, ScrollbarWidth, Shrinkable, SizeConstraintCondition,
    SizeConstraintSwitch, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, SingletonEntity};

use crate::sftp_manager::browser::SftpBrowserAction;
use crate::sftp_manager::types::{TransferDirection, TransferPhase, TransferState, TransferTask};
use crate::ui_components::icons::Icon;

/// Progress bar height
const PROGRESS_BAR_HEIGHT: f32 = 4.0;
/// Panel inner padding
const PANEL_PADDING: f32 = 8.0;

fn completed_warning_label(warning: &str) -> String {
    if warning == super::transfer_queue::SOURCE_RESTORED_WARNING {
        crate::t!("fm-transfer-completed-source-restored").to_string()
    } else {
        crate::t!("fm-transfer-completed-cleanup-required").to_string()
    }
}

#[derive(Clone, Copy)]
pub enum TransferPanelTarget {
    Browser,
    Workspace,
}

pub(super) fn transfer_state_from_queue_state(
    state: super::transfer_queue::QueuedTransferState,
) -> TransferState {
    match state {
        super::transfer_queue::QueuedTransferState::Running => TransferState::InProgress,
        super::transfer_queue::QueuedTransferState::Paused => TransferState::Paused,
        super::transfer_queue::QueuedTransferState::Cancelling => TransferState::Cancelling,
        super::transfer_queue::QueuedTransferState::Completed => TransferState::Completed,
        super::transfer_queue::QueuedTransferState::PartiallyCompleted {
            transferred,
            published,
            skipped,
            source_kept,
        } => TransferState::PartiallyCompleted {
            transferred,
            published,
            skipped,
            source_kept,
        },
        super::transfer_queue::QueuedTransferState::CompletedWithWarning(error) => {
            TransferState::CompletedWithWarning(error)
        }
        super::transfer_queue::QueuedTransferState::Failed(error) => TransferState::Failed(error),
        super::transfer_queue::QueuedTransferState::Cancelled => TransferState::Cancelled,
        super::transfer_queue::QueuedTransferState::Skipped => TransferState::Skipped,
    }
}

/// Render transfer direction icon
fn render_direction_icon(
    direction: &TransferDirection,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let icon_color = theme.sub_text_color(theme.background());

    let icon = match direction {
        TransferDirection::Upload => Icon::UploadCloud,
        TransferDirection::Download => Icon::Download,
        TransferDirection::Copy => Icon::Copy,
    };

    ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
        .with_width(14.0)
        .with_height(14.0)
        .finish()
}

/// Render transfer state label
fn render_state_label(
    state: &TransferState,
    phase: TransferPhase,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    let (label, color) = match state {
        TransferState::Pending => (
            crate::t!("fm-transfer-waiting").to_string(),
            theme.sub_text_color(theme.background()),
        ),
        TransferState::InProgress => match phase {
            TransferPhase::Transferring => (
                crate::t!("fm-transfer-inprogress").to_string(),
                theme.accent(),
            ),
            TransferPhase::Verifying => (
                crate::t!("fm-transfer-verifying").to_string(),
                theme.accent(),
            ),
            TransferPhase::Finalizing => (
                crate::t!("fm-transfer-finalizing").to_string(),
                theme.accent(),
            ),
        },
        TransferState::Paused => (
            crate::t!("fm-transfer-paused").to_string(),
            theme.sub_text_color(theme.background()),
        ),
        TransferState::Cancelling => (
            crate::t!("fm-transfer-cancelling").to_string(),
            theme.sub_text_color(theme.background()),
        ),
        TransferState::Completed => (
            crate::t!("fm-transfer-completed").to_string(),
            theme.ui_green_color().into(),
        ),
        TransferState::Skipped => (
            crate::t!("fm-transfer-skipped").to_string(),
            theme.sub_text_color(theme.background()),
        ),
        TransferState::PartiallyCompleted {
            transferred,
            published,
            skipped,
            source_kept,
        } => (
            partial_state_label(*transferred, *published, *skipped, *source_kept),
            theme.ui_warning_color().into(),
        ),
        TransferState::CompletedWithWarning(warning) => (
            completed_warning_label(warning),
            theme.ui_warning_color().into(),
        ),
        TransferState::Failed(_) => (
            crate::t!("fm-transfer-failed").to_string(),
            theme.ui_error_color().into(),
        ),
        TransferState::Cancelled => (
            crate::t!("fm-transfer-cancelled").to_string(),
            theme.sub_text_color(theme.background()),
        ),
    };

    Text::new_inline(label, ui_font, ui_font_size)
        .with_color(color.into())
        .finish()
}

fn partial_state_label(
    transferred: usize,
    published: usize,
    skipped: usize,
    source_kept: bool,
) -> String {
    if source_kept {
        crate::t!(
            "fm-transfer-partial-source-kept",
            transferred = transferred.to_string(),
            published = published.to_string(),
            skipped = skipped.to_string()
        )
    } else {
        crate::t!(
            "fm-transfer-partial",
            transferred = transferred.to_string(),
            published = published.to_string(),
            skipped = skipped.to_string()
        )
    }
}

/// Render progress bar
fn render_progress_bar(progress: u8, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();

    if progress == 0 {
        return ConstrainedBox::new(
            Container::new(Flex::row().finish())
                .with_background(theme.surface_3())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.0)))
                .finish(),
        )
        .with_height(PROGRESS_BAR_HEIGHT)
        .finish();
    }

    let remaining = 100u8.saturating_sub(progress);

    // Progress fill
    let fill = ConstrainedBox::new(
        Container::new(Flex::row().finish())
            .with_background(theme.accent())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.0)))
            .finish(),
    )
    .with_height(PROGRESS_BAR_HEIGHT)
    .finish();

    // Empty space
    let spacer = Shrinkable::new(
        remaining as f32,
        ConstrainedBox::new(Flex::row().finish())
            .with_height(PROGRESS_BAR_HEIGHT)
            .finish(),
    )
    .finish();

    ConstrainedBox::new(
        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(progress as f32, fill).finish())
                .with_child(spacer)
                .finish(),
        )
        .with_background(theme.surface_3())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.0)))
        .finish(),
    )
    .with_height(PROGRESS_BAR_HEIGHT)
    .finish()
}

/// Render single transfer row
fn render_transfer_row(
    task: &TransferTask,
    cancel_btn_state: Option<MouseStateHandle>,
    pause_btn_state: Option<MouseStateHandle>,
    retry_btn_state: Option<MouseStateHandle>,
    appearance: &Appearance,
    target: TransferPanelTarget,
) -> Box<dyn Element> {
    // Direction icon
    let dir_icon = render_direction_icon(&task.direction, appearance);

    // Filename
    let file_name = task
        .source_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let name_el = Text::new_inline(
        file_name,
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(appearance.theme().active_ui_text_color().into())
    .finish();

    // State label
    let state_el = render_state_label(&task.state, task.phase, appearance);

    // First row: icon + filename + status + cancel button
    let mut top_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(6.0)
        .with_child(dir_icon)
        .with_child(Shrinkable::new(1.0, name_el).finish())
        .with_child(state_el);

    if let (TransferState::InProgress | TransferState::Paused, false, Some(cancel_btn_state)) = (
        &task.state,
        task.phase == TransferPhase::Finalizing,
        cancel_btn_state,
    ) {
        let task_id = task.id;
        let control_epoch = task.control_epoch;
        let icon_color = appearance
            .theme()
            .sub_text_color(appearance.theme().background());
        let position_id = format!("sftp_btn:cancel_transfer:{task_id}");

        let cancel_el = Hoverable::new(cancel_btn_state, move |_| {
            let icon_el = ConstrainedBox::new(Icon::X.to_warpui_icon(icon_color).finish())
                .with_width(12.0)
                .with_height(12.0)
                .finish();
            Container::new(icon_el).with_uniform_padding(2.0).finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| match target {
            TransferPanelTarget::Browser => {
                ctx.dispatch_typed_action(SftpBrowserAction::CancelTransfer(
                    task_id,
                    control_epoch,
                ));
            }
            TransferPanelTarget::Workspace => {
                if let Some(control_epoch) = control_epoch {
                    ctx.dispatch_typed_action(crate::WorkspaceAction::CancelTransfer(
                        super::transfer_queue::TransferActionTarget {
                            id: task_id as u64,
                            control_epoch,
                        },
                    ));
                }
            }
        })
        .finish();

        let hit_target = ConstrainedBox::new(cancel_el)
            .with_width(16.0)
            .with_height(16.0)
            .finish();
        let positioned = SavePosition::new(hit_target, &position_id).finish();
        top_row = top_row.with_child(positioned);
    }
    if let (TransferState::InProgress | TransferState::Paused, false, Some(pause_btn_state)) = (
        &task.state,
        task.phase == TransferPhase::Finalizing,
        pause_btn_state,
    ) {
        let task_id = task.id;
        let control_epoch = task.control_epoch;
        let paused = matches!(task.state, TransferState::Paused);
        let label = if paused {
            crate::t!("fm-transfer-resume").to_string()
        } else {
            crate::t!("fm-transfer-pause").to_string()
        };
        let color = appearance
            .theme()
            .sub_text_color(appearance.theme().background());
        let font = appearance.ui_font_family();
        let size = appearance.ui_font_size();
        let position_id = format!("sftp_btn:pause_transfer:{task_id}");
        let pause = Hoverable::new(pause_btn_state, move |_| {
            Text::new_inline(label.clone(), font, size)
                .with_color(color.into())
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| match target {
            TransferPanelTarget::Browser => {
                if paused {
                    ctx.dispatch_typed_action(SftpBrowserAction::ResumeTransfer(
                        task_id,
                        control_epoch,
                    ));
                } else {
                    ctx.dispatch_typed_action(SftpBrowserAction::PauseTransfer(
                        task_id,
                        control_epoch,
                    ));
                }
            }
            TransferPanelTarget::Workspace => {
                if let Some(control_epoch) = control_epoch {
                    let target = super::transfer_queue::TransferActionTarget {
                        id: task_id as u64,
                        control_epoch,
                    };
                    let action = if paused {
                        crate::WorkspaceAction::ResumeTransfer(target)
                    } else {
                        crate::WorkspaceAction::PauseTransfer(target)
                    };
                    ctx.dispatch_typed_action(action);
                }
            }
        })
        .finish();
        top_row = top_row.with_child(SavePosition::new(pause, &position_id).finish());
    }
    if let (
        TransferState::Failed(_) | TransferState::CompletedWithWarning(_),
        true,
        Some(retry_btn_state),
    ) = (&task.state, task.recovery_retryable, retry_btn_state)
    {
        let task_id = task.id;
        let color = appearance
            .theme()
            .sub_text_color(appearance.theme().background());
        let font = appearance.ui_font_family();
        let size = appearance.ui_font_size();
        let position_id = format!("sftp_btn:retry_transfer_recovery:{task_id}");
        let retry = Hoverable::new(retry_btn_state, move |_| {
            Text::new_inline(
                crate::t!("fm-transfer-retry-cleanup").to_string(),
                font,
                size,
            )
            .with_color(color.into())
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| match target {
            TransferPanelTarget::Browser => {
                ctx.dispatch_typed_action(SftpBrowserAction::RetryTransferRecovery(task_id));
            }
            TransferPanelTarget::Workspace => {
                ctx.dispatch_typed_action(crate::WorkspaceAction::RetryTransferRecovery(
                    task_id as u64,
                ));
            }
        })
        .finish();
        top_row = top_row.with_child(SavePosition::new(retry, &position_id).finish());
    }

    let mut col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(4.0)
        .with_child(top_row.finish());

    // Progress bar (only shown when in progress)
    if matches!(
        task.state,
        TransferState::InProgress | TransferState::Paused | TransferState::Cancelling
    ) {
        let bar = render_progress_bar(task.progress_percent(), appearance);
        col.add_child(bar);
        if let Some(eta) = task.eta {
            let eta_position_id = format!("sftp_text:transfer_eta:{}", task.id);
            let eta_text = Text::new_inline(
                crate::t!("fm-transfer-eta", seconds = eta.as_secs()),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().background())
                    .into(),
            )
            .finish();
            col.add_child(SavePosition::new(eta_text, &eta_position_id).finish());
        }
    }
    for path in &task.recovery_paths {
        let recovery_text = Text::new_inline(
            crate::t!(
                "fm-transfer-recovery-path",
                path = path.to_string_lossy().to_string()
            ),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(appearance.theme().ui_error_color().into())
        .finish();
        col.add_child(recovery_text);
    }

    Container::new(col.finish())
        .with_padding_top(4.0)
        .with_padding_bottom(4.0)
        .finish()
}

/// Render file transfer panel (main entry point)
///
/// Always display the transfer task list, with close button on the right of the title bar.
pub fn render_transfer_panel(
    transfers: &[TransferTask],
    cancel_btn_states: &std::collections::HashMap<usize, MouseStateHandle>,
    pause_btn_states: &std::collections::HashMap<usize, MouseStateHandle>,
    retry_btn_states: &std::collections::HashMap<usize, MouseStateHandle>,
    appearance: &Appearance,
    close_btn_state: MouseStateHandle,
    scroll_state: ClippedScrollStateHandle,
    target: TransferPanelTarget,
) -> Box<dyn Element> {
    SizeConstraintSwitch::new(
        render_transfer_panel_at_height(
            transfers,
            cancel_btn_states,
            pause_btn_states,
            retry_btn_states,
            appearance,
            close_btn_state.clone(),
            scroll_state.clone(),
            320.0,
            target,
        ),
        vec![
            (
                SizeConstraintCondition::HeightLessThan(360.0),
                render_transfer_panel_at_height(
                    transfers,
                    cancel_btn_states,
                    pause_btn_states,
                    retry_btn_states,
                    appearance,
                    close_btn_state.clone(),
                    scroll_state.clone(),
                    120.0,
                    target,
                ),
            ),
            (
                SizeConstraintCondition::HeightLessThan(640.0),
                render_transfer_panel_at_height(
                    transfers,
                    cancel_btn_states,
                    pause_btn_states,
                    retry_btn_states,
                    appearance,
                    close_btn_state,
                    scroll_state,
                    220.0,
                    target,
                ),
            ),
        ],
    )
    .finish()
}

pub fn render_workspace_transfer_panel(app: &AppContext) -> Option<Box<dyn Element>> {
    let queue = super::transfer_queue::TransferQueue::as_ref(app);
    let mut transfers = Vec::new();
    let mut cancel_handles = std::collections::HashMap::new();
    let mut pause_handles = std::collections::HashMap::new();
    let mut retry_handles = std::collections::HashMap::new();
    for activity in queue.activities() {
        let Ok(task_id) = usize::try_from(activity.id) else {
            continue;
        };
        let mut task = TransferTask::new(
            task_id,
            activity.source_path,
            activity.target_path,
            activity.direction,
            activity.progress.total,
        );
        task.transferred = activity.progress.transferred;
        task.control_epoch = Some(activity.control_epoch);
        task.eta = activity.progress.eta;
        task.phase = activity.progress.phase;
        task.recovery_paths = activity.recovery_paths;
        task.recovery_retryable = activity.recovery_retryable;
        task.state = transfer_state_from_queue_state(activity.state);
        transfers.push(task);
        if let Some(handle) = queue.cancel_handle(activity.id) {
            cancel_handles.insert(task_id, handle);
        }
        if let Some(handle) = queue.pause_handle(activity.id) {
            pause_handles.insert(task_id, handle);
        }
        if let Some(handle) = queue.retry_handle(activity.id) {
            retry_handles.insert(task_id, handle);
        }
    }
    if transfers.is_empty() {
        return None;
    }
    transfers.sort_by_key(|task| {
        let priority = if !task.recovery_paths.is_empty() {
            0
        } else if matches!(
            task.state,
            TransferState::InProgress | TransferState::Paused | TransferState::Cancelling
        ) {
            1
        } else {
            2
        };
        (priority, std::cmp::Reverse(task.id))
    });
    let (close_handle, scroll_handle) = queue.workspace_panel_handles();
    Some(render_transfer_panel(
        &transfers,
        &cancel_handles,
        &pause_handles,
        &retry_handles,
        Appearance::as_ref(app),
        close_handle,
        scroll_handle,
        TransferPanelTarget::Workspace,
    ))
}

fn render_transfer_panel_at_height(
    transfers: &[TransferTask],
    cancel_btn_states: &std::collections::HashMap<usize, MouseStateHandle>,
    pause_btn_states: &std::collections::HashMap<usize, MouseStateHandle>,
    retry_btn_states: &std::collections::HashMap<usize, MouseStateHandle>,
    appearance: &Appearance,
    close_btn_state: MouseStateHandle,
    scroll_state: ClippedScrollStateHandle,
    max_history_height: f32,
    target: TransferPanelTarget,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text_color = theme.active_ui_text_color();
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    // Title bar
    let count = transfers.len();
    let title_text = crate::t!("fm-transfer-title", count = count).to_string();

    let title_el = Text::new_inline(title_text, ui_font, ui_font_size)
        .with_color(text_color.into())
        .finish();

    // Close button
    let icon_color = theme.sub_text_color(theme.background());
    let close_btn = Hoverable::new(close_btn_state, move |_| {
        let icon_el = ConstrainedBox::new(Icon::X.to_warpui_icon(icon_color).finish())
            .with_width(12.0)
            .with_height(12.0)
            .finish();
        Container::new(icon_el)
            .with_padding_left(4.0)
            .with_padding_right(4.0)
            .with_padding_top(4.0)
            .with_padding_bottom(4.0)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| match target {
        TransferPanelTarget::Browser => {
            ctx.dispatch_typed_action(SftpBrowserAction::ToggleTransferPanel);
        }
        TransferPanelTarget::Workspace => {
            ctx.dispatch_typed_action(crate::WorkspaceAction::ClearTransferHistory);
        }
    })
    .finish();

    let header = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_child(title_el)
        .with_child(close_btn)
        .finish();

    let mut col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(header);

    let render_rows = || {
        let mut inner = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(4.0);
        for task in transfers {
            let row = render_transfer_row(
                task,
                cancel_btn_states.get(&task.id).cloned(),
                pause_btn_states.get(&task.id).cloned(),
                retry_btn_states.get(&task.id).cloned(),
                appearance,
                target,
            );
            inner.add_child(row);
        }
        inner.finish()
    };
    let scroller = ConstrainedBox::new(
        ClippedScrollable::vertical(
            scroll_state,
            render_rows(),
            ScrollbarWidth::Auto,
            theme.disabled_text_color(theme.background()).into(),
            theme.main_text_color(theme.background()).into(),
            Fill::None,
        )
        .finish(),
    )
    .with_max_height(max_history_height)
    .finish();
    col.add_child(SavePosition::new(scroller, "sftp_transfer_history_scroll").finish());

    Container::new(col.finish())
        .with_uniform_padding(PANEL_PADDING)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.0)))
        .with_background(theme.surface_2())
        .finish()
}

#[cfg(test)]
#[path = "transfer_panel_tests.rs"]
mod tests;
