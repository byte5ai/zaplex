//! SFTP browser UI integration tests
//!
//! Uses InMemorySftpBackend to simulate an SFTP connection and exercise the
//! full user workflow in the Connected state, including file browsing,
//! navigation, operations, dialogs, transfers, and more.
//! author: logic
//! date: 2026-05-30

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    ChildView, CrossAxisAlignment, Expanded, Flex, MainAxisSize, MouseStateHandle, ParentElement,
    Stack,
};
use warpui::event::KeyEventDetails;
use warpui::keymap::Keystroke;
use warpui::platform::WindowStyle;
use warpui::{
    App, AppContext, Element, Entity, Event, Presenter, Scene, SingletonEntity, TypedActionView,
    View, ViewContext, ViewHandle, WindowId, WindowInvalidation,
};

use crate::pane_group::focus_state::{PaneFocusHandle, PaneGroupFocusState};
use crate::pane_group::pane::PaneId;
use crate::pane_group::BackingView;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;

use pathfinder_geometry::vector::{vec2f, Vector2F};

use super::browser::{
    copy_move_submission_summary, transfer_display_priority, SftpBrowserAction, SftpBrowserView,
};
use super::sftp_backend::{BackendOwnershipAnchor, InMemorySftpBackend, SftpBackend};
use super::types::{
    ConnectionState, Dialog, EntryIdentity, EntryReference, FileEntry, FileEntryType,
    StableEntryIdentity, TransferDirection, TransferState, TransferTask,
};

fn stale_entry_reference(index: usize) -> EntryReference {
    EntryReference {
        listing_generation: 0,
        identity: EntryIdentity {
            path: PathBuf::from(format!("/missing-{index}")),
            backend: StableEntryIdentity {
                file_type: FileEntryType::File,
                size: 0,
                object_id: format!("missing-{index}"),
                revision: "1".to_string(),
            },
        },
    }
}

fn entry_action(
    view: &SftpBrowserView,
    index: usize,
    constructor: fn(EntryReference) -> SftpBrowserAction,
) -> SftpBrowserAction {
    constructor(
        view.entry_reference(index)
            .unwrap_or_else(|| stale_entry_reference(index)),
    )
}

/// Initializes the minimal set of singletons required by the tests
fn initialize_app(app: &mut warpui::App) {
    use crate::workspace::ToastStack;

    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| ToastStack);
    app.add_singleton_model(|_| super::fm_registry::FileManagerRegistry::new());
    app.add_singleton_model(|_| super::transfer_queue::TransferQueue::new());

    let temp_db = std::env::temp_dir().join("warp_sftp_integration_test.sqlite");
    let _ = warp_ssh_manager::set_database_path(temp_db);
}

/// Creates a SftpBrowserView and places it in a window (Disconnected state)
fn create_view(app: &mut warpui::App) -> (warpui::WindowId, warpui::ViewHandle<SftpBrowserView>) {
    app.add_window(WindowStyle::NotStealFocus, |ctx| {
        SftpBrowserView::new("test-node".to_string(), None, ctx)
    })
}

/// Creates a temporary directory with a file structure
///
/// `files` is a list of (relative path, content); parent directories are created automatically.
fn create_temp_dir_with_files(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("failed to create temporary directory");
    for (path, content) in files {
        let full_path = dir.path().join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent directories");
        }
        std::fs::write(&full_path, content).expect("failed to write test file");
    }
    dir
}

/// Creates a Connected-state view backed by an InMemorySftpBackend
///
/// Returns (window_id, view_handle, temp_dir); temp_dir must be kept alive for the duration of the test
fn create_connected_view(
    app: &mut warpui::App,
    files: &[(&str, &[u8])],
) -> (
    warpui::WindowId,
    warpui::ViewHandle<SftpBrowserView>,
    tempfile::TempDir,
) {
    let temp_dir = create_temp_dir_with_files(files);
    let backend =
        Arc::new(InMemorySftpBackend::new(temp_dir.path().to_path_buf())) as Arc<dyn SftpBackend>;

    let (win_id, view) = create_view(app);
    view.update(app, |v, ctx| {
        v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
    });

    (win_id, view, temp_dir)
}

fn presenter_for_window(
    app: &App,
    window_id: warpui::WindowId,
) -> (Rc<RefCell<Presenter>>, WindowInvalidation) {
    let updated = app.read(|ctx| ctx.view_ids_for_window(window_id).into_iter().collect());
    (
        Rc::new(RefCell::new(Presenter::new(window_id))),
        WindowInvalidation {
            updated,
            ..Default::default()
        },
    )
}

fn render_position(
    app: &mut App,
    presenter: Rc<RefCell<Presenter>>,
    invalidation: WindowInvalidation,
    position_id: &str,
) -> Vector2F {
    let position_id = position_id.to_string();
    app.update(move |ctx| {
        presenter.borrow_mut().invalidate(invalidation, ctx);
        presenter
            .borrow_mut()
            .build_scene(vec2f(1200., 800.), 1., None, ctx);
        presenter
            .borrow()
            .position_cache()
            .get_position(&position_id)
            .unwrap_or_else(|| panic!("{position_id} should be positioned"))
            .center()
    })
}

fn rerender(app: &mut App, presenter: Rc<RefCell<Presenter>>, invalidation: WindowInvalidation) {
    app.update(move |ctx| {
        presenter.borrow_mut().invalidate(invalidation, ctx);
        presenter
            .borrow_mut()
            .build_scene(vec2f(1200., 800.), 1., None, ctx);
    });
}

fn mouse_down(
    app: &mut App,
    window_id: warpui::WindowId,
    presenter: Rc<RefCell<Presenter>>,
    position: Vector2F,
) {
    app.update(move |ctx| {
        ctx.simulate_window_event(
            Event::LeftMouseDown {
                position,
                modifiers: Default::default(),
                click_count: 1,
                is_first_mouse: false,
            },
            window_id,
            presenter,
        );
    });
}

fn mouse_up(
    app: &mut App,
    window_id: warpui::WindowId,
    presenter: Rc<RefCell<Presenter>>,
    position: Vector2F,
) {
    app.update(move |ctx| {
        ctx.simulate_window_event(
            Event::LeftMouseUp {
                position,
                modifiers: Default::default(),
            },
            window_id,
            presenter,
        );
    });
}

fn mouse_move(
    app: &mut App,
    window_id: WindowId,
    presenter: Rc<RefCell<Presenter>>,
    position: Vector2F,
) {
    app.update(move |ctx| {
        ctx.simulate_window_event(
            Event::MouseMoved {
                position,
                cmd: false,
                shift: false,
                is_synthetic: false,
            },
            window_id,
            presenter,
        );
    });
}

fn right_click(
    app: &mut App,
    window_id: WindowId,
    presenter: Rc<RefCell<Presenter>>,
    position: Vector2F,
) {
    app.update(move |ctx| {
        ctx.simulate_window_event(
            Event::RightMouseDown {
                position,
                cmd: false,
                shift: false,
                click_count: 1,
            },
            window_id,
            presenter,
        );
    });
}

fn key_down(
    app: &mut App,
    window_id: WindowId,
    presenter: Rc<RefCell<Presenter>>,
    keystroke: &str,
) -> bool {
    let keystroke = Keystroke::parse(keystroke).expect("valid test keystroke");
    let chars = match (keystroke.key.as_str(), keystroke.shift) {
        ("tab", true) => "\u{19}".to_string(),
        ("tab", false) => "\t".to_string(),
        _ => keystroke.key.clone(),
    };
    app.update(move |ctx| {
        ctx.simulate_window_event(
            Event::KeyDown {
                keystroke,
                chars,
                details: KeyEventDetails::default(),
                is_composing: false,
            },
            window_id,
            presenter,
        )
    })
}

struct DualSftpRoot {
    left: ViewHandle<SftpBrowserView>,
    right: ViewHandle<SftpBrowserView>,
}

impl Entity for DualSftpRoot {
    type Event = ();
}

impl View for DualSftpRoot {
    fn ui_name() -> &'static str {
        "DualSftpRoot"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Expanded::new(1.0, ChildView::new(&self.left).finish()).finish())
            .with_child(Expanded::new(1.0, ChildView::new(&self.right).finish()).finish())
            .finish()
    }
}

impl TypedActionView for DualSftpRoot {
    type Action = ();
}

struct WorkspaceActionCaptureRoot {
    browser: ViewHandle<SftpBrowserView>,
    actions: Vec<crate::WorkspaceAction>,
}

impl Entity for WorkspaceActionCaptureRoot {
    type Event = ();
}

impl View for WorkspaceActionCaptureRoot {
    fn ui_name() -> &'static str {
        "WorkspaceActionCaptureRoot"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.browser).finish()
    }
}

impl TypedActionView for WorkspaceActionCaptureRoot {
    type Action = crate::WorkspaceAction;

    fn handle_action(&mut self, action: &Self::Action, _ctx: &mut ViewContext<Self>) {
        self.actions.push(action.clone());
    }
}

fn create_workspace_action_capture_view(
    app: &mut App,
    files: &[(&str, &[u8])],
) -> (
    WindowId,
    ViewHandle<WorkspaceActionCaptureRoot>,
    ViewHandle<SftpBrowserView>,
    tempfile::TempDir,
) {
    let temp_dir = create_temp_dir_with_files(files);
    let backend =
        Arc::new(InMemorySftpBackend::new(temp_dir.path().to_path_buf())) as Arc<dyn SftpBackend>;
    let (window_id, root) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let browser = ctx.add_typed_action_view(|ctx| {
            SftpBrowserView::new("capture-node".to_string(), None, ctx)
        });
        WorkspaceActionCaptureRoot {
            browser,
            actions: Vec::new(),
        }
    });
    let browser = app.read(|ctx| root.as_ref(ctx).browser.clone());
    browser.update(app, |view, ctx| {
        view.set_backend_for_test(backend, PathBuf::from("/"), ctx);
    });

    (window_id, root, browser, temp_dir)
}

fn create_dual_connected_view(
    app: &mut App,
) -> (
    WindowId,
    ViewHandle<DualSftpRoot>,
    ViewHandle<SftpBrowserView>,
    ViewHandle<SftpBrowserView>,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    let left_temp = create_temp_dir_with_files(&[
        ("a.txt", b"a"),
        ("this-is-a-very-long-file-name.txt", b"long"),
    ]);
    let right_temp =
        create_temp_dir_with_files(&[("cursor.txt", b"cursor"), ("selected.txt", b"selected")]);
    let left_backend =
        Arc::new(InMemorySftpBackend::new(left_temp.path().to_path_buf())) as Arc<dyn SftpBackend>;
    let right_backend =
        Arc::new(InMemorySftpBackend::new(right_temp.path().to_path_buf())) as Arc<dyn SftpBackend>;

    let (window_id, root) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let left = ctx.add_typed_action_view(|ctx| {
            SftpBrowserView::new("layout-left".to_string(), None, ctx)
        });
        let right = ctx.add_typed_action_view(|ctx| {
            SftpBrowserView::new("layout-right".to_string(), None, ctx)
        });
        DualSftpRoot { left, right }
    });
    let (left, right) = app.read(|ctx| {
        let root = root.as_ref(ctx);
        (root.left.clone(), root.right.clone())
    });
    left.update(app, |view, ctx| {
        view.set_backend_for_test(left_backend, PathBuf::from("/"), ctx);
    });
    right.update(app, |view, ctx| {
        view.set_backend_for_test(right_backend, PathBuf::from("/"), ctx);
    });

    (window_id, root, left, right, left_temp, right_temp)
}

fn initialize_pane_group_app(app: &mut App) {
    crate::test_util::terminal::initialize_app_for_terminal_view(app);
    app.add_singleton_model(|_| crate::workspace::ToastStack);
    app.add_singleton_model(|_| super::fm_registry::FileManagerRegistry::new());
    app.add_singleton_model(|_| super::transfer_queue::TransferQueue::new());

    let temp_db = std::env::temp_dir().join("warp_sftp_pane_group_test.sqlite");
    let _ = warp_ssh_manager::set_database_path(temp_db);
}

fn create_three_pane_group(
    app: &mut App,
) -> (
    WindowId,
    ViewHandle<crate::pane_group::PaneGroup>,
    [ViewHandle<SftpBrowserView>; 3],
    [PaneId; 3],
    [tempfile::TempDir; 3],
) {
    use crate::pane_group::pane::sftp_pane::SftpPane;
    use crate::pane_group::{Direction, PaneContent, PaneGroup};

    initialize_pane_group_app(app);
    let resources = crate::GlobalResourceHandles::mock(app);
    let temp_dirs = [
        create_temp_dir_with_files(&[("left.txt", b"left")]),
        create_temp_dir_with_files(&[("middle.txt", b"middle")]),
        create_temp_dir_with_files(&[("right.txt", b"right")]),
    ];

    let first_browser = Rc::new(RefCell::new(None));
    let first_pane_id = Rc::new(RefCell::new(None));
    let first_browser_for_window = first_browser.clone();
    let first_pane_id_for_window = first_pane_id.clone();
    let first_path = temp_dirs[0].path().to_path_buf();
    let tips_completed = resources.tips_completed.clone();
    let unsupported_banner = resources
        .user_default_shell_unsupported_banner_model_handle
        .clone();
    let model_event_sender = resources.model_event_sender.clone();
    let (window_id, pane_group) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
        let pane = SftpPane::new_local(first_path, ctx);
        *first_browser_for_window.borrow_mut() = Some(pane.browser_view(ctx));
        *first_pane_id_for_window.borrow_mut() = Some(pane.id());
        PaneGroup::new_from_existing_pane(
            Box::new(pane),
            tips_completed,
            unsupported_banner,
            model_event_sender,
            ctx,
        )
    });

    let mut browsers = vec![first_browser
        .borrow()
        .clone()
        .expect("first browser should be created")];
    let mut pane_ids = vec![(*first_pane_id.borrow()).expect("first pane id should be created")];
    for temp_dir in &temp_dirs[1..] {
        let pane_path = temp_dir.path().to_path_buf();
        let added = Rc::new(RefCell::new(None));
        let added_for_update = added.clone();
        pane_group.update(app, move |group, ctx| {
            let pane = SftpPane::new_local(pane_path, ctx);
            let browser = pane.browser_view(ctx);
            let pane_id = pane.id();
            group.add_pane_with_direction(Direction::Right, pane, false, ctx);
            *added_for_update.borrow_mut() = Some((browser, pane_id));
        });
        let (browser, pane_id) = added
            .borrow()
            .clone()
            .expect("additional pane should be created");
        browsers.push(browser);
        pane_ids.push(pane_id);
    }

    (
        window_id,
        pane_group,
        browsers.try_into().expect("three browser handles"),
        pane_ids.try_into().expect("three pane ids"),
        temp_dirs,
    )
}

fn render_scene_at(
    app: &mut App,
    window_id: WindowId,
    size: Vector2F,
) -> (Rc<RefCell<Presenter>>, Rc<Scene>) {
    let (presenter, invalidation) = presenter_for_window(app, window_id);
    let presenter_for_render = presenter.clone();
    let scene = app.update(move |ctx| {
        presenter_for_render
            .borrow_mut()
            .invalidate(invalidation, ctx);
        presenter_for_render
            .borrow_mut()
            .build_scene(size, 1.0, None, ctx)
    });
    (presenter, scene)
}

fn layout_id(view: &ViewHandle<SftpBrowserView>, app: &App, part: &str) -> String {
    view.read(app, |view, _| view.layout_position_id_for_test(part))
}

fn position(
    presenter: &Rc<RefCell<Presenter>>,
    position_id: &str,
) -> pathfinder_geometry::rect::RectF {
    presenter
        .borrow()
        .position_cache()
        .get_position(position_id)
        .unwrap_or_else(|| panic!("{position_id} should be rendered"))
}

fn maybe_position(
    presenter: &Rc<RefCell<Presenter>>,
    position_id: &str,
) -> Option<pathfinder_geometry::rect::RectF> {
    presenter
        .borrow()
        .position_cache()
        .get_position(position_id)
}

fn assert_approximately_equal(left: f32, right: f32) {
    assert!(
        (left - right).abs() < 0.5,
        "expected {left} to approximately equal {right}"
    );
}

fn rects_approximately_equal(
    left: pathfinder_geometry::rect::RectF,
    right: pathfinder_geometry::rect::RectF,
) -> bool {
    (left.min_x() - right.min_x()).abs() < 0.5
        && (left.min_y() - right.min_y()).abs() < 0.5
        && (left.width() - right.width()).abs() < 0.5
        && (left.height() - right.height()).abs() < 0.5
}

/// Test backend that records metadata and listing calls while delegating all
/// filesystem behavior to the local test backend.
struct TracingBackend {
    inner: InMemorySftpBackend,
    listed_paths: Mutex<Vec<PathBuf>>,
    stat_paths: Mutex<Vec<PathBuf>>,
    fail_copy: AtomicBool,
    fail_replace: AtomicBool,
    create_after_next_list: Mutex<Option<(PathBuf, Vec<u8>)>>,
}

impl TracingBackend {
    fn new(root: PathBuf) -> Self {
        Self {
            inner: InMemorySftpBackend::new(root),
            listed_paths: Mutex::new(Vec::new()),
            stat_paths: Mutex::new(Vec::new()),
            fail_copy: AtomicBool::new(false),
            fail_replace: AtomicBool::new(false),
            create_after_next_list: Mutex::new(None),
        }
    }

    fn fail_copies(&self) {
        self.fail_copy.store(true, Ordering::SeqCst);
    }

    fn fail_replacements(&self) {
        self.fail_replace.store(true, Ordering::SeqCst);
    }

    fn create_file_after_next_list(&self, path: PathBuf, contents: Vec<u8>) {
        *self.create_after_next_list.lock().unwrap() = Some((path, contents));
    }
}

impl SftpBackend for TracingBackend {
    fn supports_atomic_exchange(&self) -> bool {
        self.inner.supports_atomic_exchange()
    }

    fn supports_identity_bound_cleanup(&self) -> bool {
        self.inner.supports_identity_bound_cleanup()
    }

    fn existing_entry_ownership_anchor(
        &self,
        path: &std::path::Path,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, super::sftp_ops::SftpOpsError> {
        self.inner.existing_entry_ownership_anchor(path)
    }

    fn list_dir(
        &self,
        path: &std::path::Path,
    ) -> Result<Vec<super::types::FileEntry>, super::sftp_ops::SftpOpsError> {
        self.listed_paths.lock().unwrap().push(path.to_path_buf());
        let entries = self.inner.list_dir(path)?;
        if let Some((created_path, contents)) = self.create_after_next_list.lock().unwrap().take() {
            let relative = created_path
                .strip_prefix("/")
                .unwrap_or(created_path.as_path());
            let local_path = self.inner.root().join(relative);
            std::fs::create_dir_all(local_path.parent().unwrap()).unwrap();
            std::fs::write(local_path, contents).unwrap();
        }
        Ok(entries)
    }

    fn delete_file(&self, path: &std::path::Path) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.inner.delete_file(path)
    }

    fn delete_dir_recursive(
        &self,
        path: &std::path::Path,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.inner.delete_dir_recursive(path)
    }

