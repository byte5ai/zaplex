//! Session management for the XDG RemoteDesktop and ScreenCast portals.
//!
//! This module handles creating and maintaining a portal session that enables
//! input emulation via the RemoteDesktop portal and provides stream IDs for
//! absolute pointer positioning via the ScreenCast portal.

use ashpd::desktop::remote_desktop::{
    DeviceType, RemoteDesktop, SelectDevicesOptions, StartOptions,
};
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::{CreateSessionOptions, PersistMode, Session};

/// A portal session that provides input emulation capabilities.
///
/// This session combines RemoteDesktop (for input) and ScreenCast (for absolute
/// positioning coordinates) into a single session with one permission dialog.
pub struct PortalSession {
    remote_desktop: RemoteDesktop,
    session: Session<RemoteDesktop>,
    /// The PipeWire stream node ID for the primary monitor.
    /// Used for absolute pointer positioning.
    stream_id: u32,
    /// The device types that were granted by the user.
    granted_devices: DeviceType,
}

impl PortalSession {
    /// Creates and starts a new portal session.
    ///
    /// This will show a permission dialog to the user (unless permissions are
    /// already persisted from a previous session).
    pub async fn new() -> Result<Self, String> {
        let remote_desktop = RemoteDesktop::new()
            .await
            .map_err(|e| format!("Failed to create RemoteDesktop proxy: {e}"))?;

        let screencast = Screencast::new()
            .await
            .map_err(|e| format!("Failed to create Screencast proxy: {e}"))?;

        // Create a RemoteDesktop session. This session is shared with ScreenCast.
        let session = remote_desktop
            .create_session(CreateSessionOptions::default())
            .await
            .map_err(|e| format!("Failed to create RemoteDesktop session: {e}"))?;

        // Select input devices (keyboard and pointer).
        // Note: Some portals don't support persistence for remote desktop sessions,
        // so we use DoNot to avoid errors.
        remote_desktop
            .select_devices(
                &session,
                SelectDevicesOptions::default()
                    .set_devices(DeviceType::KEYBOARD | DeviceType::POINTER)
                    .set_persist_mode(PersistMode::DoNot),
            )
            .await
            .map_err(|e| format!("Failed to select devices: {e}"))?;

        // Select screencast sources (monitors) to get stream IDs for absolute positioning.
        // We use CursorMode::Metadata since we only need the stream ID, not the video.
        screencast
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_cursor_mode(CursorMode::METADATA)
                    .set_sources(SourceType::MONITOR)
                    // Allow multiple monitors.
                    .set_multiple(true)
                    .set_persist_mode(PersistMode::DoNot),
            )
            .await
            .map_err(|e| format!("Failed to select screencast sources: {e}"))?;

        // Start the session. This shows the permission dialog to the user.
        let response = remote_desktop
            .start(&session, None, StartOptions::default())
            .await
            .map_err(|e| format!("Failed to start session request: {e}"))?
            .response()
            .map_err(|e| format!("Session start failed: {e}"))?;

        // Extract the stream ID from the response.
        let streams = response.streams();

        if streams.is_empty() {
            return Err("No monitors available for screen casting".to_string());
        }

        // Use the first stream (primary monitor) for now.
        let stream_id = streams[0].pipe_wire_node_id();

        // Get the devices that were actually granted by the user.
        let granted_devices = response.devices();

        Ok(Self {
            remote_desktop,
            session,
            stream_id,
            granted_devices,
        })
    }

    /// Returns a reference to the RemoteDesktop proxy.
    pub fn remote_desktop(&self) -> &RemoteDesktop {
        &self.remote_desktop
    }

    /// Returns a reference to the session.
    pub fn session(&self) -> &Session<RemoteDesktop> {
        &self.session
    }

    /// Returns the stream ID for the primary monitor.
    ///
    /// This ID is used for absolute pointer positioning via
    /// `notify_pointer_motion_absolute`.
    pub fn stream_id(&self) -> u32 {
        self.stream_id
    }

    /// Validates that keyboard input permission was granted.
    ///
    /// Returns an error with a clear message for the agent if permission was denied.
    pub fn require_keyboard(&self) -> Result<(), String> {
        if self.granted_devices.contains(DeviceType::KEYBOARD) {
            Ok(())
        } else {
            Err(
                "Keyboard input permission was not granted. \
                The user must allow keyboard input in the portal dialog to perform keyboard actions."
                    .to_string(),
            )
        }
    }

    /// Validates that pointer/mouse input permission was granted.
    ///
    /// Returns an error with a clear message for the agent if permission was denied.
    pub fn require_pointer(&self) -> Result<(), String> {
        if self.granted_devices.contains(DeviceType::POINTER) {
            Ok(())
        } else {
            Err("Mouse input permission was not granted. \
                The user must allow mouse input in the portal dialog to perform mouse actions."
                .to_string())
        }
    }
}
