//! SFTP integration testing helper functions
//!
//! Provide helpers for SFTP browser view acquisition, mock backend creation, pane opening, injection, etc.
//! author: logic
//! date: 2026-05-30

use std::path::PathBuf;
use std::sync::Arc;

use warpui::{App, ViewHandle, WindowId};

use crate::sftp_manager::browser::SftpBrowserView;
use crate::sftp_manager::sftp_backend::{InMemorySftpBackend, SftpBackend};

// Re-export for integration tests to use via warp::integration_testing::sftp
pub use crate::sftp_manager::browser::SftpBrowserAction;
pub use crate::sftp_manager::types::{ConnectionState, Dialog};

/// Get SFTP browser view handle
///
/// Find SftpBrowserView instance in the specified window.
/// author: logic
/// date: 2026-05-30
pub fn sftp_browser_view(app: &App, window_id: WindowId) -> ViewHandle<SftpBrowserView> {
    let views: Vec<ViewHandle<SftpBrowserView>> = app
        .views_of_type(window_id)
        .expect("should have views for window");
    views
        .into_iter()
        .next()
        .expect("should have at least one SFTP browser view")
}

/// Create temporary directory and mock backend with preset file structure
///
/// files is a list of (relative path, content); automatically creates needed parent directories.
/// author: logic
/// date: 2026-05-30
pub fn create_mock_backend(
    files: &[(&str, &[u8])],
) -> (tempfile::TempDir, Arc<dyn SftpBackend>) {
    let temp_dir = tempfile::tempdir().expect("failed to create temp directory");
    for (path, content) in files {
        let full_path = temp_dir.path().join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create subdirectory");
        }
        std::fs::write(&full_path, content).expect("failed to write test file");
    }
    let backend = Arc::new(InMemorySftpBackend::new(temp_dir.path().to_path_buf()))
        as Arc<dyn SftpBackend>;
    (temp_dir, backend)
}

/// Open SFTP pane and inject mock backend
///
/// Return (window_id, temp_dir); temp_dir must stay alive during test.
/// author: logic
/// date: 2026-05-30
pub fn open_sftp_pane_with_mock(
    app: &mut App,
    files: &[(&str, &[u8])],
) -> (WindowId, tempfile::TempDir) {
    let window_id = app.read(|ctx| {
        ctx.windows()
            .active_window()
            .expect("should have active window")
    });

    let workspace = super::view_getters::workspace_view(app, window_id);
    app.update(|ctx| {
        workspace.update(ctx, |ws, ctx| {
            ws.open_sftp_pane("__mock_sftp_test__".to_string(), ctx);
        });
    });

    let (temp_dir, backend) = create_mock_backend(files);
    let view = sftp_browser_view(app, window_id);
    view.update(app, |v, ctx| {
        v.inject_mock_backend(backend, PathBuf::from("/"), ctx);
    });

    (window_id, temp_dir)
}