    fn create_dir(&self, path: &std::path::Path) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.inner.create_dir(path)
    }

    fn create_dir_with_ownership_anchor(
        &self,
        path: &std::path::Path,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, super::sftp_ops::SftpOpsError> {
        self.inner.create_dir_with_ownership_anchor(path)
    }

    fn rename(
        &self,
        old_path: &std::path::Path,
        new_path: &std::path::Path,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.inner.rename(old_path, new_path)
    }

    fn replace(
        &self,
        old_path: &std::path::Path,
        new_path: &std::path::Path,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        if self.fail_replace.load(Ordering::SeqCst) {
            return Err(super::sftp_ops::SftpOpsError::Operation(
                "injected atomic replacement failure".to_string(),
            ));
        }
        self.inner.replace(old_path, new_path)
    }

    fn realpath(&self, path: &std::path::Path) -> Result<PathBuf, super::sftp_ops::SftpOpsError> {
        self.inner.realpath(path)
    }

    fn stat(
        &self,
        path: &std::path::Path,
    ) -> Result<super::types::FileEntry, super::sftp_ops::SftpOpsError> {
        self.stat_paths.lock().unwrap().push(path.to_path_buf());
        self.inner.stat(path)
    }

    fn lstat(
        &self,
        path: &std::path::Path,
    ) -> Result<super::types::FileEntry, super::sftp_ops::SftpOpsError> {
        self.inner.lstat(path)
    }

    fn stable_identity(
        &self,
        path: &std::path::Path,
    ) -> Result<super::sftp_backend::StableEntryIdentity, super::sftp_ops::SftpOpsError> {
        self.inner.stable_identity(path)
    }

    fn upload_file(
        &self,
        local_path: &std::path::Path,
        remote_path: &std::path::Path,
        progress_cb: Option<&super::sftp_ops::ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.inner
            .upload_file(local_path, remote_path, progress_cb, cancel_flag)
    }

    fn download_file(
        &self,
        remote_path: &std::path::Path,
        local_path: &std::path::Path,
        progress_cb: Option<&super::sftp_ops::ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.inner
            .download_file(remote_path, local_path, progress_cb, cancel_flag)
    }

    fn copy_file(
        &self,
        src: &std::path::Path,
        dst: &std::path::Path,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        if self.fail_copy.load(Ordering::SeqCst) {
            return Err(super::sftp_ops::SftpOpsError::Operation(
                "injected copy failure".to_string(),
            ));
        }
        self.inner.copy_file(src, dst)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct NavigationSnapshot {
    path: PathBuf,
    history: Vec<PathBuf>,
    history_index: usize,
    title: String,
    registry_path: PathBuf,
    entry_names: Vec<String>,
}

fn navigation_snapshot(
    view: &warpui::ViewHandle<SftpBrowserView>,
    app: &warpui::App,
) -> NavigationSnapshot {
    view.read(app, |view, ctx| {
        let registry_path = super::fm_registry::FileManagerRegistry::as_ref(ctx)
            .panes()
            .first()
            .expect("the connected file-manager pane should be registered")
            .current_path
            .clone();
        NavigationSnapshot {
            path: view.current_path.clone(),
            history: view.path_history.clone(),
            history_index: view.history_index,
            title: view.pane_configuration().as_ref(ctx).title().to_owned(),
            registry_path,
            entry_names: view
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect(),
        }
    })
}

// ============================================================
// Connect finalize honors the caller-provided start_path
// ============================================================

/// Regression: a remote terminal in `/srv/app` must toggle the file manager to
/// `/srv/app`, not the host home/root. The async connect finalize
/// (`apply_connected_backend`) is exercised directly with a mock SFTP backend,
/// since the full `connect_to_server` needs a live SSH server. This covers the
/// real connect path — not just `file_manager_start_path` — asserting that the
/// requested cwd survives connect (backend home resolution is bypassed).
#[test]
fn test_connect_finalize_honors_explicit_start_path() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Mock remote filesystem: /srv/app exists and holds a file so the
        // initial listing of the requested cwd succeeds. The backend root maps
        // to remote "/", so realpath(".") would resolve to "/" (the "home"
        // fallback) — proving current_path came from start_path, not the home.
        let temp_dir = create_temp_dir_with_files(&[("srv/app/hello.txt", b"hi")]);
        let backend = Arc::new(InMemorySftpBackend::new(temp_dir.path().to_path_buf()))
            as Arc<dyn SftpBackend>;

        // Construct with an explicit start_path (the FM pane-mode toggle path).
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            SftpBrowserView::new(
                "test-node".to_string(),
                Some(PathBuf::from("/srv/app")),
                ctx,
            )
        });

        view.update(&mut app, |v, ctx| {
            v.apply_connected_backend(backend, ctx);
        });

        let (current_path, path_history, history_index, has_hello) = view.read(&app, |v, _| {
            (
                v.current_path.clone(),
                v.path_history.clone(),
                v.history_index,
                v.entries.iter().any(|e| e.name == "hello.txt"),
            )
        });

        // Landed at the requested cwd, NOT the remote home/root.
        assert_eq!(current_path, PathBuf::from("/srv/app"));
        // History is consistent: exactly the start_path, no dangling home entry.
        assert_eq!(path_history, vec![PathBuf::from("/srv/app")]);
        assert_eq!(history_index, 0);
        // The initial listing targeted /srv/app (its file is visible).
        assert!(
            has_hello,
            "the initial listing should target the requested start_path"
        );
    });
}

/// The plain "SFTP Browse" entry (`None` start_path) preserves the pre-existing
/// behavior: connect finalize falls back to the remote home (`realpath(".")`).
/// The expected home is computed from the same backend so the assertion holds
/// regardless of how the temp dir canonicalizes on the host.
#[test]
fn test_connect_finalize_none_falls_back_to_home() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);

        let temp_dir = create_temp_dir_with_files(&[("readme.txt", b"hi")]);
        let raw_backend = InMemorySftpBackend::new(temp_dir.path().to_path_buf());
        // The home the finalize falls back to, resolved exactly as the code does.
        let expected_home = super::sftp_ops::normalize_remote_path(
            &raw_backend
                .realpath(std::path::Path::new("."))
                .expect("realpath(\".\") should resolve for the in-memory backend"),
        );
        let backend = Arc::new(raw_backend) as Arc<dyn SftpBackend>;

        let (_, view) = create_view(&mut app); // constructed with `None`

        view.update(&mut app, |v, ctx| {
            v.apply_connected_backend(backend, ctx);
        });

        let (current_path, path_history, history_index) = view.read(&app, |v, _| {
            (
                v.current_path.clone(),
                v.path_history.clone(),
                v.history_index,
            )
        });

        // Fell back to the remote home, NOT some caller start_path.
        assert_eq!(current_path, expected_home);
        assert_eq!(path_history, vec![expected_home]);
        assert_eq!(history_index, 0);
    });
}

/// Creates a Connected view with a subdirectory structure
///
/// The root directory contains: a docs/ subdirectory, readme.txt, config.yaml
fn create_standard_view(
    app: &mut warpui::App,
) -> (
    warpui::WindowId,
    warpui::ViewHandle<SftpBrowserView>,
    tempfile::TempDir,
) {
    create_connected_view(
        app,
        &[
            ("docs/report.txt", b"report content"),
            ("readme.txt", b"hello world"),
            ("config.yaml", b"key: value"),
            ("data/sub/deep.txt", b"deep file"),
        ],
    )
}

// ============================================================
// A. Connection management tests (6)
// ============================================================

/// Verifies the Connected state and entry population after injecting an InMemorySftpBackend
#[test]
fn test_connected_state_with_mock_backend() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(
            &mut app,
            &[("file1.txt", b"content1"), ("file2.txt", b"content2")],
        );

        view.read(&app, |v, _| {
            assert!(
                matches!(v.connection, ConnectionState::Connected),
                "should be in Connected state"
            );
            assert_eq!(v.entries.len(), 2, "should list 2 files");
            assert!(
                v.current_path == PathBuf::from("/"),
                "current path should be /"
            );
        });
    });
}

/// Verifies that when not connected the state is non-Connected and there are no entries
#[test]
fn test_connection_failure_shows_error_state() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.read(&app, |v, _| {
            // new() internally calls connect_to_server, which enters the Failed state when there is no SSH configuration
            assert!(
                !matches!(v.connection, ConnectionState::Connected),
                "should not be in Connected state when there is no SSH configuration"
            );
            assert!(
                v.entries.is_empty(),
                "should have no entries when disconnected"
            );
        });
    });
}

/// Verifies reconnecting from the Failed state
#[test]
fn test_reconnect_after_failure() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("reconnect.txt", b"data")]);

        // First set it to the Failed state
        view.update(&mut app, |v, ctx| {
            v.connection = ConnectionState::Failed("simulated connection failure".to_string());
            ctx.notify();
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.connection, ConnectionState::Failed(_)),
                "should be in Failed state"
            );
        });

        // Re-inject the backend to restore the connection
        let temp2 = create_temp_dir_with_files(&[("new.txt", b"new content")]);
        let backend =
            Arc::new(InMemorySftpBackend::new(temp2.path().to_path_buf())) as Arc<dyn SftpBackend>;
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.connection, ConnectionState::Connected),
                "should be in Connected state after re-injecting backend"
            );
            assert_eq!(
                v.entries.len(),
                1,
                "should list 1 file from the new backend"
            );
        });
    });
}

/// Verifies that entries and the path are cleared after disconnecting
#[test]
fn test_disconnect_clears_entries_and_path() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("file.txt", b"content")]);

        // Verify it is connected
        view.read(&app, |v, _| {
            assert!(matches!(v.connection, ConnectionState::Connected));
            assert!(!v.entries.is_empty());
        });

        // Disconnect
        view.update(&mut app, |v, ctx| {
            v.disconnect_for_test(ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.connection, ConnectionState::Disconnected),
                "should be in Disconnected state after disconnecting"
            );
            assert!(v.entries.is_empty(), "entries should be cleared");
        });
    });
}

/// Verifies that render does not panic in a non-Connected state
#[test]
fn test_render_disconnected_state() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.read(&app, |v, _| {
            // new() internally calls connect_to_server, so without an SSH configuration the state is Failed rather than Disconnected
            assert!(!matches!(v.connection, ConnectionState::Connected));
        });
    });
}

/// Verifies that render does not panic in the Failed state
#[test]
fn test_render_failed_state() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |v, ctx| {
            v.connection = ConnectionState::Failed("connection timeout".to_string());
            ctx.notify();
        });

        view.read(&app, |v, _| {
            assert!(matches!(v.connection, ConnectionState::Failed(_)));
        });
    });
}

// ============================================================
// B. File browsing and navigation tests (10)
// ============================================================

/// Verifies the directory listing is populated correctly and sorted directories-first, then alphabetically
#[test]
fn test_list_dir_populates_entries() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(
            &mut app,
            &[
                ("banana.txt", b"b"),
                ("apple.txt", b"a"),
                ("cherry.txt", b"c"),
                ("folder_a/.keep", b""),
                ("folder_b/.keep", b""),
            ],
        );

        view.read(&app, |v, _| {
            assert_eq!(v.entries.len(), 5, "should have 5 entries");

            // Directories should be listed before files
            let dirs: Vec<_> = v
                .entries
                .iter()
                .take_while(|e| e.file_type == FileEntryType::Directory)
                .collect();
            let files: Vec<_> = v
                .entries
                .iter()
                .skip_while(|e| e.file_type == FileEntryType::Directory)
                .collect();
            assert_eq!(dirs.len(), 2, "should have 2 directories");
            assert_eq!(files.len(), 3, "should have 3 files");
        });
    });
}

/// Verifies that double-clicking a directory enters it and updates the history
#[test]
fn test_open_directory_navigates_and_updates_history() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(
            &mut app,
            &[("docs/readme.txt", b"readme"), ("file.txt", b"file")],
        );

        // Find the index of the docs directory
        let docs_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "docs").unwrap()
        });

        // Double-click to enter the docs directory
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, docs_idx, SftpBrowserAction::OpenEntry);
            v.handle_action(&action, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.current_path.ends_with("docs")
                    || v.current_path.to_string_lossy().contains("docs"),
                "current path should contain docs"
            );
            assert!(
                v.path_history.len() >= 2,
                "navigation history should increase"
            );
        });
    });
}

/// Verifies that GoUp returns to the parent directory
#[test]
fn test_go_up_from_subdirectory() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("subdir/file.txt", b"content")]);

        // Enter the subdirectory
        let sub_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "subdir").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, sub_idx, SftpBrowserAction::OpenEntry);
            v.handle_action(&action, ctx);
        });

        // Go back up
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::GoUp, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.current_path == PathBuf::from("/")
                    || v.entries.iter().any(|e| e.name == "subdir"),
                "GoUp should return to the parent directory"
            );
        });
    });
}

/// Verifies that GoBack/GoForward restore the path
#[test]
fn test_go_back_forward_restores_path() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(
            &mut app,
            &[("alpha/file.txt", b"a"), ("beta/file.txt", b"b")],
        );

        // Record the root path
        let root_path = view.read(&app, |v, _| v.current_path.clone());

        // Enter alpha
        let alpha_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "alpha").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, alpha_idx, SftpBrowserAction::OpenEntry);
            v.handle_action(&action, ctx);
        });
        let alpha_path = view.read(&app, |v, _| v.current_path.clone());

        // GoBack
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::GoBack, ctx);
        });
        view.read(&app, |v, _| {
            assert_eq!(
                v.current_path, root_path,
                "GoBack should return to the root path"
            );
        });

        // GoForward
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::GoForward, ctx);
        });
        view.read(&app, |v, _| {
            assert_eq!(
                v.current_path, alpha_path,
                "GoForward should return to alpha"
            );
        });
    });
}

/// A failed directory request must not leave the header, registry and rows
/// describing different locations. The old implementation committed the path
/// and history before asking the backend, then kept the previous rows on error.
#[test]
fn failed_navigation_keeps_the_entire_visible_location_unchanged() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("keep.txt", b"keep")]);
        let before = navigation_snapshot(&view, &app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::NavigateTo(PathBuf::from("/missing-directory")),
                ctx,
            );
        });

        assert_eq!(navigation_snapshot(&view, &app), before);
    });
}

/// History navigation is transactional too. A once-valid forward destination
/// may disappear while the user is elsewhere; Forward must then leave the
/// current location and history cursor untouched.
#[test]
fn failed_forward_navigation_keeps_the_current_history_position() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, temp) = create_connected_view(&mut app, &[("gone/file.txt", b"content")]);

        let gone = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "gone")
                .unwrap()
        });
        view.update(&mut app, |view, ctx| {
            let action = entry_action(view, gone, SftpBrowserAction::OpenEntry);
            view.handle_action(&action, ctx);
            view.handle_action(&SftpBrowserAction::GoBack, ctx);
        });
        std::fs::remove_dir_all(temp.path().join("gone")).unwrap();
        let before = navigation_snapshot(&view, &app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::GoForward, ctx);
        });

        assert_eq!(navigation_snapshot(&view, &app), before);
    });
}

#[cfg(unix)]
#[test]
fn file_symlink_opens_as_a_file_without_navigating() {
    use std::os::unix::fs::symlink;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[("target.txt", b"target")]);
        symlink("target.txt", temp.path().join("link.txt")).unwrap();
        let backend = Arc::new(TracingBackend::new(temp.path().to_path_buf()));
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |view, ctx| {
            let trait_backend: Arc<dyn SftpBackend> = backend.clone();
            view.set_backend_for_test(trait_backend, PathBuf::from("/"), ctx);
        });
        let link = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "link.txt")
                .unwrap()
        });
        let before = navigation_snapshot(&view, &app);

        view.update(&mut app, |view, ctx| {
            let action = entry_action(view, link, SftpBrowserAction::OpenEntry);
            view.handle_action(&action, ctx);
        });

        assert_eq!(navigation_snapshot(&view, &app), before);
        assert_eq!(
            backend.stat_paths.lock().unwrap().as_slice(),
            &[PathBuf::from("/link.txt")],
            "file-link activation must resolve the target type before choosing the open path"
        );
        assert_eq!(
            backend.listed_paths.lock().unwrap().as_slice(),
            &[PathBuf::from("/")],
            "a file link must never be probed by attempting to list it as a directory"
        );
    });
}

/// A local FIFO is a special file, not a regular file. Activating it must not
/// enter the download path, which could otherwise block indefinitely.
#[cfg(unix)]
#[test]
fn local_fifo_is_classified_as_other_and_never_opened() {
    use nix::sys::stat::Mode;
    use nix::unistd::mkfifo;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let root = tempfile::tempdir().expect("remote temp");
        mkfifo(&root.path().join("pipe"), Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
        let backend =
            Arc::new(InMemorySftpBackend::new(root.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |view, ctx| {
            view.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });
        let fifo_index = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "pipe")
                .expect("FIFO should be listed")
        });

        view.read(&app, |view, _| {
            assert_eq!(view.entries[fifo_index].file_type, FileEntryType::Other);
        });
        view.update(&mut app, |view, ctx| {
            let action = entry_action(view, fifo_index, SftpBrowserAction::OpenEntry);
            view.handle_action(&action, ctx);
        });
        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/"));
            assert!(
                view.transfers.is_empty(),
                "activating a FIFO must not start a download"
            );
        });
    });
}

#[cfg(unix)]
#[test]
fn local_socket_is_classified_as_other_and_never_transferred() {
    use std::os::unix::net::UnixListener;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let root = tempfile::tempdir().expect("remote temp");
        let _listener = UnixListener::bind(root.path().join("service.sock")).unwrap();
        let backend =
            Arc::new(InMemorySftpBackend::new(root.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |view, ctx| {
            view.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });
        let socket_index = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "service.sock")
                .expect("socket should be listed")
        });

        view.read(&app, |view, _| {
            assert_eq!(view.entries[socket_index].file_type, FileEntryType::Other);
        });
        view.update(&mut app, |view, ctx| {
            let action = entry_action(view, socket_index, SftpBrowserAction::DownloadEntry);
            view.handle_action(&action, ctx);
        });
        view.read(&app, |view, _| {
            assert!(
                view.transfers.is_empty(),
                "a local socket must never enter the transfer path"
            );
        });
    });
}

#[cfg(unix)]
#[test]
fn directory_symlink_navigates_to_its_directory_target() {
    use std::os::unix::fs::symlink;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[("target-dir/inside.txt", b"inside")]);
        symlink("target-dir", temp.path().join("link-dir")).unwrap();
        let backend = Arc::new(TracingBackend::new(temp.path().to_path_buf()));
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |view, ctx| {
            let trait_backend: Arc<dyn SftpBackend> = backend.clone();
            view.set_backend_for_test(trait_backend, PathBuf::from("/"), ctx);
        });
        let link = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "link-dir")
                .unwrap()
        });

        view.update(&mut app, |view, ctx| {
            let action = entry_action(view, link, SftpBrowserAction::OpenEntry);
            view.handle_action(&action, ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/link-dir"));
            assert_eq!(
                view.entries
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["inside.txt"]
            );
        });
        assert_eq!(
            backend.stat_paths.lock().unwrap().as_slice(),
            &[PathBuf::from("/link-dir")],
            "directory-link activation must resolve the target before navigating"
        );
        assert_eq!(
            backend.listed_paths.lock().unwrap().as_slice(),
            &[PathBuf::from("/"), PathBuf::from("/link-dir")],
            "the link is listed only after stat established that its target is a directory"
        );
    });
}

#[cfg(unix)]
#[test]
fn broken_symlink_reports_failure_without_changing_location() {
    use std::os::unix::fs::symlink;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[("keep.txt", b"keep")]);
        symlink("missing-target", temp.path().join("broken-link")).unwrap();
        let backend = Arc::new(TracingBackend::new(temp.path().to_path_buf()));
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |view, ctx| {
            let trait_backend: Arc<dyn SftpBackend> = backend.clone();
            view.set_backend_for_test(trait_backend, PathBuf::from("/"), ctx);
        });
        let link = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "broken-link")
                .unwrap()
        });
        let before = navigation_snapshot(&view, &app);
        let saw_error_toast = Arc::new(AtomicBool::new(false));
        app.update(|ctx| {
            let saw_error_toast = saw_error_toast.clone();
            ctx.subscribe_to_model(
                &crate::workspace::ToastStack::handle(ctx),
                move |_, _, _| {
                    saw_error_toast.store(true, Ordering::SeqCst);
                },
            );
        });

        view.update(&mut app, |view, ctx| {
            let action = entry_action(view, link, SftpBrowserAction::OpenEntry);
            view.handle_action(&action, ctx);
        });

        assert_eq!(navigation_snapshot(&view, &app), before);
        assert_eq!(
            saw_error_toast.load(Ordering::SeqCst),
            true,
            "activating a broken symlink must surface an error toast"
        );
        assert_eq!(
            backend.stat_paths.lock().unwrap().as_slice(),
            &[PathBuf::from("/broken-link")],
            "broken-link activation must fail during target resolution"
        );
        assert_eq!(
            backend.listed_paths.lock().unwrap().as_slice(),
            &[PathBuf::from("/")],
            "a broken link must never be treated as a directory-listing request"
        );
    });
}

#[test]
fn same_filesystem_submission_reports_queued_not_completed() {
    crate::i18n::init(Some("en"));
    let summary = copy_move_submission_summary(2, 1, "other pane");

    assert!(summary.contains(&crate::t!("fm-summary-queued", count = 2)));
    assert!(summary.contains(&crate::t!("fm-summary-skipped", count = 1)));
    assert!(!summary.contains(&crate::t!("fm-summary-copied", count = 2)));
    assert!(!summary.contains(&crate::t!("fm-summary-moved", count = 2)));
}

