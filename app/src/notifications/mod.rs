//! Notification center (mailbox + toast).
//!
//! Rebuilt after accidental deletion by 002ce467 cloud-removal; preserves only local paths unrelated to cloud:
//! - Software native BYOP agent (Oz) completion/error notifications
//! - Third-party CLI agent (Claude Code / Codex / DeepSeek, etc.) status notifications
//!
//! Module layout:
//! - `item`           — data model (`NotificationItem` / `NotificationItems`, etc.)
//! - `item_rendering` — single notification UI (shared by mailbox and toast)
//! - `model`          — singleton `NotificationsModel` (subscribes to history / CLI session model, produces notifications)
//! - `view`           — `NotificationMailboxView` (mailbox main panel)
//! - `toast_stack`    — `AgentNotificationToastStack` (bottom-right toast)
//! - `telemetry`      — notification center telemetry events (`NotificationsTelemetryEvent`)

pub(crate) mod item;
pub(crate) mod item_rendering;
pub mod model;
pub(crate) mod telemetry;
pub mod toast_stack;
pub mod view;

pub(crate) use item::{
    NotificationCategory, NotificationFilter, NotificationId, NotificationItem, NotificationItems,
    NotificationSourceAgent,
};
pub use toast_stack::AgentNotificationToastStack;
pub use view::{NotificationMailboxView, NotificationMailboxViewEvent};

pub fn init(app: &mut warpui::AppContext) {
    NotificationMailboxView::init(app);
}
