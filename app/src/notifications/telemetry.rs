//! Telemetry events for the in-app notification mailbox / toast stack.
//!
//! This is a minimal cut-down version of `AgentManagementTelemetryEvent` removed in 002ce467 cloud-removal,
//! retaining only the variants actually still used by the notification center (`item_rendering.rs`) —
//! artifact click events + tombstones that no longer exist but kept for schema backward compatibility / future restoration.

use serde::Serialize;

/// Notification artifact type (for telemetry).
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    Plan,
    Branch,
    PullRequest,
}

/// Telemetry events related to the notification center.
#[derive(Serialize, Debug)]
pub enum NotificationsTelemetryEvent {
    /// User clicked an artifact button in a notification item (plan / branch / PR)
    ArtifactClicked { artifact_type: ArtifactType },
}