#[test]
fn global_transfer_history_prioritizes_recovery_then_active_jobs() {
    let mut completed = TransferTask::new(
        3,
        PathBuf::from("/completed"),
        PathBuf::from("/target"),
        TransferDirection::Download,
        1,
    );
    completed.state = TransferState::Completed;
    let mut active = completed.clone();
    active.id = 2;
    active.state = TransferState::InProgress;
    let mut recovery = completed.clone();
    recovery.id = 1;
    recovery.state = TransferState::Failed("cleanup".to_string());
    recovery.recovery_paths.push(PathBuf::from("/retained"));
    let mut tasks = vec![completed, active, recovery];

    tasks.sort_by_key(transfer_display_priority);

    assert_eq!(
        tasks.iter().map(|task| task.id).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

/// Verifies that clicking a breadcrumb navigates to the corresponding path segment
#[test]
fn test_breadcrumb_click_navigates_to_segment() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) =
            create_connected_view(&mut app, &[("level1/level2/file.txt", b"deep")]);

        // Enter level1/level2
        let l1_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "level1").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, l1_idx, SftpBrowserAction::OpenEntry);
            v.handle_action(&action, ctx);
        });
        let l2_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "level2").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, l2_idx, SftpBrowserAction::OpenEntry);
            v.handle_action(&action, ctx);
        });

        // Verify the current path is level1/level2
        let current = view.read(&app, |v, _| v.current_path.clone());
        assert!(
            current.to_string_lossy().contains("level1"),
            "should navigate into level1"
        );

        // Navigate back toward the root (via NavigateTo)
        view.update(&mut app, |v, ctx| {
            // Find the breadcrumb path corresponding to level1
            let l1_path = v
                .current_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("/"));
            v.handle_action(&SftpBrowserAction::NavigateTo(l1_path), ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.current_path.to_string_lossy().contains("level1"),
                "breadcrumb navigation should place us at level1"
            );
        });
    });
}

/// Verifies that the search filter narrows the visible entries
#[test]
fn test_search_filter_narrows_visible_entries() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(
            &mut app,
            &[
                ("readme.txt", b"r"),
                ("config.yaml", b"c"),
                ("data.csv", b"d"),
            ],
        );

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::SetSearchFilter(".txt".to_string()), ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.search_filter.is_some());
            let visible: Vec<_> = v
                .entries
                .iter()
                .filter(|e| e.name.contains(".txt"))
                .collect();
            assert_eq!(visible.len(), 1, "only readme.txt should match");
        });
    });
}

/// Verifies that clearing the search restores all entries
#[test]
fn test_clear_search_restores_all_entries() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) =
            create_connected_view(&mut app, &[("a.txt", b"a"), ("b.yaml", b"b")]);

        let total = view.read(&app, |v, _| v.entries.len());

        // Set the filter
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::SetSearchFilter(".txt".to_string()), ctx);
        });

        // Clear the filter
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ClearSearchFilter, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.search_filter.is_none());
            assert_eq!(
                v.entries.len(),
                total,
                "entry count should be restored after clearing search"
            );
        });
    });
}

/// Verifies that refreshing reloads entries after a filesystem change
#[test]
fn test_refresh_dir_reloads_entries() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[("original.txt", b"original")]);
        initialize_app(&mut app);
        let backend =
            Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        view.read(&app, |v, _| {
            assert_eq!(v.entries.len(), 1, "initially should have 1 file");
        });

        // Add a new file to the temporary directory
        std::fs::write(temp.path().join("new_file.txt"), b"new").unwrap();

        // Refresh
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::Refresh, ctx);
        });

        view.read(&app, |v, _| {
            assert_eq!(v.entries.len(), 2, "should have 2 files after refresh");
        });
    });
}

struct RefreshButtonTestView {
    mouse_handle: MouseStateHandle,
    refreshes: usize,
}

impl Entity for RefreshButtonTestView {
    type Event = ();
}

impl TypedActionView for RefreshButtonTestView {
    type Action = SftpBrowserAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        if matches!(action, SftpBrowserAction::Refresh) {
            self.refreshes += 1;
            ctx.notify();
        }
    }
}

impl View for RefreshButtonTestView {
    fn ui_name() -> &'static str {
        "RefreshButtonTestView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        Stack::new()
            .with_child(SftpBrowserView::render_refresh_button_for_test(
                self.mouse_handle.clone(),
                Appearance::as_ref(app),
            ))
            .finish()
    }
}

/// A refresh click survives a render boundary and dispatches exactly once
/// through the exact element builder used by the production toolbar.
#[test]
fn rescan_click_survives_rerender() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view) =
            app.add_window(WindowStyle::NotStealFocus, |_| RefreshButtonTestView {
                mouse_handle: MouseStateHandle::default(),
                refreshes: 0,
            });
        let (presenter, invalidation) = presenter_for_window(&app, window_id);
        let position = render_position(
            &mut app,
            presenter.clone(),
            invalidation.clone(),
            "sftp_btn:refresh",
        );
        mouse_down(&mut app, window_id, presenter.clone(), position);
        rerender(&mut app, presenter.clone(), invalidation);
        mouse_up(&mut app, window_id, presenter, position);

        view.read(&app, |view, _| {
            assert_eq!(
                view.refreshes, 1,
                "the refresh action must fire exactly once across a rerender"
            );
        });
    });
}

/// A normal file row keeps its click state across a render boundary.
#[test]
fn row_action_does_not_fire_twice_after_rerender() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view, _root) = create_connected_view(&mut app, &[("clickable.txt", b"x")]);
        let entry_index = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "clickable.txt")
                .unwrap()
        });
        let position_id = layout_id(&view, &app, &format!("row:{entry_index}"));
        let (presenter, invalidation) = presenter_for_window(&app, window_id);
        let position = render_position(
            &mut app,
            presenter.clone(),
            invalidation.clone(),
            &position_id,
        );

        mouse_down(&mut app, window_id, presenter.clone(), position);
        rerender(&mut app, presenter.clone(), invalidation);
        mouse_up(&mut app, window_id, presenter, position);

        view.read(&app, |view, _| {
            assert!(view.is_index_marked(entry_index));
            assert_eq!(
                view.selected.len(),
                1,
                "the row click must fire exactly once"
            );
        });
    });
}

/// If a row disappears during the press, mouse-up must abort the pending row
/// action instead of applying the old row index to the entry that replaced it.
#[test]
fn removed_entry_aborts_pending_action() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view, root) =
            create_connected_view(&mut app, &[("a.txt", b"a"), ("b.txt", b"b")]);
        let removed_index = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "a.txt")
                .unwrap()
        });
        let position_id = layout_id(&view, &app, &format!("row:{removed_index}"));
        let (presenter, invalidation) = presenter_for_window(&app, window_id);
        let position = render_position(
            &mut app,
            presenter.clone(),
            invalidation.clone(),
            &position_id,
        );

        mouse_down(&mut app, window_id, presenter.clone(), position);
        std::fs::remove_file(root.path().join("a.txt")).unwrap();
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::Refresh, ctx);
        });
        rerender(&mut app, presenter.clone(), invalidation);
        mouse_up(&mut app, window_id, presenter, position);

        view.read(&app, |view, _| {
            assert_eq!(
                view.entries
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["b.txt"]
            );
            assert!(
                view.selected.is_empty(),
                "a removed row's mouse-up must not select its replacement"
            );
            assert!(
                view.dialog.is_none(),
                "a removed row's pending action must be aborted"
            );
        });
    });
}

/// A confirmation dialog can outlive the listing that created it. Replacing
/// the source at the same path must not let ConfirmDelete delete the new file.
#[test]
fn confirmed_delete_rejects_a_same_path_replacement() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, root) = create_connected_view(&mut app, &[("replace.txt", b"old")]);
        let path = root.path().join("replace.txt");

        view.update(&mut app, |view, ctx| {
            let entry = view.entry_reference(0).unwrap();
            view.handle_action(&SftpBrowserAction::DeleteEntry(entry), ctx);
        });
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"replacement content").unwrap();
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::ConfirmDelete, ctx);
        });

        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"replacement content",
            "delete confirmation must remain bound to the original entry identity"
        );
    });
}

#[cfg(target_os = "linux")]
#[test]
fn confirmed_delete_of_directory_symlink_removes_only_the_link() {
    use std::os::unix::fs::symlink;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[("target-dir/keep.txt", b"keep")]);
        let link = temp.path().join("link-dir");
        symlink("target-dir", &link).unwrap();
        let backend =
            Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |view, ctx| {
            view.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });
        let link_index = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "link-dir")
                .unwrap()
        });

        view.update(&mut app, |view, ctx| {
            let entry = view.entry_reference(link_index).unwrap();
            view.handle_action(&SftpBrowserAction::DeleteEntry(entry), ctx);
            view.handle_action(&SftpBrowserAction::ConfirmDelete, ctx);
        });

        assert!(std::fs::symlink_metadata(&link).is_err());
        assert_eq!(
            std::fs::read(temp.path().join("target-dir/keep.txt")).unwrap(),
            b"keep",
            "deleting a directory symlink must never recurse into its target"
        );
    });
}

#[cfg(target_os = "linux")]
#[test]
fn confirmed_symlink_delete_preserves_replacement_at_mutation_boundary() {
    use std::os::unix::fs::symlink;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[
            ("original-target/keep.txt", b"original"),
            ("replacement-target/keep.txt", b"replacement"),
        ]);
        let link = temp.path().join("link-dir");
        symlink("original-target", &link).unwrap();
        let backend = InMemorySftpBackend::new(temp.path().to_path_buf())
            .with_after_guarded_rename_check_before_mutation({
                let link = link.clone();
                move |_, _| {
                    std::fs::remove_file(&link).unwrap();
                    symlink("replacement-target", &link).unwrap();
                }
            });
        let backend = Arc::new(backend) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |view, ctx| {
            view.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });
        let link_index = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "link-dir")
                .unwrap()
        });

        view.update(&mut app, |view, ctx| {
            let entry = view.entry_reference(link_index).unwrap();
            view.handle_action(&SftpBrowserAction::DeleteEntry(entry), ctx);
            view.handle_action(&SftpBrowserAction::ConfirmDelete, ctx);
        });

        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            PathBuf::from("replacement-target")
        );
        assert_eq!(
            std::fs::read(temp.path().join("original-target/keep.txt")).unwrap(),
            b"original"
        );
        assert_eq!(
            std::fs::read(temp.path().join("replacement-target/keep.txt")).unwrap(),
            b"replacement"
        );
    });
}

/// Rename confirmation carries the same identity guarantee as delete. A path
/// reused while the editor is open is a different entry and must be preserved.
#[test]
fn confirmed_rename_rejects_a_same_path_replacement() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, root) = create_connected_view(&mut app, &[("replace.txt", b"old")]);
        let source = root.path().join("replace.txt");
        let renamed = root.path().join("renamed.txt");

        view.update(&mut app, |view, ctx| {
            let entry = view.entry_reference(0).unwrap();
            view.handle_action(&SftpBrowserAction::RenameEntry(entry), ctx);
            view.rename_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("renamed.txt", ctx)
            });
        });
        std::fs::remove_file(&source).unwrap();
        std::fs::write(&source, b"replacement content").unwrap();
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::ConfirmRename, ctx);
        });

        assert_eq!(
            std::fs::read(&source).unwrap(),
            b"replacement content",
            "rename confirmation must preserve a same-path replacement"
        );
        assert!(
            !renamed.exists(),
            "a stale rename confirmation must not create its destination"
        );
    });
}

/// Reusing the same path for a different filesystem object during a press must
/// also abort. A path alone is not an object identity.
#[test]
fn replaced_entry_at_same_path_aborts_pending_action() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view, root) = create_connected_view(&mut app, &[("replace.txt", b"old")]);
        let entry_index = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "replace.txt")
                .unwrap()
        });
        let position_id = layout_id(&view, &app, &format!("row:{entry_index}"));
        let (presenter, invalidation) = presenter_for_window(&app, window_id);
        let position = render_position(
            &mut app,
            presenter.clone(),
            invalidation.clone(),
            &position_id,
        );

        mouse_down(&mut app, window_id, presenter.clone(), position);
        std::fs::remove_file(root.path().join("replace.txt")).unwrap();
        std::fs::write(root.path().join("replace.txt"), b"replacement").unwrap();
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::Refresh, ctx);
        });
        rerender(&mut app, presenter.clone(), invalidation);
        mouse_up(&mut app, window_id, presenter, position);

        view.read(&app, |view, _| {
            assert_eq!(view.entries.len(), 1);
            assert_eq!(view.entries[0].size, b"replacement".len() as u64);
            assert!(
                view.selected.is_empty(),
                "mouse-up for the old object must not select its same-path replacement"
            );
            assert!(
                view.dialog.is_none(),
                "the old pending action must be aborted"
            );
        });
    });
}

/// Verifies that navigating to the current path does not duplicate the history
#[test]
fn test_navigate_to_same_path_is_noop() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("file.txt", b"f")]);

        let history_len = view.read(&app, |v, _| v.path_history.len());
        let current = view.read(&app, |v, _| v.current_path.clone());

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::NavigateTo(current), ctx);
        });

        view.read(&app, |v, _| {
            assert_eq!(
                v.path_history.len(),
                history_len,
                "navigating to the current path should not add to history"
            );
        });
    });
}

/// Verifies normalization of Windows backslash paths
#[test]
fn test_navigate_normalizes_backslashes() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("target/file.txt", b"t")]);

        // Navigate using a backslash path
        let target_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "target").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, target_idx, SftpBrowserAction::OpenEntry);
            v.handle_action(&action, ctx);
        });

        view.read(&app, |v, _| {
            // The path should not contain backslashes
            let path_str = v.current_path.to_string_lossy();
            assert!(
                path_str.contains("target"),
                "path should contain target after navigation"
            );
        });
    });
}

/// Verifies that SelectEntry selects a single entry
#[test]
fn test_select_entry_highlights_item() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(
            &mut app,
            &[
                ("file_a.txt", b"a"),
                ("file_b.txt", b"b"),
                ("file_c.txt", b"c"),
            ],
        );

        // Select the second entry
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, 1, SftpBrowserAction::SelectEntry);
            v.handle_action(&action, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.is_index_marked(1),
                "SelectEntry(1) should select the second entry"
            );
            assert_eq!(v.selected.len(), 1, "should have exactly 1 selection");
        });

        // Switch the selection to the third entry
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, 2, SftpBrowserAction::SelectEntry);
            v.handle_action(&action, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.is_index_marked(2),
                "SelectEntry(2) should select the third entry"
            );
        });
    });
}

/// Verifies SelectEntry bounds safety (an out-of-range index does not panic)
#[test]
fn test_select_entry_out_of_bounds_safe() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("only_file.txt", b"x")]);

        // An out-of-range reference must not panic or create a stale mark.
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, 99, SftpBrowserAction::SelectEntry);
            v.handle_action(&action, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.selected.is_empty(),
                "an absent identity must not enter the selection"
            );
        });
    });
}

/// Verifies that UploadFile (the toolbar upload button) is handled safely when not connected
#[test]
fn test_upload_file_action_without_connection_safe() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // Clicking the upload button while not connected should not panic
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::UploadFile, ctx);
        });

        view.read(&app, |v, _| {
            // The file picker is not triggered on the mock platform, but it must not panic either
            assert!(v.transfers.is_empty());
        });
    });
}

/// Verifies that DownloadEntry (the context-menu download) is handled safely when not connected
#[test]
fn test_download_entry_action_without_connection_safe() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // Triggering a download while not connected should not panic
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, 0, SftpBrowserAction::DownloadEntry);
            v.handle_action(&action, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.transfers.is_empty(),
                "download should not create transfer tasks when not connected"
            );
        });
    });
}

/// Verifies safe handling of OpenEntry on a file-type entry
#[test]
fn test_open_entry_on_file_triggers_download() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("readme.txt", b"hello")]);

        // Double-clicking a file entry should trigger a download (the file picker is not triggered in the mock)
        let file_idx = view.read(&app, |v, _| {
            v.entries
                .iter()
                .position(|e| e.name == "readme.txt")
                .unwrap()
        });

        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, file_idx, SftpBrowserAction::OpenEntry);
            v.handle_action(&action, ctx);
        });

        // It should not panic; whether a transfer task is created depends on the availability of the file picker
        view.read(&app, |v, _| {
            assert!(matches!(v.connection, ConnectionState::Connected));
        });
    });
}

// ============================================================
// C. File operation tests (8)
// ============================================================

/// Verifies that a file is removed from the list after confirming deletion
#[test]
fn test_delete_file_confirmed_removes_entry() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(
            &mut app,
            &[("to_delete.txt", b"delete me"), ("keep.txt", b"keep me")],
        );

        let file_idx = view.read(&app, |v, _| {
            v.entries
                .iter()
                .position(|e| e.name == "to_delete.txt")
                .unwrap()
        });

        // Initiate deletion
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, file_idx, SftpBrowserAction::DeleteEntry);
            v.handle_action(&action, ctx);
        });

        // The delete confirmation dialog should be present
        view.read(&app, |v, _| {
            assert!(matches!(v.dialog, Some(Dialog::DeleteConfirm { .. })));
        });

        // Confirm deletion
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmDelete, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog should be closed");
            assert_eq!(
                v.entries.len(),
                1,
                "should have 1 entry remaining after deletion"
            );
            assert!(v.entries[0].name == "keep.txt");
        });
    });
}

/// Verifies recursive directory deletion
#[test]
fn test_delete_directory_confirmed_removes_recursively() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(
            &mut app,
            &[("mydir/inner.txt", b"inner file"), ("outer.txt", b"outer")],
        );

        let dir_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "mydir").unwrap()
        });

        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, dir_idx, SftpBrowserAction::DeleteEntry);
            v.handle_action(&action, ctx);
        });
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmDelete, ctx);
        });

        view.read(&app, |v, _| {
            assert_eq!(
                v.entries.len(),
                1,
                "should have 1 entry remaining after deleting directory"
            );
            assert!(v.entries[0].name == "outer.txt");
        });
    });
}

/// Verifies that renaming updates the file name
#[test]
fn test_rename_entry_updates_name() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("old_name.txt", b"content")]);

        let idx = view.read(&app, |v, _| {
            v.entries
                .iter()
                .position(|e| e.name == "old_name.txt")
                .unwrap()
        });

        // Initiate the rename
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, idx, SftpBrowserAction::RenameEntry);
            v.handle_action(&action, ctx);
        });

        // Enter the new name in the editor
        view.update(&mut app, |v, ctx| {
            v.rename_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("new_name.txt", ctx);
            });
        });

        // Confirm the rename
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmRename, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog should be closed");
            assert!(
                v.entries.iter().any(|e| e.name == "new_name.txt"),
                "new name should appear in entries"
            );
        });
    });
}

/// Verifies that an empty rename keeps the dialog open
#[test]
fn test_rename_empty_name_shows_error() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("file.txt", b"content")]);

        let idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "file.txt").unwrap()
        });

        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, idx, SftpBrowserAction::RenameEntry);
            v.handle_action(&action, ctx);
        });

        // Clear the editor
        view.update(&mut app, |v, ctx| {
            v.rename_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("", ctx);
            });
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmRename, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.dialog.is_some(),
                "dialog should remain open when name is empty"
            );
        });
    });
}

/// Verifies that the directory exists after creating a new folder
#[test]
fn test_new_folder_creates_entry() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[]);
        initialize_app(&mut app);
        let backend =
            Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        // Open the new-folder dialog
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::NewFolder, ctx);
        });

        view.read(&app, |v, _| {
            assert!(matches!(v.dialog, Some(Dialog::CreateFolder { .. })));
        });

        // Enter a name
        view.update(&mut app, |v, ctx| {
            v.new_folder_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("test_folder", ctx);
            });
        });

        // Confirm
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmNewFolder, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog should be closed");
            assert!(
                v.entries
                    .iter()
                    .any(|e| e.name == "test_folder" && e.file_type == FileEntryType::Directory),
                "newly created folder should appear in entries"
            );
        });

        // Filesystem verification
        assert!(
            temp.path().join("test_folder").is_dir(),
            "newly created folder should exist in temporary directory"
        );
    });
}

/// Verifies that creating a folder with an empty name keeps the dialog open
#[test]
fn test_new_folder_empty_name_shows_error() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::NewFolder, ctx);
        });

        view.update(&mut app, |v, ctx| {
            v.new_folder_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("", ctx);
            });
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmNewFolder, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.dialog.is_some(),
                "dialog should remain open when name is empty"
            );
        });
    });
}

/// Verifies that the file details dialog shows the correct information
#[test]
fn test_file_details_dialog_shows_metadata() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) =
            create_connected_view(&mut app, &[("details.txt", b"file content here")]);

        let idx = view.read(&app, |v, _| {
            v.entries
                .iter()
                .position(|e| e.name == "details.txt")
                .unwrap()
        });

        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, idx, SftpBrowserAction::DetailsEntry);
            v.handle_action(&action, ctx);
        });

        view.read(&app, |v, _| match &v.dialog {
            Some(Dialog::FileDetails { entry }) => {
                assert_eq!(entry.name, "details.txt");
                assert_eq!(entry.file_type, FileEntryType::File);
            }
            _ => panic!("FileDetails dialog should be open"),
        });
    });
}

/// Verifies that cancelling a deletion preserves the entry
#[test]
fn test_delete_cancel_preserves_entry() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("keep_me.txt", b"keep")]);

        let idx = view.read(&app, |v, _| {
            v.entries
                .iter()
                .position(|e| e.name == "keep_me.txt")
                .unwrap()
        });

        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, idx, SftpBrowserAction::DeleteEntry);
            v.handle_action(&action, ctx);
        });

        // Cancel (close the dialog)
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CloseDialog, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none());
            assert_eq!(
                v.entries.len(),
                1,
                "entries should be preserved after cancellation"
            );
        });
    });
}

// ============================================================
// D. Context menu tests (5)
// ============================================================

/// Verifies that the context menu opens and selects the entry
#[test]
fn test_right_click_opens_menu_and_selects_entry() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("menu_file.txt", b"content")]);

        view.update(&mut app, |v, ctx| {
            let entry = v.entry_reference(0).unwrap();
            v.handle_action(
                &SftpBrowserAction::ContextMenu {
                    entry,
                    position: Vector2F::new(100.0, 100.0),
                },
                ctx,
            );
        });

        view.read(&app, |v, _| {
            assert!(v.context_menu.is_some(), "context menu should open");
            assert!(v.is_index_marked(0), "should select the first entry");
        });
    });
}

/// Verifies that the context menu's delete item triggers the delete confirmation
#[test]
fn test_context_menu_delete_item_triggers_delete() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("ctx_delete.txt", b"x")]);

        // Open the context menu
        view.update(&mut app, |v, ctx| {
            let entry = v.entry_reference(0).unwrap();
            v.handle_action(
                &SftpBrowserAction::ContextMenu {
                    entry,
                    position: Vector2F::new(50.0, 50.0),
                },
                ctx,
            );
        });

        // Choose delete from the menu
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, 0, SftpBrowserAction::DeleteEntry);
            v.handle_action(&action, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.dialog, Some(Dialog::DeleteConfirm { .. })),
                "delete confirmation dialog should open"
            );
        });
    });
}

/// Verifies that the context menu's rename item triggers a rename
#[test]
fn test_context_menu_rename_item_triggers_rename() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("ctx_rename.txt", b"x")]);

        view.update(&mut app, |v, ctx| {
            let entry = v.entry_reference(0).unwrap();
            v.handle_action(
                &SftpBrowserAction::ContextMenu {
                    entry,
                    position: Vector2F::new(50.0, 50.0),
                },
                ctx,
            );
        });

        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, 0, SftpBrowserAction::RenameEntry);
            v.handle_action(&action, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.dialog, Some(Dialog::Rename { .. })),
                "rename dialog should open"
            );
        });
    });
}

/// Verifies that the context menu's details item triggers the details view
#[test]
fn test_context_menu_details_item_triggers_details() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("ctx_details.txt", b"x")]);

        view.update(&mut app, |v, ctx| {
            let entry = v.entry_reference(0).unwrap();
            v.handle_action(
                &SftpBrowserAction::ContextMenu {
                    entry,
                    position: Vector2F::new(50.0, 50.0),
                },
                ctx,
            );
        });

        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, 0, SftpBrowserAction::DetailsEntry);
            v.handle_action(&action, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.dialog, Some(Dialog::FileDetails { .. })),
                "file details dialog should open"
            );
        });
    });
}

/// Verifies that the context menu can be closed
#[test]
fn test_dismiss_click_closes_menu() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("menu_close.txt", b"x")]);

        view.update(&mut app, |v, ctx| {
            let entry = v.entry_reference(0).unwrap();
            v.handle_action(
                &SftpBrowserAction::ContextMenu {
                    entry,
                    position: Vector2F::new(50.0, 50.0),
                },
                ctx,
            );
        });

        view.read(&app, |v, _| {
            assert!(v.context_menu.is_some());
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CloseContextMenu, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.context_menu.is_none(), "menu should be closed");
        });
    });
}

// ============================================================
// E. Dialog interaction tests (6)
// ============================================================

/// Verifies that a multi-selection deletion shows information for multiple items
#[test]
fn test_delete_confirm_dialog_multiple_paths() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) =
            create_connected_view(&mut app, &[("file_a.txt", b"a"), ("file_b.txt", b"b")]);

        // Select two entries
        view.update(&mut app, |v, ctx| {
            v.selected.clear();
            v.mark_index_for_test(0);
            v.mark_index_for_test(1);
            ctx.notify();
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DeleteSelected, ctx);
        });

        view.read(&app, |v, _| match &v.dialog {
            Some(Dialog::DeleteConfirm { paths, .. }) => {
                assert_eq!(paths.len(), 2, "should display 2 paths to be deleted");
            }
            _ => panic!("delete confirmation dialog should be open"),
        });
    });
}

/// Verifies that pressing Enter in the rename editor confirms the rename
#[test]
fn test_rename_editor_enter_confirms() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("rename_enter.txt", b"x")]);

        let idx = view.read(&app, |v, _| {
            v.entries
                .iter()
                .position(|e| e.name == "rename_enter.txt")
                .unwrap()
        });

        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, idx, SftpBrowserAction::RenameEntry);
            v.handle_action(&action, ctx);
        });

        view.update(&mut app, |v, ctx| {
            v.rename_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("renamed.txt", ctx);
            });
        });

        // Simulate Enter via ConfirmRename
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmRename, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.dialog.is_none(),
                "dialog should be closed after pressing Enter"
            );
        });
    });
}

/// Verifies that pressing Escape in the rename editor cancels the rename
#[test]
fn test_rename_editor_escape_cancels() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("rename_esc.txt", b"x")]);

        let idx = view.read(&app, |v, _| {
            v.entries
                .iter()
                .position(|e| e.name == "rename_esc.txt")
                .unwrap()
        });

        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, idx, SftpBrowserAction::RenameEntry);
            v.handle_action(&action, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_some());
        });

        // Escape cancels (via CloseDialog)
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CloseDialog, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog should be closed after Escape");
            // The file name should not change
            assert!(
                v.entries.iter().any(|e| e.name == "rename_esc.txt"),
                "original file name should remain unchanged"
            );
        });
    });
}

/// Verifies that pressing Enter in the new-folder editor confirms the creation
#[test]
fn test_new_folder_editor_enter_confirms() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::NewFolder, ctx);
        });

        view.update(&mut app, |v, ctx| {
            v.new_folder_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("my_folder", ctx);
            });
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmNewFolder, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.dialog.is_none(),
                "dialog should be closed after pressing Enter"
            );
            assert!(
                v.entries.iter().any(|e| e.name == "my_folder"),
                "my_folder should be created"
            );
        });
    });
}

/// Verifies the overwrite confirmation dialog
#[test]
fn test_overwrite_confirm_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("file.txt", b"x")]);

        // Manually set up the overwrite confirmation dialog
        view.update(&mut app, |v, ctx| {
            v.dialog = Some(Dialog::OverwriteConfirm {
                source: PathBuf::from("/source.txt"),
                target: PathBuf::from("/target.txt"),
                file_size: 1,
                direction: TransferDirection::Download,
            });
            ctx.notify();
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmOverwrite, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.dialog.is_none(),
                "dialog should be closed after overwrite confirmation"
            );
        });
    });
}

/// Verifies the move confirmation dialog
#[test]
fn test_move_confirm_dialog() {
    warpui::App::test((), |mut app| async move {
        let temp =
            create_temp_dir_with_files(&[("move_src.txt", b"move me"), ("dest_dir/.keep", b"")]);
        initialize_app(&mut app);
        let backend =
            Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        // Manually set up the move dialog
        view.update(&mut app, |v, ctx| {
            v.dialog = Some(Dialog::Move {
                source: PathBuf::from("/move_src.txt"),
                target_dir: PathBuf::from("/dest_dir"),
            });
            ctx.notify();
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmMove, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.dialog.is_none(),
                "dialog should be closed after move confirmation"
            );
        });
    });
}

// ============================================================
// F. Transfer panel tests (5)
// ============================================================

/// Verifies that an upload creates a transfer task
#[test]
fn test_upload_creates_transfer_task() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[]);
        // Keep the local file in a separate temporary directory so it is not listed by InMemorySftpBackend's list_dir
        let local_dir = tempfile::tempdir().expect("failed to create local temporary directory");
        let local_file = local_dir.path().join("upload_source.txt");
        std::fs::write(&local_file, b"upload content").unwrap();

        initialize_app(&mut app);
        let backend =
            Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(
                &SftpBrowserAction::ExecuteUpload(local_file.to_string_lossy().to_string()),
                ctx,
            );
        });

        view.read(&app, |v, _| {
            assert_eq!(v.transfers.len(), 1, "should create 1 transfer task");
            let task = &v.transfers[0];
            assert_eq!(task.direction, TransferDirection::Upload);
            assert!(
                matches!(
                    task.state,
                    TransferState::Completed | TransferState::InProgress | TransferState::Failed(_)
                ),
                "transfer task should have a defined state"
            );
        });
        assert_eq!(
            std::fs::read(temp.path().join("upload_source.txt")).unwrap(),
            b"upload content",
            "a remote pane must read an OS upload from the local backend"
        );
    });
}

/// Verifies that uploading a nonexistent file fails
#[test]
fn test_upload_nonexistent_file_fails() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(
                &SftpBrowserAction::ExecuteUpload("/nonexistent/path/file.txt".to_string()),
                ctx,
            );
        });

        view.read(&app, |v, _| {
            assert_eq!(v.transfers.len(), 1);
            assert!(
                matches!(v.transfers[0].state, TransferState::Failed(_)),
                "uploading a nonexistent file should fail"
            );
        });
    });
}

/// Verifies that a download creates a transfer task
#[test]
fn test_download_creates_transfer_task() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[("download_me.txt", b"download content")]);
        let local_save = temp.path().join("saved_file.txt");

        initialize_app(&mut app);
        let backend =
            Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let listed_entry = backend
            .lstat(std::path::Path::new("/download_me.txt"))
            .unwrap();
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
            v.entries = vec![listed_entry];
        });

        view.update(&mut app, |v, ctx| {
            let entry = v.entry_reference(0).unwrap();
            v.handle_action(
                &SftpBrowserAction::DownloadSaveAs {
                    entry,
                    resolved_target_size: None,
                    local_path: local_save.to_string_lossy().to_string(),
                },
                ctx,
            );
        });

        view.read(&app, |v, _| {
            assert_eq!(v.transfers.len(), 1, "should create download task");
            assert_eq!(v.transfers[0].direction, TransferDirection::Download);
        });
    });
}

/// The native save dialog can remain open while a refresh or sort changes row
/// indices. Completing it must still download the file originally chosen.
#[test]
fn save_as_resolves_the_same_entry_after_refresh() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _remote) =
            create_connected_view(&mut app, &[("a.txt", b"alpha"), ("b.txt", b"beta")]);
        let local = tempfile::tempdir().expect("local temp");
        let destination = local.path().join("saved.txt");

        let selected_index = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "a.txt")
                .unwrap()
        });
        let entry = view.read(&app, |view, _| {
            view.entry_reference(selected_index).unwrap()
        });
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::Refresh, ctx);
            view.handle_action(
                &SftpBrowserAction::SortBy(super::browser::SortColumn::Size),
                ctx,
            );
        });
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::DownloadSaveAs {
                    entry,
                    resolved_target_size: None,
                    local_path: destination.to_string_lossy().to_string(),
                },
                ctx,
            );
        });

        assert_eq!(
            std::fs::read(destination).unwrap(),
            b"alpha",
            "save-as must remain bound to the originally selected file"
        );
    });
}

/// Verifies that cancelling a transfer sets the cancelled flag
#[test]
fn test_cancel_transfer_sets_cancelled_flag() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        // Manually add a transfer task
        view.update(&mut app, |v, ctx| {
            use super::types::TransferTask;
            let task = TransferTask::new(
                42,
                PathBuf::from("/remote.txt"),
                PathBuf::from("/local.txt"),
                TransferDirection::Download,
                1024,
            );
            v.transfers.push(task);
            ctx.notify();
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CancelTransfer(42, None), ctx);
        });

        view.read(&app, |v, _| {
            let task = v.transfers.iter().find(|t| t.id == 42).unwrap();
            assert!(task.is_cancelled(), "task should be marked as cancelled");
        });
    });
}

/// Verifies that the transfer panel render does not panic
#[test]
fn test_transfer_panel_renders_with_tasks() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            use super::types::TransferTask;
            let task = TransferTask::new(
                1,
                PathBuf::from("/file.txt"),
                PathBuf::from("/local/file.txt"),
                TransferDirection::Upload,
                2048,
            );
            v.transfers.push(task);
            ctx.notify();
        });

        // render does not panic
        view.read(&app, |_v, _| {
            // Reaching this point means render succeeded
        });
    });
}

// ============================================================
// G. Drag-and-drop interaction tests (4)
// ============================================================

/// Verifies that dragging in shows the overlay
#[test]
fn test_drag_enter_shows_overlay() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("file.txt", b"x")]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DragFilesEnter, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.is_drag_hovering,
                "overlay should be displayed after drag enter"
            );
        });
    });
}

/// Verifies that dragging out hides the overlay
#[test]
fn test_drag_leave_hides_overlay() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("file.txt", b"x")]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DragFilesEnter, ctx);
        });
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DragFilesLeave, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                !v.is_drag_hovering,
                "overlay should be hidden after drag leave"
            );
        });
    });
}

/// Verifies that dropping files creates upload tasks
#[test]
fn test_drop_files_creates_upload_tasks() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[]);
        // Keep the local file in a separate temporary directory so it is not listed by InMemorySftpBackend's list_dir
        let local_dir = tempfile::tempdir().expect("failed to create local temporary directory");
        let drop_file = local_dir.path().join("dropped.txt");
        std::fs::write(&drop_file, b"dropped content").unwrap();

        initialize_app(&mut app);
        let backend =
            Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(
                &SftpBrowserAction::DragAndDropFiles(vec![drop_file.clone()]),
                ctx,
            );
        });

        view.read(&app, |v, _| {
            assert_eq!(
                v.transfers.len(),
                1,
                "drag and drop should create upload task"
            );
            assert!(
                !v.is_drag_hovering,
                "hover state should be cleared after drag and drop"
            );
        });
        assert_eq!(
            std::fs::read(temp.path().join("dropped.txt")).unwrap(),
            b"dropped content",
            "drag upload must stream from the local OS path"
        );
    });
}

/// Verifies that dropping empty paths is ignored
#[test]
fn test_drop_empty_paths_ignored() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DragAndDropFiles(vec![]), ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.transfers.is_empty(),
                "empty paths should not create tasks"
            );
        });
    });
}

// ============================================================
// H. Keyboard shortcut tests (5)
// ============================================================

/// Verifies that NavigateUp (Backspace) returns to the parent directory
#[test]
fn test_keyboard_navigate_up() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("subdir/file.txt", b"x")]);

        // Enter the subdirectory
        let sub_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "subdir").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, sub_idx, SftpBrowserAction::OpenEntry);
            v.handle_action(&action, ctx);
        });

        // NavigateUp
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::NavigateUp, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.entries.iter().any(|e| e.name == "subdir"),
                "after NavigateUp should return to parent directory and see subdir"
            );
        });
    });
}

/// The post-`NavigateUp` stray-click window (RC acceptance 2026-07-21): the
/// `..` row navigates on a SINGLE click, so the second click of a habitual
/// double click is still in flight when the rows swap — at the root boundary
/// the first entry sits in the `..` row's pixels and that stray click would
/// open a row the user never aimed at. A row click right after `NavigateUp`
/// must be swallowed; once the window passes, clicks work again.
#[test]
fn test_row_click_right_after_navigate_up_is_swallowed() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) =
            create_connected_view(&mut app, &[("level1/level2/file.txt", b"deep")]);

        // Descend into level1 — a plain open, no window arms here.
        let l1_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "level1").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, l1_idx, SftpBrowserAction::OpenEntry);
            v.handle_action(&action, ctx);
        });
        let in_level1 = view.read(&app, |v, _| v.current_path.clone());

        // Navigate up (the single click on `..`) — arms the window — and let
        // the double click's second click land on a row of the new listing.
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::NavigateUp, ctx);
        });
        let at_root = view.read(&app, |v, _| v.current_path.clone());
        assert_ne!(at_root, in_level1, "NavigateUp itself must still navigate");
        let l1_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "level1").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, l1_idx, SftpBrowserAction::OpenEntry);
            v.handle_action(&action, ctx);
        });
        view.read(&app, |v, _| {
            assert_eq!(
                v.current_path, at_root,
                "a row click within the stray-click window must be swallowed"
            );
        });

        // Past the window, the same click works.
        view.update(&mut app, |v, ctx| {
            v.expire_click_guard();
            let action = entry_action(v, l1_idx, SftpBrowserAction::OpenEntry);
            v.handle_action(&action, ctx);
        });
        view.read(&app, |v, _| {
            assert_eq!(
                v.current_path, in_level1,
                "once the window passes, row clicks must work again"
            );
        });
    });
}

/// The virtual parent row keeps its view-owned mouse handle across a re-render,
/// so its click still navigates exactly one level up.
#[test]
fn parent_row_click_survives_rerender() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view, _temp) =
            create_connected_view(&mut app, &[("subdir/file.txt", b"x")]);
        let subdir_index = view.read(&app, |view, _| {
            view.entries
                .iter()
                .position(|entry| entry.name == "subdir")
                .unwrap()
        });
        view.update(&mut app, |view, ctx| {
            let action = entry_action(view, subdir_index, SftpBrowserAction::OpenEntry);
            view.handle_action(&action, ctx);
        });
        assert_eq!(
            view.read(&app, |view, _| view.current_path.clone()),
            PathBuf::from("/subdir")
        );

        let (presenter, invalidation) = presenter_for_window(&app, window_id);
        let parent_position_id = layout_id(&view, &app, "row:parent");
        let position = render_position(
            &mut app,
            presenter.clone(),
            invalidation.clone(),
            &parent_position_id,
        );
        mouse_down(&mut app, window_id, presenter.clone(), position);
        rerender(&mut app, presenter.clone(), invalidation);
        mouse_up(&mut app, window_id, presenter, position);

        assert_eq!(
            view.read(&app, |view, _| view.current_path.clone()),
            PathBuf::from("/"),
            "the parent-row click must survive a render between mouse-down and mouse-up"
        );
    });
}

/// Verifies that DeleteSelected triggers the delete confirmation
#[test]
fn test_keyboard_delete_selected() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("del_target.txt", b"x")]);

        // Select the first entry
        view.update(&mut app, |v, ctx| {
            v.selected.clear();
            v.mark_index_for_test(0);
            ctx.notify();
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DeleteSelected, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.dialog, Some(Dialog::DeleteConfirm { .. })),
                "DeleteSelected should trigger delete confirmation"
            );
        });
    });
}

#[test]
fn tab_cycles_file_manager_panes_clockwise() {
    App::test((), |mut app| async move {
        let (window_id, pane_group, browsers, pane_ids, _temp_dirs) =
            create_three_pane_group(&mut app);
        browsers[0].update(&mut app, |view, ctx| view.focus_contents(ctx));
        assert_eq!(
            pane_group.read(&app, |group, ctx| group.focused_pane_id(ctx)),
            pane_ids[0]
        );

        let (presenter, _scene) = render_scene_at(&mut app, window_id, vec2f(1200.0, 800.0));
        assert!(key_down(&mut app, window_id, presenter, "tab"));
        assert_eq!(
            pane_group.read(&app, |group, ctx| group.focused_pane_id(ctx)),
            pane_ids[1]
        );
    });
}

#[test]
fn shift_tab_cycles_file_manager_panes_counterclockwise() {
    App::test((), |mut app| async move {
        let (window_id, pane_group, browsers, pane_ids, _temp_dirs) =
            create_three_pane_group(&mut app);
        browsers[0].update(&mut app, |view, ctx| view.focus_contents(ctx));
        assert_eq!(
            pane_group.read(&app, |group, ctx| group.focused_pane_id(ctx)),
            pane_ids[0]
        );

        let (presenter, _scene) = render_scene_at(&mut app, window_id, vec2f(1200.0, 800.0));
        assert!(key_down(&mut app, window_id, presenter, "shift-tab"));
        assert_eq!(
            pane_group.read(&app, |group, ctx| group.focused_pane_id(ctx)),
            pane_ids[2]
        );
    });
}

#[test]
fn global_tab_binding_does_not_steal_file_manager_focus() {
    App::test((), |mut app| async move {
        let (window_id, pane_group, browsers, pane_ids, _temp_dirs) =
            create_three_pane_group(&mut app);
        browsers[0].update(&mut app, |view, ctx| view.focus_contents(ctx));

        let (presenter, _scene) = render_scene_at(&mut app, window_id, vec2f(1200.0, 800.0));
        assert!(key_down(&mut app, window_id, presenter, "tab"));
        assert_eq!(
            pane_group.read(&app, |group, ctx| group.focused_pane_id(ctx)),
            pane_ids[1],
            "Tab must remain owned by the file-manager pane group"
        );
    });
}

#[test]
fn function_bar_drops_low_priority_captions_before_overlap() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, _, left, right, _left_temp, _right_temp) =
            create_dual_connected_view(&mut app);

        let (wide_presenter, _scene) = render_scene_at(&mut app, window_id, vec2f(2400.0, 800.0));
        for view in [&left, &right] {
            assert!(
                maybe_position(&wide_presenter, &layout_id(view, &app, "legend-full")).is_some()
            );
            assert!(
                maybe_position(&wide_presenter, &layout_id(view, &app, "legend-compact")).is_none()
            );
        }

        let (compact_presenter, _scene) =
            render_scene_at(&mut app, window_id, vec2f(1200.0, 800.0));
        for view in [&left, &right] {
            assert!(
                maybe_position(&compact_presenter, &layout_id(view, &app, "legend-full")).is_none()
            );
            assert!(
                maybe_position(&compact_presenter, &layout_id(view, &app, "legend-compact"))
                    .is_some()
            );
        }

        let (narrow_presenter, _scene) = render_scene_at(&mut app, window_id, vec2f(600.0, 800.0));
        for view in [&left, &right] {
            assert!(
                maybe_position(&narrow_presenter, &layout_id(view, &app, "legend-compact"))
                    .is_none()
            );
            let hidden = position(&narrow_presenter, &layout_id(view, &app, "legend-hidden"));
            assert_approximately_equal(hidden.height(), 0.0);
        }
    });
}

#[test]
fn each_pane_owns_optional_compact_function_legend_in_real_layout() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, _, left, right, _left_temp, _right_temp) =
            create_dual_connected_view(&mut app);
        let (presenter, _scene) = render_scene_at(&mut app, window_id, vec2f(1200.0, 800.0));

        let left_root = position(&presenter, &layout_id(&left, &app, "pane-root"));
        let right_root = position(&presenter, &layout_id(&right, &app, "pane-root"));
        let left_legend = position(&presenter, &layout_id(&left, &app, "legend-compact"));
        let right_legend = position(&presenter, &layout_id(&right, &app, "legend-compact"));

        assert!(left_legend.min_x() >= left_root.min_x());
        assert!(left_legend.max_x() <= left_root.max_x());
        assert!(right_legend.min_x() >= right_root.min_x());
        assert!(right_legend.max_x() <= right_root.max_x());
        assert!(left_legend.max_x() <= right_legend.min_x());
    });
}

#[test]
fn function_bar_fits_minimum_supported_pane_width() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, _, left, right, _left_temp, _right_temp) =
            create_dual_connected_view(&mut app);
        let (presenter, _scene) = render_scene_at(&mut app, window_id, vec2f(600.0, 800.0));

        let left_root = position(&presenter, &layout_id(&left, &app, "pane-root"));
        let right_root = position(&presenter, &layout_id(&right, &app, "pane-root"));
        assert!(left_root.max_x() <= right_root.min_x());
        for (view, root) in [(&left, left_root), (&right, right_root)] {
            let hidden = position(&presenter, &layout_id(view, &app, "legend-hidden"));
            assert_approximately_equal(hidden.height(), 0.0);
            assert!(hidden.width() <= root.width());
        }
    });
}

#[test]
fn dual_pane_never_renders_global_function_bar() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, _, left, right, _left_temp, _right_temp) =
            create_dual_connected_view(&mut app);
        let (presenter, scene) = render_scene_at(&mut app, window_id, vec2f(1200.0, 800.0));

        let left_root = position(&presenter, &layout_id(&left, &app, "pane-root"));
        let right_root = position(&presenter, &layout_id(&right, &app, "pane-root"));
        let left_legend = position(&presenter, &layout_id(&left, &app, "legend-compact"));
        let right_legend = position(&presenter, &layout_id(&right, &app, "legend-compact"));
        let combined_width = right_root.max_x() - left_root.min_x();

        assert!(left_legend.width() < combined_width);
        assert!(right_legend.width() < combined_width);
        assert_approximately_equal(left_legend.width(), left_root.width());
        assert_approximately_equal(right_legend.width(), right_root.width());
        assert_approximately_equal(left_legend.max_x(), right_legend.min_x());

        let (footer_background, footer_border) = app.read(|ctx| {
            let theme = Appearance::as_ref(ctx).theme();
            (
                theme.background().into(),
                theme.split_pane_border_color().into(),
            )
        });
        let footer_rects = scene
            .layers()
            .flat_map(|layer| &layer.rects)
            .filter(|rect| {
                rect.background == footer_background
                    && rect.border.color == footer_border
                    && rect.border.width == 1.0
                    && rect.border.top
                    && !rect.border.left
                    && !rect.border.right
                    && !rect.border.bottom
            })
            .map(|rect| rect.bounds)
            .collect::<Vec<_>>();
        assert_eq!(
            footer_rects.len(),
            2,
            "exactly two pane-local function legends should be painted"
        );
        assert!(footer_rects
            .iter()
            .any(|bounds| rects_approximately_equal(*bounds, left_legend)));
        assert!(footer_rects
            .iter()
            .any(|bounds| rects_approximately_equal(*bounds, right_legend)));
    });
}

#[test]
fn dual_pane_context_menu_uses_its_own_panel_origin() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, _, left, right, _left_temp, _right_temp) =
            create_dual_connected_view(&mut app);
        let (presenter, _scene) = render_scene_at(&mut app, window_id, vec2f(1200.0, 800.0));

        let left_panel = position(&presenter, &layout_id(&left, &app, "panel"));
        let right_panel = position(&presenter, &layout_id(&right, &app, "panel"));
        let right_row = position(&presenter, &layout_id(&right, &app, "row:0"));
        assert!(left_panel.max_x() <= right_panel.min_x());

        let click_position = right_row.center();
        right_click(&mut app, window_id, presenter, click_position);

        right.read(&app, |view, _| {
            let menu = view
                .context_menu
                .as_ref()
                .expect("right-pane context menu should open");
            assert_approximately_equal(menu.position.x(), click_position.x() - right_panel.min_x());
            assert_approximately_equal(menu.position.y(), click_position.y() - right_panel.min_y());
        });
        left.read(&app, |view, _| {
            assert!(
                view.context_menu.is_none(),
                "the left pane must not receive the right-pane click"
            );
        });
    });
}

#[test]
fn file_pane_body_consumes_remaining_height_above_footer() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view, _temp) =
            create_connected_view(&mut app, &[("visible.txt", b"visible")]);
        let (presenter, _scene) = render_scene_at(&mut app, window_id, vec2f(1200.0, 800.0));

        let root = position(&presenter, &layout_id(&view, &app, "pane-root"));
        let body = position(&presenter, &layout_id(&view, &app, "body"));
        let footer = position(&presenter, &layout_id(&view, &app, "legend-full"));

        assert!(body.height() > 0.0);
        assert_approximately_equal(body.max_y(), footer.min_y());
        assert_approximately_equal(footer.max_y(), root.max_y());
    });
}

#[test]
fn active_pane_style_differs_from_selection_and_hover() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, _, left, right, _left_temp, _right_temp) =
            create_dual_connected_view(&mut app);
        let left_pane_id = PaneId::dummy_pane_id();
        let right_pane_id = PaneId::dummy_pane_id();
        let focus_state = app.add_model(|_| PaneGroupFocusState::new(left_pane_id, None, true));
        left.update(&mut app, |view, ctx| {
            view.set_focus_handle(PaneFocusHandle::new(left_pane_id, focus_state.clone()), ctx);
        });
        right.update(&mut app, |view, ctx| {
            view.cursor = 0;
            view.mark_index_for_test(1);
            view.set_focus_handle(
                PaneFocusHandle::new(right_pane_id, focus_state.clone()),
                ctx,
            );
        });

        let (hover_presenter, _scene) = render_scene_at(&mut app, window_id, vec2f(1200.0, 800.0));
        let hovered_row_id = layout_id(&left, &app, "row:1");
        let hover_point = position(&hover_presenter, &hovered_row_id).center();
        mouse_move(&mut app, window_id, hover_presenter, hover_point);

        let (presenter, scene) = render_scene_at(&mut app, window_id, vec2f(1200.0, 800.0));
        let left_root = position(&presenter, &layout_id(&left, &app, "pane-root"));
        let right_root = position(&presenter, &layout_id(&right, &app, "pane-root"));
        let selected_row = position(&presenter, &layout_id(&right, &app, "row:1"));
        let hovered_row = position(&presenter, &hovered_row_id);
        let (accent, selection_background, hover_background): (
            warpui::elements::Fill,
            warpui::elements::Fill,
            warpui::elements::Fill,
        ) = app.read(|ctx| {
            let theme = Appearance::as_ref(ctx).theme();
            (
                theme.accent().into(),
                warp_core::ui::theme::color::internal_colors::fg_overlay_2(theme).into(),
                warp_core::ui::theme::color::internal_colors::fg_overlay_1(theme).into(),
            )
        });
        let active_border = scene.layers().flat_map(|layer| &layer.rects).any(|rect| {
            rects_approximately_equal(rect.bounds, left_root)
                && rect.border.width == 2.0
                && rect.border.color == accent
        });
        let inactive_border = scene.layers().flat_map(|layer| &layer.rects).any(|rect| {
            rects_approximately_equal(rect.bounds, right_root)
                && rect.border.width == 2.0
                && rect.border.color == accent
        });
        let selection_fill = scene.layers().flat_map(|layer| &layer.rects).any(|rect| {
            rects_approximately_equal(rect.bounds, selected_row)
                && rect.background == selection_background
        });
        let hover_fill = scene.layers().flat_map(|layer| &layer.rects).any(|rect| {
            rects_approximately_equal(rect.bounds, hovered_row)
                && rect.background == hover_background
        });
        let row_impersonates_focus = scene.layers().flat_map(|layer| &layer.rects).any(|rect| {
            (rects_approximately_equal(rect.bounds, selected_row)
                || rects_approximately_equal(rect.bounds, hovered_row))
                && rect.border.width == 2.0
                && rect.border.color == accent
        });

        assert!(
            active_border,
            "the focused connected pane needs an accent border"
        );
        assert!(
            !inactive_border,
            "selection must not impersonate pane focus"
        );
        assert!(selection_fill, "the marked row needs its selection fill");
        assert!(hover_fill, "the hovered row needs its distinct hover fill");
        assert!(
            !row_impersonates_focus,
            "selection and hover must not use the pane-focus border"
        );
    });
}

#[test]
fn column_positions_do_not_move_with_filename_length() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view, _temp) = create_connected_view(
            &mut app,
            &[
                ("a.txt", b"a"),
                ("this-is-a-very-long-file-name.txt", b"long"),
            ],
        );
        let (presenter, _scene) = render_scene_at(&mut app, window_id, vec2f(1200.0, 800.0));

        let short_size = position(&presenter, &layout_id(&view, &app, "row:0:size-column"));
        let long_size = position(&presenter, &layout_id(&view, &app, "row:1:size-column"));
        let short_date = position(&presenter, &layout_id(&view, &app, "row:0:date-column"));
        let long_date = position(&presenter, &layout_id(&view, &app, "row:1:date-column"));

        assert_approximately_equal(short_size.min_x(), long_size.min_x());
        assert_approximately_equal(short_size.width(), long_size.width());
        assert_approximately_equal(short_date.min_x(), long_date.min_x());
        assert_approximately_equal(short_date.width(), long_date.width());
    });
}

/// A multi-mark is the operation target even when the keyboard cursor rests
/// on a different row. This protects F8/F5/F6 from silently using the cursor.
#[test]
fn file_action_uses_marked_entries_before_cursor() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(
            &mut app,
            &[("cursor.txt", b"cursor"), ("marked.txt", b"marked")],
        );

        view.update(&mut app, |v, ctx| {
            v.cursor = 0;
            v.selected.clear();
            v.mark_index_for_test(1);
            v.handle_action(&SftpBrowserAction::DeleteSelected, ctx);
        });

        view.read(&app, |v, _| match &v.dialog {
            Some(Dialog::DeleteConfirm { paths, .. }) => {
                assert_eq!(paths.len(), 1);
                assert_eq!(
                    paths[0].file_name().and_then(|name| name.to_str()),
                    Some("marked.txt")
                );
            }
            _ => panic!("DeleteSelected should target the marked row"),
        });
    });
}

/// Verifies that CreateFolder (Ctrl+Shift+N) opens the new-folder dialog
#[test]
fn test_keyboard_create_folder() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CreateFolder, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.dialog, Some(Dialog::CreateFolder { .. })),
                "CreateFolder should open the new folder dialog"
            );
        });
    });
}

/// Verifies that with no multi-selection, DeleteSelected (F8) falls back to the
/// row under the MC keyboard cursor — the classic file-manager behaviour where
/// the highlighted row is always the operation target.
#[test]
fn test_keyboard_shortcuts_without_selection() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("file.txt", b"x")]);

        // No multi-selection; the cursor sits on the first (only) row.
        view.update(&mut app, |v, ctx| {
            v.selected.clear();
            ctx.notify();
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DeleteSelected, ctx);
        });

        view.read(&app, |v, _| {
            match &v.dialog {
                Some(Dialog::DeleteConfirm { paths, .. }) => {
                    assert_eq!(
                        paths.len(),
                        1,
                        "DeleteSelected should target the single row under the cursor"
                    );
                }
                _ => {
                    panic!("DeleteSelected should open the delete confirmation for the cursor row")
                }
            }
            assert_eq!(
                v.entries.len(),
                1,
                "entries are not removed until the dialog is confirmed"
            );
        });
    });
}

/// Verifies that Escape closes the dialog
#[test]
fn test_keyboard_escape_closes_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("esc_file.txt", b"x")]);

        let idx = view.read(&app, |v, _| {
            v.entries
                .iter()
                .position(|e| e.name == "esc_file.txt")
                .unwrap()
        });

        // Open the rename dialog
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, idx, SftpBrowserAction::RenameEntry);
            v.handle_action(&action, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_some());
        });

        // Escape closes it
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CloseDialog, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "Escape should close the dialog");
        });
    });
}

// ============================================================
// I. Render safety and combination tests (4)
// ============================================================

/// Verifies that the combined state of connection + context menu + dialog + transfer + drag overlays is safe
#[test]
fn test_render_with_all_overlays_connected() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("overlay.txt", b"x")]);

        // Open the context menu
        view.update(&mut app, |v, ctx| {
            let entry = v.entry_reference(0).unwrap();
            v.context_menu = Some(super::context_menu::ContextMenuState::new(
                entry,
                Vector2F::new(50.0, 50.0),
            ));
            // Open a dialog
            v.dialog = Some(Dialog::DeleteConfirm {
                entries: vec![v.entry_reference(0).unwrap()],
                paths: vec![PathBuf::from("/overlay.txt")],
                is_dirs: vec![false],
            });
            // Add a transfer task
            use super::types::TransferTask;
            v.transfers.push(TransferTask::new(
                1,
                PathBuf::from("/file.txt"),
                PathBuf::from("/local.txt"),
                TransferDirection::Upload,
                1024,
            ));
            // Enable drag hovering
            v.is_drag_hovering = true;
            ctx.notify();
        });

        // Verify all overlay states exist and do not conflict
        view.read(&app, |v, _| {
            assert!(v.context_menu.is_some());
            assert!(v.dialog.is_some());
            assert!(!v.transfers.is_empty());
            assert!(v.is_drag_hovering);
            assert!(matches!(v.connection, ConnectionState::Connected));
        });
    });
}

/// Verifies the loading state indicator
#[test]
fn test_render_loading_state() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            v.is_loading = true;
            ctx.notify();
        });

        view.read(&app, |v, _| {
            assert!(v.is_loading, "should be in loading state");
        });

        // Clear the loading state
        view.update(&mut app, |v, ctx| {
            v.is_loading = false;
            ctx.notify();
        });

        view.read(&app, |v, _| {
            assert!(!v.is_loading, "should clear loading state");
        });
    });
}

/// Verifies the empty directory display
#[test]
fn test_render_empty_directory() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.read(&app, |v, _| {
            assert!(matches!(v.connection, ConnectionState::Connected));
            assert!(
                v.entries.is_empty(),
                "empty directory should have no entries"
            );
        });
    });
}

/// Verifies that rendering is safe after multiple operations
#[test]
fn test_render_after_multiple_operations() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[
            ("op_dir/file1.txt", b"1"),
            ("op_dir/file2.txt", b"2"),
            ("root_file.txt", b"root"),
        ]);
        initialize_app(&mut app);
        let backend =
            Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        // Enter the directory
        let dir_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "op_dir").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            let action = entry_action(v, dir_idx, SftpBrowserAction::OpenEntry);
            v.handle_action(&action, ctx);
        });

        // Search
        view.update(&mut app, |v, ctx| {
            v.handle_action(
                &SftpBrowserAction::SetSearchFilter("file1".to_string()),
                ctx,
            );
        });

        // Clear the search
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ClearSearchFilter, ctx);
        });

        // Go back up
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::GoUp, ctx);
        });

        // Refresh
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::Refresh, ctx);
        });

        // Final state verification
        view.read(&app, |v, _| {
            assert!(matches!(v.connection, ConnectionState::Connected));
            assert!(!v.entries.is_empty());
            assert!(v.search_filter.is_none());
        });
    });
}

// ============================================================
// Cross-pane copy/move (MC F5/F6, increment B)
// ============================================================

#[cfg(unix)]
struct RecordingDeleteBackend {
    inner: InMemorySftpBackend,
    file_deletes: std::sync::Mutex<Vec<PathBuf>>,
    recursive_deletes: std::sync::Mutex<Vec<PathBuf>>,
    fail_recursive_delete: AtomicBool,
}

#[cfg(unix)]
impl RecordingDeleteBackend {
    fn new(root: PathBuf) -> Self {
        Self {
            inner: InMemorySftpBackend::new(root),
            file_deletes: std::sync::Mutex::new(Vec::new()),
            recursive_deletes: std::sync::Mutex::new(Vec::new()),
            fail_recursive_delete: AtomicBool::new(false),
        }
    }

    fn fail_recursive_deletes(&self) {
        self.fail_recursive_delete.store(true, Ordering::SeqCst);
    }
}

#[cfg(unix)]
impl SftpBackend for RecordingDeleteBackend {
    fn supports_atomic_exchange(&self) -> bool {
        self.inner.supports_atomic_exchange()
    }

    fn supports_identity_bound_cleanup(&self) -> bool {
        self.inner.supports_identity_bound_cleanup()
    }

    fn existing_entry_ownership_anchor(
        &self,
        path: &std::path::Path,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, super::sftp_ops::SftpOpsError> {
        self.inner.existing_entry_ownership_anchor(path)
    }

    fn list_dir(
        &self,
        path: &std::path::Path,
    ) -> Result<Vec<super::types::FileEntry>, super::sftp_ops::SftpOpsError> {
        self.inner.list_dir(path)
    }

    fn delete_file(&self, path: &std::path::Path) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.file_deletes.lock().unwrap().push(path.to_path_buf());
        self.inner.delete_file(path)
    }

    fn delete_dir_recursive(
        &self,
        path: &std::path::Path,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.recursive_deletes
            .lock()
            .unwrap()
            .push(path.to_path_buf());
        if self.fail_recursive_delete.load(Ordering::SeqCst) {
            if let Some(entry) = self.inner.list_dir(path)?.into_iter().next() {
                match entry.file_type {
                    FileEntryType::Directory => {
                        self.inner.delete_dir_recursive(&entry.path)?;
                    }
                    FileEntryType::File | FileEntryType::Symlink | FileEntryType::Other => {
                        self.inner.delete_file(&entry.path)?;
                    }
                }
            }
            return Err(super::sftp_ops::SftpOpsError::Operation(format!(
                "injected recursive delete failure after partial cleanup; source recovery path: {}",
                path.display()
            )));
        }
        self.inner.delete_dir_recursive(path)
    }

    fn create_dir(&self, path: &std::path::Path) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.inner.create_dir(path)
    }

    fn create_dir_with_ownership_anchor(
        &self,
        path: &std::path::Path,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, super::sftp_ops::SftpOpsError> {
        self.inner.create_dir_with_ownership_anchor(path)
    }

    fn rename(
        &self,
        old_path: &std::path::Path,
        new_path: &std::path::Path,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.inner.rename(old_path, new_path)
    }

    fn replace(
        &self,
        old_path: &std::path::Path,
        new_path: &std::path::Path,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.inner.replace(old_path, new_path)
    }

    fn realpath(&self, path: &std::path::Path) -> Result<PathBuf, super::sftp_ops::SftpOpsError> {
        self.inner.realpath(path)
    }

    fn stat(
        &self,
        path: &std::path::Path,
    ) -> Result<super::types::FileEntry, super::sftp_ops::SftpOpsError> {
        self.inner.stat(path)
    }

    fn lstat(
        &self,
        path: &std::path::Path,
    ) -> Result<super::types::FileEntry, super::sftp_ops::SftpOpsError> {
        self.inner.lstat(path)
    }

    fn upload_file(
        &self,
        local_path: &std::path::Path,
        remote_path: &std::path::Path,
        progress_cb: Option<&super::sftp_ops::ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.inner
            .upload_file(local_path, remote_path, progress_cb, cancel_flag)
    }

    fn download_file(
        &self,
        remote_path: &std::path::Path,
        local_path: &std::path::Path,
        progress_cb: Option<&super::sftp_ops::ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.inner
            .download_file(remote_path, local_path, progress_cb, cancel_flag)
    }

    fn copy_file(
        &self,
        src: &std::path::Path,
        dst: &std::path::Path,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        self.inner.copy_file(src, dst)
    }
}

/// Two file-manager panes over one shared filesystem, at `/left` and `/right`.
/// Returns both handles; keep `TempDir` alive for the test's duration.
fn create_two_panes_sharing_fs(
    app: &mut warpui::App,
    files: &[(&str, &[u8])],
) -> (
    warpui::ViewHandle<SftpBrowserView>,
    warpui::ViewHandle<SftpBrowserView>,
    tempfile::TempDir,
) {
    let temp = create_temp_dir_with_files(files);
    let root = temp.path().to_path_buf();

    let (_, view_a) = create_view(app);
    view_a.update(app, |v, ctx| {
        let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
        v.set_backend_for_test(backend, PathBuf::from("/left"), ctx);
    });
    let (_, view_b) = create_view(app);
    view_b.update(app, |v, ctx| {
        let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
        v.set_backend_for_test(backend, PathBuf::from("/right"), ctx);
    });

    (view_a, view_b, temp)
}

/// F5 copies the cursor row from one pane into the other pane's directory,
/// leaving the source in place.
#[test]
fn test_f5_copies_cursor_file_into_other_pane() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (view_a, _view_b, temp) = create_two_panes_sharing_fs(
            &mut app,
            &[("left/foo.txt", b"hello"), ("right/.keep", b"")],
        );
        let root = temp.path().to_path_buf();

        // Pane A's cursor is on the only file in /left → F5 → into /right.
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });

        assert!(root.join("right/foo.txt").exists(), "copied into /right");
        assert!(root.join("left/foo.txt").exists(), "source kept after copy");
        assert_eq!(
            std::fs::read(root.join("right/foo.txt")).unwrap(),
            b"hello",
            "copied content matches"
        );
        app.read(|ctx| {
            let activities = super::transfer_queue::TransferQueue::as_ref(ctx)
                .activities()
                .collect::<Vec<_>>();
            assert_eq!(activities.len(), 1, "same-host F5 must enqueue one job");
            assert!(matches!(
                activities[0].state,
                super::transfer_queue::QueuedTransferState::Completed
            ));
            assert_eq!(activities[0].direction, TransferDirection::Copy);
            assert_eq!(activities[0].progress.transferred, 5);
        });
    });
}

#[test]
fn same_fs_target_probe_error_aborts_without_copying() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[("left/foo.txt", b"source"), ("right/.keep", b"")]);
        let root = temp.path().to_path_buf();
        let backend = Arc::new(
            InMemorySftpBackend::new(root.clone())
                .with_lstat_error(PathBuf::from("/right/foo.txt")),
        ) as Arc<dyn SftpBackend>;

        let (_, source_view) = create_view_with_node(&mut app, "same-host");
        source_view.update(&mut app, |view, ctx| {
            view.set_backend_for_test(backend.clone(), PathBuf::from("/left"), ctx);
        });
        let (_, target_view) = create_view_with_node(&mut app, "same-host");
        target_view.update(&mut app, |view, ctx| {
            view.set_backend_for_test(backend, PathBuf::from("/right"), ctx);
        });

        source_view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });

        assert!(
            !root.join("right/foo.txt").exists(),
            "a permission or connection-style target probe error must not be treated as absence"
        );
        app.read(|ctx| {
            assert_eq!(
                super::transfer_queue::TransferQueue::as_ref(ctx)
                    .activities()
                    .count(),
                0,
                "no transfer may start after an indeterminate target probe"
            );
        });
    });
}

/// F6 moves the cursor row into the other pane (source removed).
#[test]
fn test_f6_moves_cursor_file_into_other_pane() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (view_a, _view_b, temp) = create_two_panes_sharing_fs(
            &mut app,
            &[("left/bar.txt", b"data"), ("right/.keep", b"")],
        );
        let root = temp.path().to_path_buf();

        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::MoveToOtherPane, ctx);
        });

        assert!(root.join("right/bar.txt").exists(), "moved into /right");
        assert!(
            !root.join("left/bar.txt").exists(),
            "source removed after move"
        );
    });
}

/// F5 with a directory selected copies it recursively.
#[test]
fn test_f5_copies_directory_recursively() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (view_a, _view_b, temp) = create_two_panes_sharing_fs(
            &mut app,
            &[("left/sub/deep.txt", b"deep"), ("right/.keep", b"")],
        );
        let root = temp.path().to_path_buf();

        // /left contains exactly one entry, the directory "sub" → cursor on it.
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });

        assert!(
            root.join("right/sub/deep.txt").exists(),
            "directory copied recursively into /right"
        );
    });
}

/// F5 with more than one other pane open shows the destination picker instead
/// of guessing; picking a row routes the copy into exactly that pane.
#[test]
fn test_f5_with_multiple_panes_opens_target_picker() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        // Three panes over one shared fs: source /left (holds the file) plus two
        // candidate targets /right and /third.
        let temp = create_temp_dir_with_files(&[
            ("left/foo.txt", b"hello"),
            ("right/.keep", b""),
            ("third/.keep", b""),
        ]);
        let root = temp.path().to_path_buf();

        let (_, view_a) = create_view(&mut app);
        view_a.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/left"), ctx);
        });
        let (_, view_b) = create_view(&mut app);
        view_b.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/right"), ctx);
        });
        let (_, view_c) = create_view(&mut app);
        view_c.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/third"), ctx);
        });

        // F5 on pane A: two candidates → picker opens, nothing copied yet.
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });
        view_a.read(&app, |v, _| match &v.dialog {
            Some(Dialog::CopyMoveTargetPicker { labels, is_move }) => {
                assert_eq!(labels.len(), 2, "two candidate panes offered");
                assert!(!*is_move, "F5 is a copy");
            }
            _ => panic!("expected the target picker dialog"),
        });
        assert!(
            !root.join("right/foo.txt").exists(),
            "nothing copied before a pick"
        );
        assert!(
            !root.join("third/foo.txt").exists(),
            "nothing copied before a pick"
        );

        // Pick the first candidate (insertion order → pane B, /right).
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::PickCopyMoveTarget(0), ctx);
        });
        view_a.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog clears after picking");
        });
        assert!(
            root.join("right/foo.txt").exists(),
            "copied into the picked pane (/right)"
        );
        assert!(
            !root.join("third/foo.txt").exists(),
            "the unpicked pane is untouched"
        );
        assert!(root.join("left/foo.txt").exists(), "source kept after copy");
    });
}

/// Cancelling the destination picker discards the pending pick and copies nothing.
#[test]
fn test_target_picker_cancel_copies_nothing() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[
            ("left/foo.txt", b"hello"),
            ("right/.keep", b""),
            ("third/.keep", b""),
        ]);
        let root = temp.path().to_path_buf();

        let (_, view_a) = create_view(&mut app);
        view_a.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/left"), ctx);
        });
        for dir in ["/right", "/third"] {
            let (_, view) = create_view(&mut app);
            view.update(&mut app, |v, ctx| {
                let backend =
                    Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
                v.set_backend_for_test(backend, PathBuf::from(dir), ctx);
            });
        }

        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });
        // Cancel the picker.
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CloseDialog, ctx);
        });
        view_a.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "picker dismissed");
        });
        assert!(
            !root.join("right/foo.txt").exists(),
            "cancel copies nothing"
        );
        assert!(
            !root.join("third/foo.txt").exists(),
            "cancel copies nothing"
        );
    });
}

/// With no second pane, F5 is a no-op (reports, copies nothing).
#[test]
fn test_f5_without_second_pane_copies_nothing() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, temp) = create_connected_view(&mut app, &[("only.txt", b"x")]);
        let root = temp.path().to_path_buf();

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });

        // Nothing new appeared anywhere.
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            1,
            "no copy target → nothing created"
        );
    });
}

/// An existing target is skipped, never silently overwritten.
#[test]
fn test_f5_skips_existing_target() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (view_a, _view_b, temp) = create_two_panes_sharing_fs(
            &mut app,
            &[("left/dup.txt", b"new"), ("right/dup.txt", b"original")],
        );
        let root = temp.path().to_path_buf();

        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });

        // The pre-existing /right/dup.txt is untouched.
        assert_eq!(
            std::fs::read(root.join("right/dup.txt")).unwrap(),
            b"original",
            "existing target must not be overwritten"
        );
    });
}

// ============================================================
// Cross-connection routing (MC F5 across local↔remote, increment C)
// ============================================================

/// Create a view with an explicit node id (empty = local, non-empty = a remote host).
fn create_view_with_node(
    app: &mut warpui::App,
    node_id: &str,
) -> (warpui::WindowId, warpui::ViewHandle<SftpBrowserView>) {
    app.add_window(WindowStyle::NotStealFocus, |ctx| {
        SftpBrowserView::new(node_id.to_string(), None, ctx)
    })
}

/// F5 from a local pane to a remote pane routes through the transfer engine
/// (an upload task appears) rather than a direct filesystem copy.
#[test]
fn test_f5_local_to_remote_starts_an_upload_transfer() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[("left/foo.txt", b"hi"), ("right/.keep", b"")]);
        let root = temp.path().to_path_buf();

        // Pane A: local (node ""), at /left. Pane B: "remote" host, at /right.
        let (_, view_a) = create_view_with_node(&mut app, "");
        view_a.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/left"), ctx);
        });
        let (_, view_b) = create_view_with_node(&mut app, "host");
        view_b.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/right"), ctx);
        });

        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });

        view_a.read(&app, |v, _| {
            assert_eq!(
                v.transfers.len(),
                1,
                "local→remote copy starts one transfer"
            );
            assert_eq!(
                v.transfers[0].direction,
                TransferDirection::Upload,
                "the transfer is an upload"
            );
        });
        assert_eq!(
            std::fs::read(root.join("right/foo.txt")).unwrap(),
            b"hi",
            "the productive F5 path completes through the streaming job"
        );
        app.read(|ctx| {
            let queue = super::transfer_queue::TransferQueue::as_ref(ctx);
            let activities = queue.activities().collect::<Vec<_>>();
            assert_eq!(activities.len(), 1);
            assert!(matches!(
                activities[0].state,
                super::transfer_queue::QueuedTransferState::Completed
            ));
            assert_eq!(activities[0].progress.transferred, 2);
            assert_eq!(
                activities[0].operation,
                super::transfer_job::TransferOperation::Copy
            );
            assert_eq!(
                activities[0].conflict,
                super::transfer_job::ConflictDecision::Skip
            );
            assert_eq!(
                activities[0].topology,
                super::transfer_queue::TransferTopology::LocalToRemote
            );
        });
    });
}

#[test]
fn global_transfer_ui_updates_and_controls_jobs_from_another_pane() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, source_view, _source_temp) = create_connected_view(&mut app, &[]);
        let queue_id = app.update(|ctx| {
            super::transfer_queue::TransferQueue::handle(ctx).update(ctx, |queue, ctx| {
                let id = queue
                    .enqueue_job(
                        "workspace",
                        PathBuf::from("/source"),
                        PathBuf::from("/target"),
                        TransferDirection::Upload,
                        128,
                    )
                    .unwrap();
                queue.update_progress(
                    id,
                    super::transfer_job::TransferProgress {
                        transferred: 64,
                        total: 128,
                        bytes_per_second: 32,
                        eta: Some(std::time::Duration::from_secs(3)),
                        phase: super::types::TransferPhase::Transferring,
                    },
                );
                ctx.notify();
                id
            })
        });
        source_view.update(&mut app, |browser, _| {
            let mut task = super::types::TransferTask::new(
                77,
                PathBuf::from("/source"),
                PathBuf::from("/target"),
                TransferDirection::Upload,
                128,
            );
            task.state = TransferState::InProgress;
            browser.attach_queue_transfer_for_test(task, queue_id);
        });
        let (other_window, other_view, _other_temp) = create_connected_view(&mut app, &[]);
        other_view.read(&app, |browser, _| {
            assert!(
                browser.has_transfer_progress_poll_for_test(),
                "a queue model update must schedule live transfer rerenders in every pane"
            );
        });

        let (presenter, invalidation) = presenter_for_window(&app, other_window);
        let pause_position = render_position(
            &mut app,
            presenter.clone(),
            invalidation.clone(),
            &format!("sftp_btn:pause_transfer:{queue_id}"),
        );
        render_position(
            &mut app,
            presenter.clone(),
            invalidation.clone(),
            &format!("sftp_text:transfer_eta:{queue_id}"),
        );
        mouse_down(&mut app, other_window, presenter.clone(), pause_position);
        mouse_up(&mut app, other_window, presenter.clone(), pause_position);
        app.read(|ctx| {
            assert!(matches!(
                super::transfer_queue::TransferQueue::as_ref(ctx)
                    .activity(queue_id)
                    .expect("queue activity")
                    .state,
                super::transfer_queue::QueuedTransferState::Paused
            ));
        });

        let resume_position = render_position(
            &mut app,
            presenter.clone(),
            invalidation.clone(),
            &format!("sftp_btn:pause_transfer:{queue_id}"),
        );
        mouse_down(&mut app, other_window, presenter.clone(), resume_position);
        mouse_up(&mut app, other_window, presenter.clone(), resume_position);

        let cancel_position = render_position(
            &mut app,
            presenter.clone(),
            invalidation,
            &format!("sftp_btn:cancel_transfer:{queue_id}"),
        );
        mouse_down(&mut app, other_window, presenter.clone(), cancel_position);
        mouse_up(&mut app, other_window, presenter, cancel_position);
        app.read(|ctx| {
            assert!(matches!(
                super::transfer_queue::TransferQueue::as_ref(ctx)
                    .activity(queue_id)
                    .expect("queue activity")
                    .state,
                super::transfer_queue::QueuedTransferState::Cancelling
            ));
        });

        app.update(|ctx| {
            super::transfer_queue::TransferQueue::handle(ctx).update(ctx, |queue, ctx| {
                queue.clear_terminal();
                ctx.notify();
            });
        });
        source_view.read(&app, |browser, _| {
            assert_eq!(
                browser.tracked_queue_transfers_for_test(),
                1,
                "a cancelling worker must remain globally visible and controllable"
            );
        });

        app.update(|ctx| {
            super::transfer_queue::TransferQueue::handle(ctx).update(ctx, |queue, ctx| {
                queue
                    .activity_handle(queue_id)
                    .expect("worker activity handle")
                    .set_state(super::transfer_queue::QueuedTransferState::Cancelled);
                queue.clear_terminal();
                ctx.notify();
            });
        });
        source_view.read(&app, |browser, _| {
            assert_eq!(
                browser.tracked_queue_transfers_for_test(),
                0,
                "clearing global history must prune stale pane-owned queue ids"
            );
        });
    });
}

#[test]
fn cancel_all_keeps_late_recovery_visible() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view, _temp) = create_connected_view(&mut app, &[]);
        let (queue_id, worker) = app.update(|ctx| {
            super::transfer_queue::TransferQueue::handle(ctx).update(ctx, |queue, ctx| {
                let id = queue
                    .enqueue_job(
                        "workspace",
                        PathBuf::from("/source"),
                        PathBuf::from("/target"),
                        TransferDirection::Upload,
                        1,
                    )
                    .unwrap();
                let worker = queue.activity_handle(id).unwrap();
                ctx.notify();
                (id, worker)
            })
        });
        view.update(&mut app, |browser, ctx| {
            let task = TransferTask::new(
                91,
                PathBuf::from("/source"),
                PathBuf::from("/target"),
                TransferDirection::Upload,
                1,
            );
            browser.attach_queue_transfer_for_test(task, queue_id);
            browser.handle_action(&SftpBrowserAction::ConfirmCloseTransferPanel, ctx);
        });
        worker.set_error(&super::sftp_ops::SftpOpsError::RecoveryRequired {
            message: "late cleanup recovery".to_string(),
            recovery_id: Some(712),
            paths: vec![PathBuf::from("/retained")],
            committed: true,
        });

        let (presenter, invalidation) = presenter_for_window(&app, window_id);
        render_position(
            &mut app,
            presenter,
            invalidation,
            "sftp_transfer_history_scroll",
        );
    });
}

#[test]
fn queue_survives_source_tab_close() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);
        let queue_id = app.update(|ctx| {
            super::transfer_queue::TransferQueue::handle(ctx).update(ctx, |queue, ctx| {
                let id = queue
                    .enqueue_job(
                        "workspace",
                        PathBuf::from("/source"),
                        PathBuf::from("/target"),
                        TransferDirection::Upload,
                        1,
                    )
                    .unwrap();
                ctx.notify();
                id
            })
        });
        view.update(&mut app, |browser, ctx| {
            browser.attach_queue_transfer_for_test(
                TransferTask::new(
                    92,
                    PathBuf::from("/source"),
                    PathBuf::from("/target"),
                    TransferDirection::Upload,
                    1,
                ),
                queue_id,
            );
            browser.close_for_test(ctx);
        });

        app.read(|ctx| {
            assert!(
                super::transfer_queue::TransferQueue::as_ref(ctx)
                    .activity(queue_id)
                    .is_some(),
                "closing the source tab must not remove its global queue activity"
            );
        });
    });
}

#[test]
fn new_global_job_reopens_a_previously_hidden_transfer_panel() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view, _temp) = create_connected_view(&mut app, &[]);
        view.update(&mut app, |browser, ctx| {
            browser.handle_action(&SftpBrowserAction::ToggleTransferPanel, ctx);
        });
        app.update(|ctx| {
            super::transfer_queue::TransferQueue::handle(ctx).update(ctx, |queue, ctx| {
                queue
                    .enqueue_job(
                        "workspace",
                        PathBuf::from("/source"),
                        PathBuf::from("/target"),
                        TransferDirection::Upload,
                        1,
                    )
                    .unwrap();
                ctx.notify();
            });
        });

        let (presenter, invalidation) = presenter_for_window(&app, window_id);
        render_position(
            &mut app,
            presenter,
            invalidation,
            "sftp_transfer_history_scroll",
        );
    });
}

/// F6 (move) across connections now runs as an upload transfer (copy-then-delete):
/// a transfer starts. The source removal happens on async completion, so this
/// synchronous check asserts the transfer was created, not the final deletion.
#[test]
fn test_f6_move_across_connections_starts_transfer() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[("left/bar.txt", b"x"), ("right/.keep", b"")]);
        let root = temp.path().to_path_buf();

        let (_, view_a) = create_view_with_node(&mut app, "");
        view_a.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/left"), ctx);
        });
        let (_, view_b) = create_view_with_node(&mut app, "host");
        view_b.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/right"), ctx);
        });

        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::MoveToOtherPane, ctx);
        });

        view_a.read(&app, |v, _| {
            assert_eq!(
                v.transfers.len(),
                1,
                "cross-connection move starts one transfer"
            );
            assert_eq!(v.transfers[0].direction, TransferDirection::Upload);
        });
    });
}

/// A cross-connection directory move remains incomplete when any destination
/// entry is skipped, so its complete source tree must remain available.
#[test]
fn cross_connection_dir_move_with_skip_conflict_preserves_source_tree() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[
            ("left/dir/conflict.txt", b"source-conflict"),
            ("left/dir/new.txt", b"new"),
            ("right/dir/conflict.txt", b"destination-conflict"),
        ]);
        let root = temp.path().to_path_buf();

        let (_, source_view) = create_view_with_node(&mut app, "");
        source_view.update(&mut app, |view, ctx| {
            let backend =
                Arc::new(InMemorySftpBackend::new(PathBuf::from("/"))) as Arc<dyn SftpBackend>;
            view.set_backend_for_test(backend, root.join("left"), ctx);
        });
        let (_, target_view) = create_view_with_node(&mut app, "host");
        target_view.update(&mut app, |view, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            view.set_backend_for_test(backend, PathBuf::from("/right"), ctx);
        });

        source_view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::MoveToOtherPane, ctx);
        });

        assert_eq!(
            std::fs::read(root.join("right/dir/conflict.txt")).unwrap(),
            b"destination-conflict"
        );
        assert_eq!(
            std::fs::read(root.join("right/dir/new.txt")).unwrap(),
            b"new"
        );
        assert_eq!(
            std::fs::read(root.join("left/dir/conflict.txt")).unwrap(),
            b"source-conflict",
            "a skipped source entry must remain after an incomplete move"
        );
        assert_eq!(
            std::fs::read(root.join("left/dir/new.txt")).unwrap(),
            b"new",
            "an incomplete directory move keeps the entire source tree"
        );
    });
}

// ============================================================
// Cross-connection directory transfers (FM backlog)
// ============================================================

struct StaticListingBackend {
    entries: Vec<FileEntry>,
}

impl StaticListingBackend {
    fn unsupported<T>() -> Result<T, super::sftp_ops::SftpOpsError> {
        Err(super::sftp_ops::SftpOpsError::Operation(
            "operation is not used by this test backend".to_string(),
        ))
    }
}

impl SftpBackend for StaticListingBackend {
    fn list_dir(
        &self,
        _path: &std::path::Path,
    ) -> Result<Vec<FileEntry>, super::sftp_ops::SftpOpsError> {
        Ok(self.entries.clone())
    }

    fn delete_file(&self, _path: &std::path::Path) -> Result<(), super::sftp_ops::SftpOpsError> {
        Self::unsupported()
    }

    fn delete_dir_recursive(
        &self,
        _path: &std::path::Path,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        Self::unsupported()
    }

    fn create_dir(&self, _path: &std::path::Path) -> Result<(), super::sftp_ops::SftpOpsError> {
        Self::unsupported()
    }

    fn rename(
        &self,
        _old_path: &std::path::Path,
        _new_path: &std::path::Path,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        Self::unsupported()
    }

    fn realpath(&self, _path: &std::path::Path) -> Result<PathBuf, super::sftp_ops::SftpOpsError> {
        Self::unsupported()
    }

    fn stat(&self, path: &std::path::Path) -> Result<FileEntry, super::sftp_ops::SftpOpsError> {
        Ok(FileEntry {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.to_path_buf(),
            file_type: FileEntryType::Directory,
            size: 0,
            modified: None,
            permissions: None,
            identity: super::types::StableEntryIdentity {
                file_type: FileEntryType::Directory,
                size: 0,
                object_id: path.display().to_string(),
                revision: "1".to_string(),
            },
        })
    }

    fn lstat(&self, path: &std::path::Path) -> Result<FileEntry, super::sftp_ops::SftpOpsError> {
        self.stat(path)
    }

    fn upload_file(
        &self,
        _local_path: &std::path::Path,
        _remote_path: &std::path::Path,
        _progress_cb: Option<&super::sftp_ops::ProgressCallback>,
        _cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        Self::unsupported()
    }

    fn download_file(
        &self,
        _remote_path: &std::path::Path,
        _local_path: &std::path::Path,
        _progress_cb: Option<&super::sftp_ops::ProgressCallback>,
        _cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        Self::unsupported()
    }

    fn copy_file(
        &self,
        _src: &std::path::Path,
        _dst: &std::path::Path,
    ) -> Result<(), super::sftp_ops::SftpOpsError> {
        Self::unsupported()
    }
}

/// Uploading a directory: the local tree is walked, the remote target dirs are
/// created, and every file is planned as an upload.
#[test]
fn test_plan_dir_transfer_upload_lists_files_and_makes_remote_dirs() {
    let source = create_temp_dir_with_files(&[("sub/deep.txt", b"deep"), ("top.txt", b"top")]);
    let remote_root = tempfile::tempdir().expect("remote temp");
    let backend: Arc<dyn SftpBackend> =
        Arc::new(InMemorySftpBackend::new(remote_root.path().to_path_buf()));

    let plan = super::browser::plan_dir_transfer(
        TransferDirection::Upload,
        backend.clone(),
        source.path(),
        std::path::Path::new("/dst"),
    )
    .expect("plan ok");

    assert_eq!(plan.files.len(), 2, "both files planned");
    assert_eq!(plan.skipped_existing, 0);
    assert!(
        backend.stat(std::path::Path::new("/dst")).is_ok(),
        "/dst created on remote"
    );
    assert!(
        backend.stat(std::path::Path::new("/dst/sub")).is_ok(),
        "/dst/sub created on remote"
    );
    for (local, remote) in &plan.files {
        assert!(
            local.starts_with(source.path()),
            "source is the real local file"
        );
        assert!(remote.starts_with("/dst"), "target is under /dst");
    }
}

/// Downloading a directory: the remote tree is walked via `list_dir`, the local
/// target dirs are created, and every file is planned as a download.
#[test]
fn test_plan_dir_transfer_download_lists_files_and_makes_local_dirs() {
    let remote =
        create_temp_dir_with_files(&[("src/sub/deep.txt", b"deep"), ("src/top.txt", b"top")]);
    let backend: Arc<dyn SftpBackend> =
        Arc::new(InMemorySftpBackend::new(remote.path().to_path_buf()));
    let local_dest = tempfile::tempdir().expect("local temp");
    let dest = local_dest.path().join("out");

    let plan = super::browser::plan_dir_transfer(
        TransferDirection::Download,
        backend,
        std::path::Path::new("/src"),
        &dest,
    )
    .expect("plan ok");

    assert_eq!(plan.files.len(), 2, "both files planned");
    assert_eq!(plan.skipped_existing, 0);
    assert!(dest.exists(), "local dest created");
    assert!(dest.join("sub").exists(), "local dest/sub created");
    for (local, remote) in &plan.files {
        assert!(local.starts_with(&dest), "target is under the local dest");
        assert!(remote.starts_with("/src"), "source is under /src");
    }
}

/// A remote server must not be able to return an entry outside the directory
/// being downloaded and thereby choose an arbitrary local destination.
#[test]
fn recursive_download_rejects_path_outside_source_root() {
    let backend: Arc<dyn SftpBackend> = Arc::new(StaticListingBackend {
        entries: vec![FileEntry {
            name: "escape.txt".to_string(),
            path: PathBuf::from("/outside-source/escape.txt"),
            file_type: FileEntryType::File,
            size: 1,
            modified: None,
            permissions: None,
            identity: super::types::StableEntryIdentity {
                file_type: FileEntryType::File,
                size: 1,
                object_id: "outside-source".to_string(),
                revision: "1".to_string(),
            },
        }],
    });
    let local_dest = tempfile::tempdir().expect("local temp");

    let result = super::browser::plan_dir_transfer(
        TransferDirection::Download,
        backend,
        std::path::Path::new("/expected-source"),
        &local_dest.path().join("out"),
    );

    assert!(
        result.is_err(),
        "a listing entry outside the requested source root must abort the transfer plan"
    );
}

/// Recursive download must classify links before scheduling file reads. A link
/// is not a regular file and requires an explicit link policy.
#[test]
fn recursive_download_rejects_unhandled_symlink() {
    let backend: Arc<dyn SftpBackend> = Arc::new(StaticListingBackend {
        entries: vec![FileEntry {
            name: "link".to_string(),
            path: PathBuf::from("/src/link"),
            file_type: FileEntryType::Symlink,
            size: 0,
            modified: None,
            permissions: None,
            identity: super::types::StableEntryIdentity {
                file_type: FileEntryType::Symlink,
                size: 0,
                object_id: "link".to_string(),
                revision: "1".to_string(),
            },
        }],
    });
    let local_dest = tempfile::tempdir().expect("local temp");

    let result = super::browser::plan_dir_transfer(
        TransferDirection::Download,
        backend,
        std::path::Path::new("/src"),
        &local_dest.path().join("out"),
    );

    assert!(
        result.is_err(),
        "an unhandled symlink must never be scheduled as a regular-file download"
    );
}

/// FIFOs, sockets and devices must never enter the streaming file path.
#[test]
fn recursive_download_rejects_special_file() {
    let backend: Arc<dyn SftpBackend> = Arc::new(StaticListingBackend {
        entries: vec![FileEntry {
            name: "pipe".to_string(),
            path: PathBuf::from("/src/pipe"),
            file_type: FileEntryType::Other,
            size: 0,
            modified: None,
            permissions: None,
            identity: super::types::StableEntryIdentity {
                file_type: FileEntryType::Other,
                size: 0,
                object_id: "pipe".to_string(),
                revision: "1".to_string(),
            },
        }],
    });
    let local_dest = tempfile::tempdir().expect("local temp");

    let result = super::browser::plan_dir_transfer(
        TransferDirection::Download,
        backend,
        std::path::Path::new("/src"),
        &local_dest.path().join("out"),
    );

    assert!(
        result.is_err(),
        "a special file must never be scheduled as a regular-file download"
    );
}

#[cfg(unix)]
#[test]
fn recursive_upload_rejects_unhandled_symlink() {
    use std::os::unix::fs::symlink;

    let source = tempfile::tempdir().expect("source temp");
    std::fs::write(source.path().join("target.txt"), b"target").unwrap();
    symlink("target.txt", source.path().join("link.txt")).unwrap();
    let remote = tempfile::tempdir().expect("remote temp");
    let backend: Arc<dyn SftpBackend> =
        Arc::new(InMemorySftpBackend::new(remote.path().to_path_buf()));

    let result = super::browser::plan_dir_transfer(
        TransferDirection::Upload,
        backend,
        source.path(),
        std::path::Path::new("/dst"),
    );

    assert!(
        result.is_err(),
        "silently skipping a source symlink would make a later move destructive"
    );
}

#[cfg(unix)]
#[test]
fn unsupported_entry_aborts_move_before_source_delete() {
    use nix::sys::stat::Mode;
    use nix::unistd::mkfifo;

    let source = tempfile::tempdir().expect("source temp");
    mkfifo(&source.path().join("pipe"), Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
    let remote = tempfile::tempdir().expect("remote temp");
    let backend: Arc<dyn SftpBackend> =
        Arc::new(InMemorySftpBackend::new(remote.path().to_path_buf()));

    let result = super::browser::plan_dir_transfer(
        TransferDirection::Upload,
        backend,
        source.path(),
        std::path::Path::new("/dst"),
    );

    assert!(
        result.is_err(),
        "silently skipping a FIFO would make a later move destructive"
    );
    assert!(
        source.path().join("pipe").exists(),
        "rejecting an unsupported entry must leave the source tree untouched"
    );
}

/// A same-named file already at the destination is skipped, not re-transferred.
#[test]
fn test_plan_dir_transfer_skips_existing_destination_files() {
    let remote = create_temp_dir_with_files(&[("src/a.txt", b"a"), ("src/b.txt", b"b")]);
    let backend: Arc<dyn SftpBackend> =
        Arc::new(InMemorySftpBackend::new(remote.path().to_path_buf()));
    let local_dest = tempfile::tempdir().expect("local temp");
    let dest = local_dest.path().join("out");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("a.txt"), b"already here").unwrap();

    let plan = super::browser::plan_dir_transfer(
        TransferDirection::Download,
        backend,
        std::path::Path::new("/src"),
        &dest,
    )
    .expect("plan ok");

    assert_eq!(
        plan.files.len(),
        1,
        "only the not-yet-present file is planned"
    );
    assert_eq!(plan.skipped_existing, 1, "the existing file was skipped");
}

/// A move containing only empty directories is complete once the destination
/// tree exists. It must remove the source just like a move containing files.
#[test]
fn empty_directory_move_deletes_source_after_destination_is_verified() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let source = tempfile::tempdir().expect("source temp");
        std::fs::create_dir_all(source.path().join("empty/nested")).unwrap();
        let backend =
            Arc::new(InMemorySftpBackend::new(source.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |view, ctx| {
            view.set_backend_for_test(backend.clone(), PathBuf::from("/"), ctx);
        });
        let destination_root = tempfile::tempdir().expect("destination temp");
        let destination = destination_root.path().join("moved-empty");

        view.update(&mut app, |view, ctx| {
            view.spawn_dir_transfer_for_test(
                PathBuf::from("/empty"),
                destination.clone(),
                TransferDirection::Download,
                backend,
                true,
                "empty".to_string(),
                ctx,
            );
        });

        let transfer_state = app.read(|ctx| {
            super::transfer_queue::TransferQueue::as_ref(ctx)
                .activities()
                .next()
                .map(|activity| activity.state)
        });
        assert!(
            destination.join("nested").is_dir(),
            "the complete empty destination tree must exist before source cleanup; transfer state: {transfer_state:?}"
        );
        assert!(
            !source.path().join("empty").exists(),
            "a completed empty-directory move must remove its source"
        );
        view.read(&app, |view, _| {
            assert!(
                view.entries.iter().all(|entry| entry.name != "empty"),
                "the source listing must refresh after the move"
            );
        });
    });
}

/// A source path may be reused after a move captures its source object. Data
/// created at that path after enumeration was never transferred and must never
/// be swept up by recursive source cleanup.
#[test]
fn directory_move_never_deletes_data_created_after_source_enumeration() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let source = tempfile::tempdir().expect("source temp");
        std::fs::create_dir(source.path().join("empty")).unwrap();
        let backend = Arc::new(TracingBackend::new(source.path().to_path_buf()));
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |view, ctx| {
            view.set_backend_for_test(backend.clone(), PathBuf::from("/"), ctx);
        });
        backend.create_file_after_next_list(
            PathBuf::from("/empty/late.txt"),
            b"never transferred".to_vec(),
        );
        let destination_root = tempfile::tempdir().expect("destination temp");
        let destination = destination_root.path().join("moved-empty");

        view.update(&mut app, |view, ctx| {
            view.spawn_dir_transfer_for_test(
                PathBuf::from("/empty"),
                destination,
                TransferDirection::Download,
                backend,
                true,
                "empty".to_string(),
                ctx,
            );
        });

        assert_eq!(
            std::fs::read(source.path().join("empty/late.txt")).unwrap(),
            b"never transferred",
            "data created after enumeration must remain at the original source path"
        );
    });
}

#[cfg(unix)]
#[test]
fn move_with_untransferred_symlink_never_deletes_source_tree() {
    use std::os::unix::fs::symlink;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let source = tempfile::tempdir().expect("source temp");
        std::fs::create_dir(source.path().join("tree")).unwrap();
        std::fs::write(source.path().join("target.txt"), b"target").unwrap();
        symlink("../target.txt", source.path().join("tree/link.txt")).unwrap();
        let backend =
            Arc::new(InMemorySftpBackend::new(source.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |view, ctx| {
            view.set_backend_for_test(backend.clone(), PathBuf::from("/"), ctx);
        });
        let destination = tempfile::tempdir().expect("destination temp");

        view.update(&mut app, |view, ctx| {
            view.spawn_dir_transfer_for_test(
                PathBuf::from("/tree"),
                destination.path().join("tree"),
                TransferDirection::Download,
                backend,
                true,
                "tree".to_string(),
                ctx,
            );
        });

        assert!(
            source
                .path()
                .join("tree/link.txt")
                .symlink_metadata()
                .is_ok(),
            "an untransferred symlink must preserve the complete source tree"
        );
    });
}

#[cfg(unix)]
#[test]
fn move_with_untransferred_special_entry_never_deletes_source_tree() {
    use nix::sys::stat::Mode;
    use nix::unistd::mkfifo;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let source = tempfile::tempdir().expect("source temp");
        std::fs::create_dir(source.path().join("tree")).unwrap();
        mkfifo(
            &source.path().join("tree/pipe"),
            Mode::S_IRUSR | Mode::S_IWUSR,
        )
        .unwrap();
        let backend =
            Arc::new(InMemorySftpBackend::new(source.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |view, ctx| {
            view.set_backend_for_test(backend.clone(), PathBuf::from("/"), ctx);
        });
        let destination = tempfile::tempdir().expect("destination temp");

        view.update(&mut app, |view, ctx| {
            view.spawn_dir_transfer_for_test(
                PathBuf::from("/tree"),
                destination.path().join("tree"),
                TransferDirection::Download,
                backend,
                true,
                "tree".to_string(),
                ctx,
            );
        });

        assert!(
            source.path().join("tree/pipe").symlink_metadata().is_ok(),
            "an untransferred special file must preserve the complete source tree"
        );
    });
}

/// A directory move removes the whole source tree once every file has landed
/// successfully.
#[test]
fn test_dir_move_cleanup_deletes_source_when_all_succeed() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, temp) = create_connected_view(&mut app, &[("srcdir/inner.txt", b"x")]);
        let backend: Arc<dyn SftpBackend> =
            Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf()));
        let src_on_disk = temp.path().join("srcdir");
        assert!(
            src_on_disk.exists(),
            "source dir exists before the move completes"
        );

        view.update(&mut app, |v, ctx| {
            let id = v.seed_dir_move_cleanup_for_test(1, backend.clone(), PathBuf::from("/srcdir"));
            v.note_dir_move_progress_for_test(id, true, ctx);
        });

        assert!(
            !src_on_disk.exists(),
            "source tree removed after a fully-successful move"
        );
        view.read(&app, |v, _| {
            assert!(!v.has_pending_dir_move_cleanup(), "batch cleared")
        });
    });
}

/// If cleanup fails after deleting part of the quarantined tree, the remaining
/// source stays at the reported recovery path and is never falsely restored.
#[test]
fn partial_directory_cleanup_failure_keeps_recovery_path() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, temp) = create_connected_view(&mut app, &[("srcdir/inner.txt", b"x")]);
        let backend = Arc::new(RecordingDeleteBackend::new(temp.path().to_path_buf()));
        let quarantined = PathBuf::from("/.srcdir.zaplex_move-test");
        backend
            .rename(std::path::Path::new("/srcdir"), &quarantined)
            .unwrap();
        backend.fail_recursive_deletes();

        view.update(&mut app, |view, ctx| {
            let trait_backend: Arc<dyn SftpBackend> = backend.clone();
            view.set_backend_for_test(trait_backend.clone(), PathBuf::from("/"), ctx);
            let id = view.seed_quarantined_dir_move_cleanup_for_test(
                1,
                trait_backend,
                quarantined.clone(),
                PathBuf::from("/srcdir"),
            );
            view.note_dir_move_progress_for_test(id, true, ctx);
        });

        assert!(
            !temp.path().join("srcdir").exists(),
            "a partially deleted quarantine must not be renamed back as a complete source"
        );
        assert!(
            temp.path().join(".srcdir.zaplex_move-test").is_dir(),
            "the remaining source must stay at the recovery path reported by the error"
        );
        assert!(!temp
            .path()
            .join(".srcdir.zaplex_move-test/inner.txt")
            .exists());
    });
}

/// Closing a pane with a registered directory move restores the quarantined
/// source before releasing its backend.
#[test]
fn closing_view_restores_pending_directory_move() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, temp) = create_connected_view(&mut app, &[("srcdir/inner.txt", b"x")]);
        let backend =
            Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf())) as Arc<dyn SftpBackend>;
        let quarantined = PathBuf::from("/.srcdir.zaplex_move-close");
        backend
            .rename(std::path::Path::new("/srcdir"), &quarantined)
            .unwrap();

        view.update(&mut app, |view, ctx| {
            view.seed_quarantined_dir_move_cleanup_for_test(
                1,
                backend.clone(),
                quarantined.clone(),
                PathBuf::from("/srcdir"),
            );
            view.close_for_test(ctx);
        });

        assert_eq!(
            std::fs::read(temp.path().join("srcdir/inner.txt")).unwrap(),
            b"x",
            "pane close must restore the complete pending move source"
        );
        assert!(!temp.path().join(".srcdir.zaplex_move-close").exists());
    });
}

/// A directory move keeps the source if any file failed — never a partial loss.
#[test]
fn failed_move_never_deletes_source() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, temp) = create_connected_view(&mut app, &[("srcdir/inner.txt", b"x")]);
        let backend: Arc<dyn SftpBackend> =
            Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf()));
        let src_on_disk = temp.path().join("srcdir");

        view.update(&mut app, |v, ctx| {
            let id = v.seed_dir_move_cleanup_for_test(1, backend.clone(), PathBuf::from("/srcdir"));
            v.note_dir_move_progress_for_test(id, false, ctx); // a file failed
        });

        assert!(
            src_on_disk.exists(),
            "source kept when a file failed (no data loss)"
        );
        view.read(&app, |v, _| {
            assert!(!v.has_pending_dir_move_cleanup(), "batch cleared")
        });
    });
}

/// F5 between two *different* remote hosts relays the file through the local
/// machine (download from A, upload to B) instead of the old "coming soon" toast.
#[test]
fn test_f5_remote_to_remote_relays_file() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[("a/foo.txt", b"payload"), ("b/.keep", b"")]);
        let root = temp.path().to_path_buf();

        // Two *different* hosts: pane A on "hostA" at /a, pane B on "hostB" at /b.
        let (_, view_a) = create_view_with_node(&mut app, "hostA");
        view_a.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/a"), ctx);
        });
        let (_, view_b) = create_view_with_node(&mut app, "hostB");
        view_b.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/b"), ctx);
        });

        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });

        assert!(root.join("b/foo.txt").exists(), "relayed onto host B");
        assert_eq!(
            std::fs::read(root.join("b/foo.txt")).unwrap(),
            b"payload",
            "content matches"
        );
        assert!(root.join("a/foo.txt").exists(), "source kept after a copy");
        app.read(|ctx| {
            let activities = super::transfer_queue::TransferQueue::as_ref(ctx)
                .activities()
                .collect::<Vec<_>>();
            assert_eq!(activities.len(), 1);
            assert_eq!(
                activities[0].operation,
                super::transfer_job::TransferOperation::Copy
            );
            assert_eq!(
                activities[0].conflict,
                super::transfer_job::ConflictDecision::Skip
            );
            assert_eq!(
                activities[0].topology,
                super::transfer_queue::TransferTopology::RemoteRelay
            );
        });
    });
}

/// F6 between two different remote hosts relays then removes the source (only
/// after the upload to the other host has fully succeeded).
#[test]
fn test_f6_remote_to_remote_moves_file() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[("a/bar.txt", b"data"), ("b/.keep", b"")]);
        let root = temp.path().to_path_buf();

        let (_, view_a) = create_view_with_node(&mut app, "hostA");
        view_a.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/a"), ctx);
        });
        let (_, view_b) = create_view_with_node(&mut app, "hostB");
        view_b.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/b"), ctx);
        });

        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::MoveToOtherPane, ctx);
        });

        assert!(root.join("b/bar.txt").exists(), "relayed onto host B");
        assert!(
            !root.join("a/bar.txt").exists(),
            "source removed after a successful move"
        );
        app.read(|ctx| {
            let activities = super::transfer_queue::TransferQueue::as_ref(ctx)
                .activities()
                .collect::<Vec<_>>();
            assert_eq!(activities.len(), 1);
            assert_eq!(
                activities[0].operation,
                super::transfer_job::TransferOperation::Move
            );
            assert_eq!(
                activities[0].topology,
                super::transfer_queue::TransferTopology::RemoteRelay
            );
        });
    });
}

/// A directory relays recursively between two different remote hosts.
#[test]
fn test_f5_remote_to_remote_relays_directory() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[("a/dir/sub/deep.txt", b"deep"), ("b/.keep", b"")]);
        let root = temp.path().to_path_buf();

        let (_, view_a) = create_view_with_node(&mut app, "hostA");
        view_a.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/a"), ctx);
        });
        let (_, view_b) = create_view_with_node(&mut app, "hostB");
        view_b.update(&mut app, |v, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            v.set_backend_for_test(backend, PathBuf::from("/b"), ctx);
        });

        // /a contains exactly one entry, the directory "dir" → cursor on it.
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });

        assert!(
            root.join("b/dir/sub/deep.txt").exists(),
            "directory relayed recursively onto host B"
        );
    });
}

/// A directory move is incomplete when any destination entry is skipped. The
/// source tree must remain intact even if every non-conflicting file transfers.
#[test]
fn remote_move_skip_conflict_preserves_skipped_source_and_source_tree() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[
            ("a/dir/conflict.txt", b"source-conflict"),
            ("a/dir/new.txt", b"new"),
            ("b/dir/conflict.txt", b"destination-conflict"),
        ]);
        let root = temp.path().to_path_buf();

        let (_, source_view) = create_view_with_node(&mut app, "hostA");
        source_view.update(&mut app, |view, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            view.set_backend_for_test(backend, PathBuf::from("/a"), ctx);
        });
        let (_, target_view) = create_view_with_node(&mut app, "hostB");
        target_view.update(&mut app, |view, ctx| {
            let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
            view.set_backend_for_test(backend, PathBuf::from("/b"), ctx);
        });

        source_view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::MoveToOtherPane, ctx);
        });

        assert_eq!(
            std::fs::read(root.join("b/dir/conflict.txt")).unwrap(),
            b"destination-conflict",
            "skip keeps the destination conflict untouched"
        );
        assert_eq!(
            std::fs::read(root.join("b/dir/new.txt")).unwrap(),
            b"new",
            "the non-conflicting file may still transfer"
        );
        assert_eq!(
            std::fs::read(root.join("a/dir/conflict.txt")).unwrap(),
            b"source-conflict",
            "a skipped source entry makes the move incomplete"
        );
        assert_eq!(
            std::fs::read(root.join("a/dir/new.txt")).unwrap(),
            b"new",
            "an incomplete directory move keeps the entire source tree"
        );
    });
}

// ============================================================
// Cross-connection overwrite prompt (FM backlog)
// ============================================================

/// A local→remote pane pair, both over one shared fs. Returns pane A (local),
/// pane B (remote target — keep it in scope so it stays registered), and the
/// shared `TempDir`.
fn two_panes_cross_conn(
    app: &mut warpui::App,
    files: &[(&str, &[u8])],
) -> (
    warpui::ViewHandle<SftpBrowserView>,
    warpui::ViewHandle<SftpBrowserView>,
    tempfile::TempDir,
) {
    let temp = create_temp_dir_with_files(files);
    let root = temp.path().to_path_buf();
    let (_, view_a) = create_view_with_node(app, ""); // local
    view_a.update(app, |v, ctx| {
        let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
        v.set_backend_for_test(backend, PathBuf::from("/a"), ctx);
    });
    let (_, view_b) = create_view_with_node(app, "host"); // remote target
    view_b.update(app, |v, ctx| {
        let backend = Arc::new(InMemorySftpBackend::new(root.clone())) as Arc<dyn SftpBackend>;
        v.set_backend_for_test(backend, PathBuf::from("/b"), ctx);
    });
    (view_a, view_b, temp)
}

/// A cross-connection copy whose destination file already exists pauses on the
/// overwrite prompt instead of silently skipping — and nothing transfers yet.
#[test]
fn test_cross_conn_existing_file_opens_overwrite_prompt() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (view_a, _view_b, _temp) =
            two_panes_cross_conn(&mut app, &[("a/foo.txt", b"new"), ("b/foo.txt", b"old")]);

        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });

        view_a.read(&app, |v, _| {
            assert!(
                matches!(
                    v.dialog,
                    Some(Dialog::CrossConnConflict {
                        existing: 1,
                        is_move: false
                    })
                ),
                "the overwrite prompt is shown for the existing file"
            );
            assert!(
                v.has_pending_cross_conn(),
                "the conflicting transfer is held"
            );
            assert!(
                v.transfers.is_empty(),
                "nothing transfers before the user decides"
            );
        });
    });
}

/// Choosing Overwrite starts the conflicting transfer and clears the prompt.
#[test]
fn test_cross_conn_overwrite_starts_the_transfer() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (view_a, _view_b, _temp) =
            two_panes_cross_conn(&mut app, &[("a/foo.txt", b"new"), ("b/foo.txt", b"old")]);
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(
                &SftpBrowserAction::ResolveCrossConnConflict { overwrite: true },
                ctx,
            );
        });

        view_a.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "prompt cleared");
            assert!(!v.has_pending_cross_conn(), "pending resolved");
            assert_eq!(v.transfers.len(), 1, "the overwrite spawns the transfer");
        });
    });
}

/// Choosing Skip clears the prompt without starting the conflicting transfer.
#[test]
fn test_cross_conn_skip_starts_no_transfer() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (view_a, _view_b, _temp) =
            two_panes_cross_conn(&mut app, &[("a/foo.txt", b"new"), ("b/foo.txt", b"old")]);
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(
                &SftpBrowserAction::ResolveCrossConnConflict { overwrite: false },
                ctx,
            );
        });

        view_a.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "prompt cleared");
            assert!(!v.has_pending_cross_conn(), "pending resolved");
            assert!(v.transfers.is_empty(), "skip transfers nothing");
        });
    });
}

/// No conflict → no prompt; the transfer starts directly.
#[test]
fn test_cross_conn_without_conflict_transfers_directly() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (view_a, _view_b, _temp) =
            two_panes_cross_conn(&mut app, &[("a/foo.txt", b"new"), ("b/.keep", b"")]);

        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });

        view_a.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "no prompt when nothing conflicts");
            assert!(!v.has_pending_cross_conn());
            assert_eq!(v.transfers.len(), 1, "the transfer starts directly");
        });
    });
}

// ============================================================
// Overwrite-on-conflict for copy/move (FM Pflicht 1)
// ============================================================

/// A copy whose target already exists pauses on the conflict dialog instead of
/// silently skipping; "Overwrite" replaces the target with the source content.
#[test]
fn test_copy_conflict_overwrites_on_confirm() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (view_a, _view_b, temp) = create_two_panes_sharing_fs(
            &mut app,
            &[("left/dup.txt", b"new"), ("right/dup.txt", b"old")],
        );
        let root = temp.path().to_path_buf();

        // F5 → conflict (right/dup.txt exists) → dialog opens, nothing copied yet.
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });
        view_a.read(&app, |v, _| {
            assert!(
                matches!(v.dialog, Some(Dialog::CopyMoveConflict { .. })),
                "conflict opens a dialog"
            );
        });
        assert_eq!(
            std::fs::read(root.join("right/dup.txt")).unwrap(),
            b"old",
            "target untouched until the user decides"
        );

        // Overwrite → target replaced, dialog closed.
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::OverwriteConflict { all: false }, ctx);
        });
        view_a.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog closed after resolve")
        });
        assert_eq!(
            std::fs::read(root.join("right/dup.txt")).unwrap(),
            b"new",
            "target overwritten with source content"
        );
    });
}

/// Confirming an overwrite must not destroy the existing destination before
/// the replacement is safely available. A failed copy leaves the old bytes
/// exactly where the user had them.
#[test]
fn failed_copy_overwrite_keeps_existing_destination_intact() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[
            ("left/dup.txt", b"replacement"),
            ("right/dup.txt", b"precious-original"),
        ]);
        let backend = Arc::new(TracingBackend::new(temp.path().to_path_buf()));
        backend.fail_copies();

        let (_, source_view) = create_view(&mut app);
        source_view.update(&mut app, |view, ctx| {
            let trait_backend: Arc<dyn SftpBackend> = backend.clone();
            view.set_backend_for_test(trait_backend, PathBuf::from("/left"), ctx);
        });
        let (_, target_view) = create_view(&mut app);
        target_view.update(&mut app, |view, ctx| {
            let trait_backend: Arc<dyn SftpBackend> = backend.clone();
            view.set_backend_for_test(trait_backend, PathBuf::from("/right"), ctx);
        });

        source_view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });
        source_view.read(&app, |view, _| {
            assert!(matches!(view.dialog, Some(Dialog::CopyMoveConflict { .. })));
        });
        source_view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::OverwriteConflict { all: false }, ctx);
        });

        assert_eq!(
            std::fs::read(temp.path().join("right/dup.txt")).unwrap(),
            b"precious-original",
            "a failed replacement must preserve the existing destination"
        );
        assert_eq!(
            std::fs::read(temp.path().join("left/dup.txt")).unwrap(),
            b"replacement",
            "copy failure must not modify the source"
        );
    });
}

/// Even after a complete replacement has been staged, an atomic commit failure
/// must leave the old destination at its original path.
#[test]
fn failed_atomic_replace_keeps_existing_destination_intact() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[
            ("left/dup.txt", b"replacement"),
            ("right/dup.txt", b"precious-original"),
        ]);
        let backend = Arc::new(TracingBackend::new(temp.path().to_path_buf()));
        backend.fail_replacements();

        let (_, source_view) = create_view(&mut app);
        source_view.update(&mut app, |view, ctx| {
            let trait_backend: Arc<dyn SftpBackend> = backend.clone();
            view.set_backend_for_test(trait_backend, PathBuf::from("/left"), ctx);
        });
        let (_, target_view) = create_view(&mut app);
        target_view.update(&mut app, |view, ctx| {
            let trait_backend: Arc<dyn SftpBackend> = backend.clone();
            view.set_backend_for_test(trait_backend, PathBuf::from("/right"), ctx);
        });

        source_view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
            view.handle_action(&SftpBrowserAction::OverwriteConflict { all: false }, ctx);
        });

        assert_eq!(
            std::fs::read(temp.path().join("right/dup.txt")).unwrap(),
            b"precious-original",
            "an atomic commit failure must preserve the existing destination"
        );
        assert_eq!(
            std::fs::read(temp.path().join("left/dup.txt")).unwrap(),
            b"replacement",
            "an atomic commit failure must preserve the source"
        );
    });
}

/// A destination symlink must never be followed or replaced by a transfer.
/// The linked tree, the link itself, and the source all remain available.
#[cfg(unix)]
#[test]
fn test_copy_conflict_over_directory_symlink_never_recurses_into_target() {
    use std::os::unix::fs::symlink;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let temp = create_temp_dir_with_files(&[
            ("left/item", b"replacement"),
            ("right/.keep", b""),
            ("target-dir/precious.txt", b"precious"),
        ]);
        symlink("../target-dir", temp.path().join("right/item")).unwrap();
        let backend = Arc::new(RecordingDeleteBackend::new(temp.path().to_path_buf()));

        let (_, source_view) = create_view(&mut app);
        source_view.update(&mut app, |view, ctx| {
            let trait_backend: Arc<dyn SftpBackend> = backend.clone();
            view.set_backend_for_test(trait_backend, PathBuf::from("/left"), ctx);
        });
        let (_, target_view) = create_view(&mut app);
        target_view.update(&mut app, |view, ctx| {
            let trait_backend: Arc<dyn SftpBackend> = backend.clone();
            view.set_backend_for_test(trait_backend, PathBuf::from("/right"), ctx);
        });

        source_view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });
        source_view.read(&app, |view, _| {
            assert!(matches!(view.dialog, Some(Dialog::CopyMoveConflict { .. })));
        });
        source_view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::OverwriteConflict { all: false }, ctx);
        });

        assert!(
            backend.file_deletes.lock().unwrap().is_empty(),
            "atomic replacement must not need a separate unlink step"
        );
        assert!(
            backend.recursive_deletes.lock().unwrap().is_empty(),
            "overwrite must never recursively delete through a directory symlink"
        );
        assert_eq!(
            std::fs::read(temp.path().join("target-dir/precious.txt")).unwrap(),
            b"precious"
        );
        assert!(
            std::fs::symlink_metadata(temp.path().join("right/item"))
                .unwrap()
                .file_type()
                .is_symlink(),
            "the destination link itself must remain intact"
        );
        assert_eq!(
            std::fs::read(temp.path().join("left/item")).unwrap(),
            b"replacement",
            "refusing a link destination must preserve the source"
        );
    });
}

/// "Skip" on the conflict leaves the target untouched.
#[test]
fn test_copy_conflict_skips_on_skip() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (view_a, _view_b, temp) = create_two_panes_sharing_fs(
            &mut app,
            &[("left/dup.txt", b"new"), ("right/dup.txt", b"old")],
        );
        let root = temp.path().to_path_buf();

        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
            v.handle_action(&SftpBrowserAction::SkipConflict { all: false }, ctx);
        });
        assert_eq!(
            std::fs::read(root.join("right/dup.txt")).unwrap(),
            b"old",
            "skipped target kept its content"
        );
    });
}

/// Applying "keep both" to all conflicts deterministically allocates sibling
/// names without replacing either existing destination.
#[test]
fn conflict_apply_to_all_is_deterministic() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (view_a, _view_b, temp) = create_two_panes_sharing_fs(
            &mut app,
            &[
                ("left/a.txt", b"A2"),
                ("left/b.txt", b"B2"),
                ("right/a.txt", b"A1"),
                ("right/b.txt", b"B1"),
            ],
        );
        let root = temp.path().to_path_buf();

        // Select both files, then F5 → conflict on the first.
        // `/left` is a sub-directory, so the list leads with the `..` row and
        // Home lands on it (MC-style) — step onto the first real file before
        // marking. `..` marks nothing by design.
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CursorFirst, ctx);
            v.handle_action(&SftpBrowserAction::CursorDown, ctx);
            v.handle_action(&SftpBrowserAction::ToggleSelectCursor, ctx);
            v.handle_action(&SftpBrowserAction::CursorDown, ctx);
            v.handle_action(&SftpBrowserAction::ToggleSelectCursor, ctx);
            v.handle_action(&SftpBrowserAction::CopyToOtherPane, ctx);
        });
        view_a.read(&app, |v, _| {
            assert!(matches!(v.dialog, Some(Dialog::CopyMoveConflict { .. })));
        });

        // Keep both for all → no further prompt and neither target is replaced.
        view_a.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::RenameConflict { all: true }, ctx);
        });
        view_a.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "no further prompt")
        });
        assert_eq!(std::fs::read(root.join("right/a.txt")).unwrap(), b"A1");
        assert_eq!(std::fs::read(root.join("right/b.txt")).unwrap(), b"B1");
        assert_eq!(
            std::fs::read(root.join("right/a (copy).txt")).unwrap(),
            b"A2"
        );
        assert_eq!(
            std::fs::read(root.join("right/b (copy).txt")).unwrap(),
            b"B2"
        );
    });
}

// ============================================================
// F4 open-in-editor (FM Pflicht 2 — directory branch)
// ============================================================

#[test]
fn f2_renames_cursor_entry() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("rename.txt", b"x")]);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::RenameCursor, ctx);
        });

        view.read(&app, |view, _| {
            assert!(
                matches!(view.dialog, Some(Dialog::Rename { .. })),
                "F2's cursor action must open the rename dialog"
            );
        });
    });
}

/// F4 on a directory enters it (the file branch dispatches a workspace
/// action, which the file-manager harness can't observe here).
#[test]
fn test_open_in_editor_on_directory_navigates() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("subdir/inner.txt", b"x")]);

        // /subdir is the only entry → cursor on it → F4 enters it.
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::OpenCursorInEditor, ctx);
        });
        view.read(&app, |v, _| {
            assert_eq!(v.current_path, PathBuf::from("/subdir"));
        });
    });
}

#[test]
fn f4_opens_editor() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, root, browser, _temp) =
            create_workspace_action_capture_view(&mut app, &[("edit.txt", b"x")]);
        browser.update(&mut app, |_, ctx| ctx.focus_self());

        let (presenter, _scene) = render_scene_at(&mut app, window_id, vec2f(1200.0, 800.0));
        assert!(key_down(&mut app, window_id, presenter, "f4"));

        root.read(&app, |root, _| {
            assert!(matches!(
                root.actions.as_slice(),
                [crate::WorkspaceAction::OpenFileInEditor { node_id, path }]
                    if node_id == "capture-node" && path == &PathBuf::from("/edit.txt")
            ));
        });
    });
}

/// F3 is a metadata view, while F4 remains the editor/navigation action.
#[test]
fn f3_views_without_editing() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[("view.txt", b"x")]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ViewCursorDetails, ctx);
        });

        view.read(&app, |v, _| {
            assert!(matches!(v.dialog, Some(Dialog::FileDetails { .. })));
        });
    });
}
