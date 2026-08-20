use super::*;
use crate::ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions};
use crate::ai::document::ai_document_model::AIDocumentModel;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::facts::manager::AIFactManager;
use crate::ai::llms::LLMPreferences;
use crate::ai::restored_conversations::RestoredAgentConversations;
use crate::ai::skills::SkillManager;
use crate::ai::AIRequestUsageModel;
use crate::auth::UserUid;
use crate::cloud_object::model::persistence::ObjectStoreModel;
use crate::cloud_object::model::view::ObjectStoreViewModel;
use crate::context_chips::prompt::Prompt;
use crate::editor::Event;
use crate::editor::ReplicaId;
use crate::gpu_state::GPUState;
use crate::network::NetworkStatus;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::notebooks::notebook::NotebookView;
use crate::pane_group::{Direction, PaneGroupAction, PaneId};
use crate::suggestions::ignored_suggestions_model::IgnoredSuggestionsModel;
use crate::terminal::shared_session::protocol::SessionSourceType;
use crate::terminal::shared_session::protocol::{ParticipantId, ParticipantList};
#[cfg(feature = "local_fs")]
use crate::user_config::tab_configs_dir;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
#[cfg(feature = "local_fs")]
use repo_metadata::RepoMetadataModel;
use std::collections::HashMap;
use std::sync::Arc;
use watcher::HomeDirectoryWatcher;

use crate::cloud_object::update_manager::UpdateManager;
use crate::server::experiments::ServerExperiments;

use crate::settings::PrivacySettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::settings_view::DisplayCount;
use crate::system::SystemStats;
use crate::tab_configs::tab_config::{TabConfigPaneNode, TabConfigPaneType};
use crate::terminal::history::History;
use crate::terminal::keys::TerminalKeybindings;
#[cfg(windows)]
use crate::util::traffic_lights::windows::RendererState;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;

use crate::terminal::local_tty::spawner::PtySpawner;
use crate::terminal::shared_session::{SharedSessionScrollbackType, SharedSessionStatus};

use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier;
use crate::ai::mcp::{
    gallery::MCPGalleryManager, templatable_manager::TemplatableMCPServerManager,
    FileBasedMCPManager, FileMCPWatcher,
};
use crate::resource_center::Tip;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::undo_close::UndoCloseSettings;
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;
use crate::workflows::local_workflows::LocalWorkflows;
use crate::ObjectActions;
use crate::{experiments, workspace, GlobalResourceHandlesProvider};

// Zaplex(localization, Phase 5): `PreferencesSyncer` has been physically deleted.

use crate::terminal::shared_session::protocol::SessionId;
use ai::project_context::model::ProjectContextModel;
use pane_group::{NotebookPane, PaneState, SplitPaneState, TerminalPaneId};
use terminal::view::ActiveSessionState;
use warpui::AddSingletonModel;
use warpui::{platform::WindowStyle, App, ViewHandle};

#[test]
fn launch_request_never_interpolates_remote_path_into_shell_source() {
    let remote_path = "/srv/projects/a path;$(touch /tmp/not-shell-source)";
    let request = RemoteAgentLaunchRequest::new(
        Some(Path::new(remote_path)),
        CLIAgent::Codex.routed_launch(None, Some("gpt-5-codex"), Some("high")),
    );

    assert!(!request.shell_source().contains(remote_path));
    let shell_argv = shell_words::split(&request.shell_command())
        .expect("launch command must remain valid argv");
    assert_eq!(shell_argv[0], "sh");
    assert_eq!(shell_argv[1], "-c");
    assert_eq!(shell_argv[2], REMOTE_AGENT_LAUNCH_SHELL_SOURCE);
    assert_eq!(shell_argv[3], "zaplex-agent-launch");
    assert_eq!(shell_argv[4], remote_path);
    let expected_agent_argv = vec![
        "codex".to_string(),
        "--model".to_string(),
        "gpt-5-codex".to_string(),
        "-c".to_string(),
        "model_reasoning_effort=\"high\"".to_string(),
    ];
    assert_eq!(&shell_argv[8..], expected_agent_argv.as_slice());
}

fn initialize_app(app: &mut App) {
    // Load the bundled localization so `t!` returns real strings (not keys) —
    // mirrors prod (`lib.rs` `i18n::init`). Force English so the label-based menu
    // tests are locale-deterministic. Without it `t!` returns raw fluent keys and
    // the UI logs "called before init()".
    crate::i18n::init(Some("en"));
    initialize_settings_for_tests(app);

    // Add the necessary singleton models to the App
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|_ctx| PtySpawner::new_for_test());
    app.add_singleton_model(|_| Prompt::mock());
    app.add_singleton_model(|_| AutoupdateState::new(Arc::new(http_client::Client::new())));
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(ObjectStoreModel::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(|_ctx| UserProfiles::new(Vec::new()));
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(MCPGalleryManager::new);
    app.add_singleton_model(ObjectStoreViewModel::mock);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(AppearanceManager::new);
    app.add_singleton_model(|_| DisplayCount::mock());
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(|_ctx| RelaunchModel::new());
    app.add_singleton_model(|_| ChangelogModel::new(Arc::new(http_client::Client::new())));
    app.add_singleton_model(|_| GitHubAuthNotifier::new());
    app.add_singleton_model(|_| crate::ssh_manager::SshTreeChangedNotifier::new());
    app.add_singleton_model(crate::cockpit::favorites::FavoritesStore::new_for_test);
    app.add_singleton_model(|_ctx| SyncedInputState::mock());
    app.add_singleton_model(|_| ResizableData::default());
    app.add_singleton_model(LocalWorkflows::new);
    app.add_singleton_model(UndoCloseStack::new);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(|_| WorkspaceToastStack);
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(TerminalKeybindings::new);
    app.add_singleton_model(NotebookManager::mock);
    // Zaplex(localization, Phase 5): `PreferencesSyncer` has been physically deleted, test singleton no longer needed.
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    app.add_singleton_model(AgentConversationsModel::new);
    // Must precede `LLMPreferences::new` (which subscribes to it) and
    // `workspace::init` (which materializes command-palette binding descriptions
    // that read it). Without it the whole workspace-view test class — and the
    // boot/open smoke gate below — panic headless.
    app.add_singleton_model(crate::ai::agent_providers::AgentProviderSecrets::new);
    app.add_singleton_model(LLMPreferences::new);
    app.add_singleton_model(|_| SettingsPaneManager::new());
    app.add_singleton_model(|_| AIFactManager::new());

    // Initialize file-based MCP dependencies.
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
    app.add_singleton_model(DirectoryWatcher::new);
    app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
    app.add_singleton_model(FileMCPWatcher::new);
    app.add_singleton_model(|_| FileBasedMCPManager::default());

    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&crate::LaunchMode::new_for_unit_test(), ctx)
    });
    // Zaplex: RepoOutlines has been deleted, no longer registered.
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
    app.add_singleton_model(BlocklistAIPermissions::new);
    app.add_singleton_model(|_| GPUState::new());
    app.add_singleton_model(|_| RestoredAgentConversations::default());
    app.add_singleton_model(OneTimeModalModel::new);
    // Register GlobalResourceHandlesProvider before ServerExperiments which depends on it
    let global_resource_handles = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
    app.add_singleton_model(|ctx| ServerExperiments::new_from_cache(vec![], ctx));
    app.add_singleton_model(DefaultTerminal::new);
    app.add_singleton_model(|_| IgnoredSuggestionsModel::new(vec![]));
    app.add_singleton_model(|_| crate::code_review::git_status_update::GitStatusUpdateModel::new());
    app.add_singleton_model(remote_server::manager::RemoteServerManager::new);

    #[cfg(feature = "local_fs")]
    app.add_singleton_model(RepoMetadataModel::new);
    app.add_singleton_model(search::files::model::FileSearchModel::new);

    #[cfg(windows)]
    {
        app.add_singleton_model(RendererState::new);
    }

    #[cfg(feature = "local_tty")]
    terminal::available_shells::register(app);
    AltScreenReporting::register(app);

    #[cfg(enable_crash_recovery)]
    crate::crash_recovery::CrashRecovery::register_for_test(app);

    app.update(experiments::init);

    app.add_singleton_model(AIRequestUsageModel::new_for_test);
    app.add_singleton_model(
        crate::workspace::bonus_grant_notification_model::BonusGrantNotificationModel::new,
    );

    app.add_singleton_model(|_| ProjectContextModel::default());
    app.add_singleton_model(AIDocumentModel::new);
    app.add_singleton_model(|_| History::new(vec![]));

    // SkillManager must be registered because the command palette materializes
    // binding descriptions eagerly, and `workspace:send_feedback`'s dynamic
    // label calls `is_feedback_skill_available`, which reads `SkillManager`.
    // Registered after `HomeDirectoryWatcher`, `DirectoryWatcher`,
    // `WarpManagedPathsWatcher`, `DetectedRepositories`, and `RepoMetadataModel`
    // because `SkillWatcher::new` subscribes to all of them.
    app.add_singleton_model(SkillManager::new);

    // The cockpit spine leads the left panel, so building a workspace window
    // constructs the CockpitPanel — which reads these. CockpitModel subscribes to
    // HomeDirectoryWatcher (already registered above) and AttentionDriver to
    // CockpitModel, so the order matters. CLIAgentInstallModel is read by the
    // new-session menu + agent surfaces.
    app.add_singleton_model(crate::terminal::cli_agent::CLIAgentInstallModel::new);
    app.add_singleton_model(crate::settings::network_secrets::ProxyCredentials::new);
    app.add_singleton_model(crate::settings::CloudSyncTokenStore::new);
    // FileModel before GlobalBufferModel (whose `new` reads it), mirroring prod.
    app.add_singleton_model(warp_files::FileModel::new);
    app.add_singleton_model(crate::code::global_buffer_model::GlobalBufferModel::new);
    app.add_singleton_model(crate::cockpit::CockpitModel::new);
    app.add_singleton_model(crate::cockpit::AttentionDriver::new);

    // Make sure to initialize the keybindings so that they are available for subviews
    app.update(workspace::init);
}

fn assert_review21_skip_toast_is_neutral(directory: bool) {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.add_singleton_model(|_| crate::sftp_manager::fm_registry::FileManagerRegistry::new());
        app.add_singleton_model(|_| crate::sftp_manager::transfer_queue::TransferQueue::new());
        let source_root = tempfile::tempdir().unwrap();
        let target_root = tempfile::tempdir().unwrap();
        if directory {
            std::fs::create_dir_all(source_root.path().join("item")).unwrap();
            std::fs::create_dir_all(target_root.path().join("item")).unwrap();
            std::fs::write(source_root.path().join("item/same.bin"), b"same").unwrap();
            std::fs::write(target_root.path().join("item/same.bin"), b"same").unwrap();
        } else {
            std::fs::write(source_root.path().join("item.bin"), b"same").unwrap();
            std::fs::write(target_root.path().join("item.bin"), b"same").unwrap();
        }
        let source_backend = Arc::new(crate::sftp_manager::sftp_backend::InMemorySftpBackend::new(
            source_root.path().to_path_buf(),
        )) as Arc<dyn crate::sftp_manager::sftp_backend::SftpBackend>;
        let target_backend = Arc::new(crate::sftp_manager::sftp_backend::InMemorySftpBackend::new(
            target_root.path().to_path_buf(),
        )) as Arc<dyn crate::sftp_manager::sftp_backend::SftpBackend>;
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            crate::sftp_manager::browser::SftpBrowserView::new("review21".to_string(), None, ctx)
        });
        let flavor = Arc::new(std::sync::Mutex::new(None));
        app.update(|ctx| {
            let observed = flavor.clone();
            ctx.subscribe_to_model(
                &crate::workspace::ToastStack::handle(ctx),
                move |_, event, _| {
                    if let crate::workspace::toast_stack::ToastStackEvent::AddEphemeralToast {
                        toast,
                        ..
                    } = event
                    {
                        *observed.lock().unwrap() = Some(toast.flavor_for_test());
                    }
                },
            );
        });

        view.update(&mut app, |view, ctx| {
            view.spawn_backend_queue_job_for_test(
                source_backend,
                target_backend,
                if directory {
                    PathBuf::from("/item")
                } else {
                    PathBuf::from("/item.bin")
                },
                if directory {
                    PathBuf::from("/item")
                } else {
                    PathBuf::from("/item.bin")
                },
                directory,
                if directory {
                    crate::sftp_manager::transfer_job::ConflictDecision::MergeSkip
                } else {
                    crate::sftp_manager::transfer_job::ConflictDecision::Skip
                },
                "item".to_string(),
                ctx,
            );
        });

        assert_eq!(
            *flavor.lock().unwrap(),
            Some(crate::view_components::ToastFlavor::Default),
            "a fully skipped transfer must use a neutral toast presentation"
        );
    });
}

#[test]
fn review21_file_skip_toast_is_neutral() {
    assert_review21_skip_toast_is_neutral(false);
}

#[test]
fn review21_directory_skip_toast_is_neutral() {
    assert_review21_skip_toast_is_neutral(true);
}

fn mock_workspace(app: &mut App) -> ViewHandle<Workspace> {
    let global_resource_handles = GlobalResourceHandles::mock(app);
    let active_window_id = app.read(|ctx| ctx.windows().active_window());
    let (_, workspace) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        Workspace::new(
            global_resource_handles,
            None,
            NewWorkspaceSource::Empty {
                previous_active_window: active_window_id,
                shell: None,
            },
            ctx,
        )
    });
    workspace
}

/// Boot / keymap smoke test — closes the gap that let a malformed keybinding
/// (`cmd-shift-o` instead of `cmd-shift-O`) pass `cargo check` yet panic at
/// startup during binding registration, leaving the app unopenable. The merge
/// gate is a Linux `cargo check`, which compiles the code but never *runs* the
/// registration path, so it cannot catch this "does it even open" class.
///
/// This exercises the real registration path:
///   * `initialize_app` runs `workspace::init` (where the regression lived), and
///   * `terminal::init` runs the terminal + terminal-view binding registration.
///
/// Under debug assertions `Keystroke::parse` panics on a malformed binding
/// string, and — thanks to the cross-platform validation in `warpui::keymap`
/// (`validate_off_platform_binding`) — that check fires for *both* platform
/// variants of every per-platform binding even on this (Linux) CI host, not just
/// the variant this OS uses. Building a workspace window then covers the rest of
/// the "does it open" path. Reaching the end without panicking proves every
/// registered binding (mac and linux/windows) parses and that a window opens.
///
/// No longer `#[ignore]`d (#104): `initialize_app` now registers the full
/// singleton graph the construction path reads (`AgentProviderSecrets`,
/// `LanguageSettings`, `CockpitSettings`, `NetworkSettings`, `CloudSyncSettings`,
/// the cockpit + code/file singletons, …), so building a workspace headless no
/// longer panics. This is now a real boot/open smoke gate: it runs the binding
/// registration path *and* constructs a window, catching both the keymap-crash
/// class and "does the UI wiring survive construction".
#[test]
fn test_boot_registers_all_keybindings_without_panicking() {
    App::test((), |mut app| async move {
        // `initialize_app` runs `workspace::init` — where the `cmd-shift-o`
        // regression lived — after registering every singleton the registration
        // path reads. Under debug assertions, `warpui::keymap`'s
        // `validate_off_platform_binding` parses *both* platform variants of every
        // per-platform binding even on this Linux CI host, so a malformed mac
        // binding (the exact regression) panics during registration here rather
        // than only on macOS. Building a workspace window then covers the rest of
        // the "does it even open" path (constructing the views that register the
        // terminal keybindings). Reaching the end proves the registration path runs
        // and a window opens without panicking.
        initialize_app(&mut app);
        let _workspace = mock_workspace(&mut app);
    });
}

#[test]
fn transfer_recovery_retry_allocation_failure_is_visible_and_non_destructive() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.add_singleton_model(|_| crate::sftp_manager::transfer_queue::TransferQueue::new());
        let workspace = mock_workspace(&mut app);
        let transfer_id = crate::sftp_manager::transfer_queue::TransferQueue::handle(&app).update(
            &mut app,
            |queue, _| {
                let transfer_id = queue.enqueue("workspace", 1).unwrap();
                queue.activity_handle(transfer_id).unwrap().set_error(
                    &crate::sftp_manager::sftp_ops::SftpOpsError::RecoveryRequired {
                        message: "retained cleanup".to_string(),
                        recovery_id: Some(42),
                        paths: vec![std::path::PathBuf::from("/retained")],
                        committed: false,
                    },
                );
                queue.exhaust_recovery_attempt_ids();
                transfer_id
            },
        );

        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::RetryTransferRecovery(transfer_id), ctx);
        });

        workspace.read(&app, |workspace, app| {
            assert!(
                workspace.toast_stack.as_ref(app).has_toasts(),
                "the global retry error must be visible"
            );
        });
        crate::sftp_manager::transfer_queue::TransferQueue::handle(&app).update(
            &mut app,
            |queue, _| {
                let retained = queue.activity(transfer_id).unwrap();
                assert!(retained.recovery_retryable);
                assert!(matches!(
                    retained.state,
                    crate::sftp_manager::transfer_queue::QueuedTransferState::Failed(_)
                ));
                assert!(queue.enqueue("workspace", 1).is_ok());
            },
        );
    });
}

#[test]
fn startup_directory_recovery_is_visible_and_retryable_without_sftp_browser() {
    use crate::sftp_manager::sftp_backend::{
        DirectoryReservationFailure, InMemorySftpBackend, SftpBackend,
    };
    use crate::sftp_manager::transfer_queue::{QueuedTransferState, TransferQueue};
    use std::path::Path;
    use std::time::Duration;
    use warpui::r#async::Timer;

    let root = tempfile::tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_directory_reservation_failure(DirectoryReservationFailure::Identity);
    let retained =
        match backend.create_dir_with_ownership_anchor(Path::new("/retained-after-crash")) {
            Ok(_) => panic!("the injected crash artifact must be retained"),
            Err(error) => error,
        };
    assert!(
        retained.recovery_id().is_none(),
        "the backend-level artifact is discovered by the next process"
    );
    drop(backend);
    let restarted: Arc<dyn SftpBackend> =
        Arc::new(InMemorySftpBackend::new(root.path().to_path_buf()));
    assert_eq!(restarted.startup_recovery_paths().len(), 1);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.add_singleton_model(move |ctx| {
            TransferQueue::new_with_startup_backend_for_test(restarted, ctx)
        });
        let workspace = mock_workspace(&mut app);
        let transfer_id = TransferQueue::handle(&app).read(&app, |queue, _| {
            let activities = queue.activities().collect::<Vec<_>>();
            assert_eq!(activities.len(), 1);
            assert!(activities[0].recovery_retryable);
            assert!(matches!(
                activities[0].state,
                QueuedTransferState::Failed(_)
            ));
            activities[0].id
        });
        app.read(|app| {
            assert!(
                crate::sftp_manager::transfer_panel::render_workspace_transfer_panel(app).is_some(),
                "startup recovery must be visible at workspace level without an SFTP pane"
            );
        });

        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::RetryTransferRecovery(transfer_id), ctx);
        });
        for _ in 0..50 {
            Timer::after(Duration::from_millis(20)).await;
            let terminal = TransferQueue::handle(&app).read(&app, |queue, _| {
                queue.activity(transfer_id).is_some_and(|activity| {
                    matches!(activity.state, QueuedTransferState::Completed)
                        && !activity.recovery_retryable
                })
            });
            if terminal {
                return;
            }
        }
        let activity =
            TransferQueue::handle(&app).read(&app, |queue, _| queue.activity(transfer_id));
        panic!("workspace retry did not complete the restart recovery: {activity:?}");
    });
}

fn restored_workspace(
    app: &mut App,
    window_snapshot: crate::app_state::WindowSnapshot,
) -> ViewHandle<Workspace> {
    let global_resource_handles = GlobalResourceHandles::mock(app);
    let (_, workspace) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        Workspace::new(
            global_resource_handles,
            None,
            NewWorkspaceSource::Restored {
                window_snapshot,
                block_lists: Arc::new(HashMap::new()),
            },
            ctx,
        )
    });
    workspace
}

fn transferred_tab_workspace(
    app: &mut App,
    vertical_tabs_panel_open: bool,
) -> ViewHandle<Workspace> {
    let global_resource_handles = GlobalResourceHandles::mock(app);
    let (_, workspace) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        Workspace::new(
            global_resource_handles,
            None,
            NewWorkspaceSource::TransferredTab {
                tab_color: None,
                is_pinned: false,
                custom_title: None,
                left_panel_open: false,
                vertical_tabs_panel_open,
                right_panel_open: false,
                is_right_panel_maximized: false,
                is_tab_drag_preview: false,
            },
            ctx,
        )
    });
    workspace
}

#[cfg(feature = "local_fs")]
fn open_worktree_sidecar(workspace: &ViewHandle<Workspace>, app: &mut App) {
    workspace.update(app, |workspace, ctx| {
        workspace.open_new_session_dropdown_menu(Vector2F::zero(), ctx);

        let worktree_index = workspace
            .new_session_dropdown_menu
            .read(ctx, |menu, _| {
                menu.items().iter().position(|item| {
                    matches!(
                        item,
                        MenuItem::Item(fields) if fields.label() == "New worktree config"
                    )
                })
            })
            .expect("expected new worktree config item in new-session menu");

        workspace
            .new_session_dropdown_menu
            .update(ctx, |menu, view_ctx| {
                menu.set_selected_by_index(worktree_index, view_ctx);
            });
    });
}

#[cfg(feature = "local_fs")]
#[test]
#[ignore = "depends on decommissioned PersistedWorkspace"]
fn test_worktree_sidecar_hover_takes_precedence_over_selection() {
    unimplemented!(
        "PersistedWorkspace has been decommissioned, worktree sidecar repo list tests suspended"
    );
}

#[cfg(feature = "local_fs")]
#[test]
#[ignore = "depends on decommissioned PersistedWorkspace"]
fn test_worktree_sidecar_pointer_entry_does_not_select_top_repo() {
    unimplemented!(
        "PersistedWorkspace has been decommissioned, worktree sidecar repo list tests suspended"
    );
}

#[cfg(feature = "local_fs")]
#[test]
#[ignore = "depends on decommissioned PersistedWorkspace"]
fn test_worktree_sidecar_close_via_select_item_executes_from_workspace() {
    unimplemented!(
        "PersistedWorkspace has been decommissioned, worktree sidecar repo list tests suspended"
    );
}

#[cfg(feature = "local_fs")]
#[test]
#[ignore = "depends on decommissioned PersistedWorkspace"]
fn test_worktree_sidecar_search_editor_enter_executes_selection() {
    unimplemented!(
        "PersistedWorkspace has been decommissioned, worktree sidecar repo list tests suspended"
    );
}

/// RAII guard that removes tab config TOML files whose name starts with
/// `prefix` from `~/.warp/tab_configs/` on drop. Because `Drop` runs even
/// when a test panics, this prevents stale worktree configs from leaking
/// into Zaplex dev.
#[cfg(feature = "local_fs")]
struct TabConfigCleanupGuard {
    prefix: &'static str,
}

#[cfg(feature = "local_fs")]
impl TabConfigCleanupGuard {
    fn new(prefix: &'static str) -> Self {
        // Eagerly clean up leftovers from any previously-crashed run.
        Self::clean(prefix);
        Self { prefix }
    }

    fn clean(prefix: &str) {
        let dir = tab_configs_dir();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(prefix) && name.ends_with(".toml") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

#[cfg(feature = "local_fs")]
impl Drop for TabConfigCleanupGuard {
    fn drop(&mut self) {
        Self::clean(self.prefix);
    }
}

/// Creates a workspace with a single, shared session.
fn mock_workspace_with_shared_session(app: &mut App) -> ViewHandle<Workspace> {
    // Create the workspace as a session-sharing sharer.
    let global_resource_handles = GlobalResourceHandles::mock(app);
    let (_, workspace) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        Workspace::new(
            global_resource_handles,
            None,
            NewWorkspaceSource::Empty {
                previous_active_window: None,
                shell: None,
            },
            ctx,
        )
    });

    // Get the single terminal view in the workspace.
    let terminal_view = workspace.read(app, |workspace, ctx| {
        assert_eq!(workspace.tabs.len(), 1);
        workspace
            .active_tab_pane_group()
            .as_ref(ctx)
            .focused_session_view(ctx)
            .unwrap()
    });

    terminal_view.update(app, |view, ctx| {
        view.model.lock().block_list_mut().set_bootstrapped();
        view.attempt_to_share_session(
            SharedSessionScrollbackType::All,
            None,
            SessionSourceType::default(),
            false,
            ctx,
        );
    });

    workspace
}

// Creates a workspace as a viewer of a shared session.
fn mock_workspace_viewing_shared_session(app: &mut App) -> ViewHandle<Workspace> {
    // Create the workspace as a session-sharing sharer.
    let global_resource_handles = GlobalResourceHandles::mock(app);

    let (_, workspace) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        Workspace::new(
            global_resource_handles,
            None,
            NewWorkspaceSource::Empty {
                previous_active_window: None,
                shell: None,
            },
            ctx,
        )
    });

    // Get the single terminal view in the workspace.
    let terminal_view = workspace.read(app, |workspace, ctx| {
        assert_eq!(workspace.tabs.len(), 1);
        workspace
            .active_tab_pane_group()
            .as_ref(ctx)
            .focused_session_view(ctx)
            .unwrap()
    });

    terminal_view.update(app, |view, ctx| {
        view.on_session_share_joined(
            ParticipantId::new(),
            UserUid::new("mock_user_uid"),
            ReplicaId::random(),
            Box::new(ParticipantList::default()),
            SessionId::new(),
            SessionSourceType::default(),
            ctx,
        );
    });

    workspace
}

/// Disable the warn-before-quit setting. Because we don't fully bootstrap the shell in tests, this
/// is generally needed in tests that close tabs.
fn disable_quit_warning(app: &mut AppContext) {
    GeneralSettings::handle(app).update(app, |settings, ctx| {
        settings
            .show_warning_before_quitting
            .set_value(false, ctx)
            .expect("Failed to disable quit warning");
    });
}

fn get_newly_created_pane_id(panes: &PaneGroup, existing_ids: &[PaneId]) -> PaneId {
    panes
        .pane_ids()
        .find(|id| !existing_ids.contains(id))
        .unwrap()
}

fn split_pane_state(
    panes: &PaneGroup,
    pane_id: impl Into<PaneId>,
    ctx: &AppContext,
) -> SplitPaneState {
    // Split pane state is now inferred from the pane group's focus state
    panes
        .focus_state_handle()
        .as_ref(ctx)
        .split_pane_state_for(pane_id.into())
}

fn active_session_state(
    panes: &PaneGroup,
    pane_id: TerminalPaneId,
    ctx: &AppContext,
) -> ActiveSessionState {
    if panes
        .terminal_view_from_pane_id(pane_id, ctx)
        .expect("Not a terminal pane")
        .as_ref(ctx)
        .is_active_session(ctx)
    {
        ActiveSessionState::Active
    } else {
        ActiveSessionState::Inactive
    }
}

fn new_session_menu_label(item: &MenuItem<WorkspaceAction>) -> String {
    match item {
        MenuItem::Item(fields) => fields.label().to_string(),
        MenuItem::Separator => "---".to_string(),
        MenuItem::ItemsRow { items } => items
            .iter()
            .map(|fields| fields.label().to_string())
            .collect::<Vec<_>>()
            .join(" | "),
        MenuItem::Submenu { fields, .. } => fields.label().to_string(),
        MenuItem::Header { fields, .. } => fields.label().to_string(),
    }
}

fn reopen_closed_session_menu_item(
    menu_items: &[MenuItem<WorkspaceAction>],
) -> &MenuItemFields<WorkspaceAction> {
    match menu_items.last() {
        Some(MenuItem::Item(fields)) if fields.label() == "Reopen closed session" => fields,
        _ => panic!("expected Reopen closed session to be the last new-session menu item"),
    }
}

fn favorite_host_submenu() -> MenuItem<WorkspaceAction> {
    super::favorite_host_menu_item(
        &zaplex_cockpit::Favorite::new(zaplex_cockpit::FavoriteKind::Host, "node-dev", "devhost"),
        &[("node-dev".to_string(), "devhost".to_string())],
        false,
    )
}

#[test]
fn connections_registry_drives_favorite_launch_menu() {
    use diesel::Connection as _;
    use diesel_migrations::MigrationHarness as _;

    let mut conn = diesel::sqlite::SqliteConnection::establish(":memory:").unwrap();
    conn.run_pending_migrations(persistence::MIGRATIONS)
        .unwrap();

    let mut favorite_server_info = warp_ssh_manager::SshServerInfo::new_default(String::new());
    favorite_server_info.host = "remote.example.test".into();
    let favorite_server = warp_ssh_manager::SshRepository::create_server(
        &mut conn,
        None,
        "stale-display-name",
        &favorite_server_info,
    )
    .unwrap();

    let mut other_server_info = warp_ssh_manager::SshServerInfo::new_default(String::new());
    other_server_info.host = "other.example.test".into();
    warp_ssh_manager::SshRepository::create_server(
        &mut conn,
        None,
        "not-favorited",
        &other_server_info,
    )
    .unwrap();
    warp_ssh_manager::SshRepository::create_folder(&mut conn, None, "not-a-host").unwrap();

    let connection_nodes = warp_ssh_manager::SshRepository::list_nodes(&mut conn).unwrap();
    warp_ssh_manager::SshRepository::rename_node(&mut conn, &favorite_server.id, "renamed-remote")
        .unwrap();
    let registered_hosts = warp_ssh_manager::SshRepository::list_nodes(&mut conn)
        .unwrap()
        .into_iter()
        .filter(|node| matches!(node.kind, warp_ssh_manager::NodeKind::Server))
        .map(|node| (node.id, node.name))
        .collect::<Vec<_>>();

    App::test((), move |mut app| async move {
        initialize_app(&mut app);
        let (_, connections) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            crate::ssh_manager::SshManagerPanel::new(ctx)
        });
        connections.update(&mut app, |connections, ctx| {
            connections.set_nodes_for_test(connection_nodes);
            <crate::ssh_manager::SshManagerPanel as warpui::TypedActionView>::handle_action(
                connections,
                &crate::ssh_manager::SshManagerPanelAction::ToggleFavorite(
                    favorite_server.id.clone(),
                ),
                ctx,
            );
        });

        let favorites_store = crate::cockpit::favorites::FavoritesStore::handle(&app);
        let menu_items = favorites_store.read(&app, |store, _| {
            assert_eq!(store.items().len(), 1);
            assert_eq!(store.items()[0].label, "stale-display-name");
            assert!(store.contains(zaplex_cockpit::FavoriteKind::Host, &favorite_server.id));
            super::favorites_menu_items_from_sources(store, registered_hosts, false)
        });

        let favorite_submenus = menu_items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Submenu { fields, menu } => Some((fields, menu)),
                MenuItem::Item(_)
                | MenuItem::Separator
                | MenuItem::ItemsRow { .. }
                | MenuItem::Header { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            favorite_submenus.len(),
            1,
            "only the host toggled in Connections belongs in the menu"
        );
        let (fields, menu) = favorite_submenus[0];
        assert_eq!(
            fields.label(),
            "renamed-remote",
            "the connection registry owns the current host label"
        );
        assert!(matches!(
            menu.items()[0].item_on_select_action(),
            Some(WorkspaceAction::OpenSshTerminalByNode { node_id })
                if node_id == &favorite_server.id
        ));
        assert!(matches!(
            menu.items()[1].item_on_select_action(),
            Some(WorkspaceAction::OpenSpawnCard {
                registry_node_id: Some(node_id),
                host_id: None,
                host: Some(host),
                project: None,
            }) if node_id == &favorite_server.id && host == "renamed-remote"
        ));
    });
}

#[test]
fn favorite_host_submenu_offers_terminal_and_new_agent() {
    let MenuItem::Submenu { menu, .. } = favorite_host_submenu() else {
        panic!("a favorite host must open a right-hand submenu");
    };
    assert_eq!(menu.items().len(), 2);
    assert!(matches!(
        menu.items()[0].item_on_select_action(),
        Some(WorkspaceAction::OpenSshTerminalByNode { node_id }) if node_id == "node-dev"
    ));
    assert!(matches!(
        menu.items()[1].item_on_select_action(),
        Some(WorkspaceAction::OpenSpawnCard { .. })
    ));
}

#[test]
fn new_agent_submenu_opens_spawn_card_without_launching() {
    let MenuItem::Submenu { menu, .. } = favorite_host_submenu() else {
        panic!("a favorite host must open a right-hand submenu");
    };
    assert!(matches!(
        menu.items()[1].item_on_select_action(),
        Some(WorkspaceAction::OpenSpawnCard {
            registry_node_id: Some(node_id),
            host_id: None,
            host: Some(host),
            project: None,
        }) if node_id == "node-dev" && host == "devhost"
    ));
}

#[test]
fn stale_favorite_is_disabled_and_removable() {
    let favorite = zaplex_cockpit::Favorite::new(
        zaplex_cockpit::FavoriteKind::Host,
        "deleted-node",
        "old-devhost",
    );
    let mut items = super::favorite_host_menu_items(std::slice::from_ref(&favorite), &[], false);
    assert_eq!(items.len(), 1);
    let MenuItem::Submenu { fields, menu } = items.pop().unwrap() else {
        panic!("a stale favorite must remain visible as a submenu");
    };
    assert!(fields.label().contains("old-devhost"));
    assert_ne!(fields.label(), "old-devhost");
    assert_eq!(menu.items().len(), 2);
    let MenuItem::Item(unavailable) = &menu.items()[0] else {
        panic!("the stale target state must be a menu item");
    };
    assert!(unavailable.is_disabled());
    assert!(unavailable.on_select_action().is_none());
    let MenuItem::Item(remove) = &menu.items()[1] else {
        panic!("stale favorites need an explicit removal item");
    };
    assert!(!remove.is_disabled());
    assert!(matches!(
        menu.items()[1].item_on_select_action(),
        Some(WorkspaceAction::RemoveFavorite { kind, target })
            if *kind == zaplex_cockpit::FavoriteKind::Host && target == "deleted-node"
    ));
}

#[test]
fn protected_favorite_store_disables_stale_removal() {
    let favorite = zaplex_cockpit::Favorite::new(
        zaplex_cockpit::FavoriteKind::Host,
        "deleted-node",
        "old-devhost",
    );
    let MenuItem::Submenu { menu, .. } = super::favorite_host_menu_item(&favorite, &[], true)
    else {
        panic!("the protected stale favorite must remain visible");
    };
    let MenuItem::Item(remove) = &menu.items()[1] else {
        panic!("the protected stale favorite must retain its removal row");
    };
    assert!(remove.is_disabled());
    assert!(remove.on_select_action().is_none());
}

#[test]
fn removed_favorite_host_is_disabled_and_never_routed() {
    let favorite = zaplex_cockpit::Favorite::new(
        zaplex_cockpit::FavoriteKind::Host,
        "removed-node",
        "removed-host",
    );
    let MenuItem::Submenu { menu, .. } = super::favorite_host_menu_item(&favorite, &[], false)
    else {
        panic!("a removed favorite must remain explicitly removable");
    };
    assert!(menu.items().iter().all(|item| !matches!(
        item.item_on_select_action(),
        Some(WorkspaceAction::OpenSshTerminalByNode { .. } | WorkspaceAction::OpenSpawnCard { .. })
    )));
}

#[test]
fn automatic_host_registration_never_adds_menu_favorite() {
    let registered_hosts = vec![("node-dev".to_string(), "devhost".to_string())];
    assert!(super::favorite_host_menu_items(&[], &registered_hosts, false).is_empty());
}

#[test]
fn primary_sidebar_keeps_cockpit_and_connections_separate() {
    assert_eq!(
        super::primary_host_navigation_views(true),
        vec![ToolPanelView::Cockpit, ToolPanelView::SshManager]
    );
}

#[test]
fn connections_remains_available_when_cockpit_is_enabled() {
    let views = super::primary_host_navigation_views(true);
    assert!(views.contains(&ToolPanelView::SshManager));
    assert!(views.contains(&ToolPanelView::Cockpit));
    assert_eq!(views.len(), 2);
}

#[test]
fn test_tab_renaming_editor_selections() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        // Add second tab and rename both of them to prepare for the test
        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.rename_tab_internal(0, "short_title", ctx);
            let selected_text = workspace
                .tab_rename_editor
                .read(ctx, |editor, ctx| editor.selected_text(ctx));
            assert_eq!("short_title", selected_text);

            // Ensure that whatever is selected, is the full title and not the leftover from
            // the previous, shorter one.
            workspace.rename_tab_internal(1, "very_long_title_this_is", ctx);
            let selected_text = workspace
                .tab_rename_editor
                .read(ctx, |editor, ctx| editor.selected_text(ctx));
            assert_eq!("very_long_title_this_is", selected_text);

            // Ensure that if we escape, the current editor's contents is going to be cleared
            // as well.
            workspace.handle_tab_rename_editor_event(&Event::Escape, ctx);
            let selected_text = workspace
                .tab_rename_editor
                .read(ctx, |editor, ctx| editor.selected_text(ctx));
            assert_eq!("", selected_text);
        });
    });
}

#[test]
fn test_tab_renaming_editor_reset() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.rename_tab_internal(0, "short_title", ctx);
            workspace.rename_tab_internal(1, "very_long_title_this_is", ctx);

            // Ensure that when the editor is initially not empty, it will be cleared before a user renames a tab
            workspace.tab_rename_editor.update(ctx, |editor, ctx| {
                editor.insert_selected_text("some-text", ctx);
            });
            workspace.rename_tab_internal(1, "new_very_long_title", ctx);
            let selected_text: String = workspace
                .tab_rename_editor
                .read(ctx, |editor, ctx| editor.selected_text(ctx));
            assert_eq!("new_very_long_title", selected_text);
        });
    });
}

#[test]
fn test_set_active_tab_name() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);

            workspace.handle_action(
                &WorkspaceAction::SetActiveTabName("  Backend API  ".to_string()),
                ctx,
            );
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .display_title(ctx),
                "Backend API"
            );
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .custom_title(ctx)
                    .as_deref(),
                Some("Backend API")
            );

            workspace.handle_action(&WorkspaceAction::ActivateTab(0), ctx);
            assert_ne!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .custom_title(ctx)
                    .as_deref(),
                Some("Backend API")
            );

            workspace.handle_action(&WorkspaceAction::ActivateTab(1), ctx);
            workspace.handle_action(&WorkspaceAction::SetActiveTabName("   ".to_string()), ctx);
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .custom_title(ctx)
                    .as_deref(),
                Some("Backend API")
            );
        });
    });
}

#[test]
fn test_set_active_tab_name_clears_active_rename_editor_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.rename_tab_internal(0, "old title", ctx);
            assert!(workspace.current_workspace_state.is_tab_being_renamed());

            workspace.handle_action(
                &WorkspaceAction::SetActiveTabName("new title".to_string()),
                ctx,
            );

            assert!(!workspace.current_workspace_state.is_tab_being_renamed());
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .display_title(ctx),
                "new title"
            );
        });
    });
}

#[test]
fn test_set_active_tab_color() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            let active = workspace.active_tab_index;

            // Setting a color stores it as the manual selection and resolves to it.
            workspace.handle_action(
                &WorkspaceAction::SetActiveTabColor(SelectedTabColor::Color(
                    AnsiColorIdentifier::Magenta,
                )),
                ctx,
            );
            assert_eq!(
                workspace.tabs[active].selected_color,
                SelectedTabColor::Color(AnsiColorIdentifier::Magenta),
            );
            assert_eq!(
                workspace.tabs[active].color(),
                Some(AnsiColorIdentifier::Magenta),
            );

            // Replacing with a different color overwrites the previous selection.
            workspace.handle_action(
                &WorkspaceAction::SetActiveTabColor(SelectedTabColor::Color(
                    AnsiColorIdentifier::Green,
                )),
                ctx,
            );
            assert_eq!(
                workspace.tabs[active].selected_color,
                SelectedTabColor::Color(AnsiColorIdentifier::Green),
            );

            // `Cleared` explicitly suppresses any color (including a directory default).
            workspace.handle_action(
                &WorkspaceAction::SetActiveTabColor(SelectedTabColor::Cleared),
                ctx,
            );
            assert_eq!(
                workspace.tabs[active].selected_color,
                SelectedTabColor::Cleared,
            );
            assert_eq!(workspace.tabs[active].color(), None);

            // `Unset` removes the manual override so a directory default could apply.
            // With no directory default configured, the resolved color is still `None`.
            workspace.handle_action(
                &WorkspaceAction::SetActiveTabColor(SelectedTabColor::Unset),
                ctx,
            );
            assert_eq!(
                workspace.tabs[active].selected_color,
                SelectedTabColor::Unset,
            );
            assert_eq!(workspace.tabs[active].color(), None);

            // Action targets the active tab — switching to tab 0 leaves the second tab
            // unaffected.
            workspace.handle_action(&WorkspaceAction::ActivateTab(0), ctx);
            workspace.handle_action(
                &WorkspaceAction::SetActiveTabColor(SelectedTabColor::Color(
                    AnsiColorIdentifier::Blue,
                )),
                ctx,
            );
            assert_eq!(
                workspace.tabs[0].selected_color,
                SelectedTabColor::Color(AnsiColorIdentifier::Blue),
            );
            assert_eq!(
                workspace.tabs[active].selected_color,
                SelectedTabColor::Unset,
            );
        });
    });
}

#[test]
fn test_workspace_sessions_retrieves_tabs() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let pane_id = workspace
                .get_pane_group_view(0)
                .map(|tab| tab.read(ctx, |tab, _ctx| tab.pane_id_by_index(0).unwrap()))
                .expect("WindowId was not retrieved.");

            assert!(workspace
                .workspace_sessions(ctx.window_id(), ctx)
                .any(|x| { x.pane_view_locator().pane_id == pane_id }));

            // Add a tab and check if workspace_sessions finds the second session from the new tab.
            workspace.add_terminal_tab(false, ctx);
            let new_pane_id = workspace
                .get_pane_group_view(1)
                .map(|tab| tab.read(ctx, |tab, _ctx| tab.pane_id_by_index(0).unwrap()))
                .expect("WindowId was not retrieved.");

            assert!(workspace
                .workspace_sessions(ctx.window_id(), ctx)
                .any(|x| { x.pane_view_locator().pane_id == new_pane_id }));
        });
    });
}

#[test]
fn test_workspace_sessions_retrieves_panes() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            // Add a new split pane to the right.
            if let Some(tab_view) = workspace.get_pane_group_view(0) {
                tab_view.update(ctx, |view, ctx| {
                    view.handle_action(&PaneGroupAction::Add(Direction::Right), ctx);
                })
            }

            // Get the EntityId of the new pane added to the current tab.
            let new_pane_id = workspace
                .get_pane_group_view(0)
                .map(|tab| tab.read(ctx, |tab, _ctx| tab.pane_id_by_index(1).unwrap()))
                .expect("WindowId was not retrieved.");
            assert!(workspace
                .workspace_sessions(ctx.window_id(), ctx)
                .any(|x| { x.pane_view_locator().pane_id == new_pane_id }));
        });
    });
}

fn number_of_shared_sessions_in_tab(
    workspace: &Workspace,
    index: usize,
    ctx: &AppContext,
) -> usize {
    workspace
        .get_pane_group_view(index)
        .map_or(0, |view| view.as_ref(ctx).number_of_shared_sessions(ctx))
}

/// Sets up the workspace with three tabs. The middle tab has two panes, where one is shared.
fn setup_session_sharing_test(workspace: &ViewHandle<Workspace>, app: &mut App) -> PaneId {
    let shared_pane_id = workspace.update(app, |workspace, ctx| {
        workspace.add_terminal_tab(false, ctx);
        workspace.add_terminal_tab(false, ctx);

        let tab_view = workspace.get_pane_group_view(1).unwrap();

        tab_view.update(ctx, |view, ctx| {
            assert_eq!(view.pane_count(), 1);
            view.focused_session_view(ctx)
                .unwrap()
                .update(ctx, |terminal, ctx| {
                    terminal.attempt_to_share_session(
                        SharedSessionScrollbackType::None,
                        None,
                        SessionSourceType::default(),
                        false,
                        ctx,
                    );
                });

            view.handle_action(&PaneGroupAction::Add(Direction::Right), ctx);
            assert_eq!(view.pane_count(), 2);

            view.pane_id_by_index(0).unwrap()
        })
    });

    workspace.read(app, |workspace, ctx| {
        assert_eq!(number_of_shared_sessions_in_tab(workspace, 1, ctx), 1);

        // Confirmation dialog starts not open.
        assert!(
            !workspace
                .current_workspace_state
                .is_close_session_confirmation_dialog_open
        );
    });

    shared_pane_id
}

#[test]
fn test_close_tab_confirmation_dialog() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(disable_quit_warning);

        let workspace = mock_workspace(&mut app);
        setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let first_tab_id = workspace.get_pane_group_view(0).unwrap().id();

            // Trying to close tab with a shared pane opens dialog.
            workspace.handle_action(&WorkspaceAction::CloseTab(1), ctx);
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // User clicking cancel closes dialog.
            workspace.handle_close_session_confirmation_dialog_event(
                &CloseSessionConfirmationEvent::Cancel,
                ctx,
            );
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // Trying to close tab without a shared pane goes through without dialog.
            workspace.handle_action(&WorkspaceAction::CloseTab(2), ctx);
            assert_eq!(workspace.tab_count(), 2);
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // Close the tab with the shared pane.
            workspace.handle_action(&WorkspaceAction::CloseTab(1), ctx);
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            workspace.handle_close_session_confirmation_dialog_event(
                &CloseSessionConfirmationEvent::CloseSession {
                    dont_show_again: false,
                    open_confirmation_source: OpenDialogSource::CloseTab { tab_index: 1 },
                },
                ctx,
            );
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            assert_eq!(workspace.tab_count(), 1);
            assert_eq!(workspace.get_pane_group_view(0).unwrap().id(), first_tab_id);
        });
    });
}

#[test]
fn test_close_pane_confirmation_dialog() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let shared_pane_id = setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let shared_pane_group_id = workspace.get_pane_group_view(1).unwrap().id();

            // User tries to close shared pane, dialog comes up.
            workspace.handle_file_tree_event(
                workspace.get_pane_group_view(1).unwrap().clone(),
                &pane_group::Event::CloseSharedSessionPaneRequested {
                    pane_id: shared_pane_id,
                },
                ctx,
            );
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // User confirms.
            workspace.handle_close_session_confirmation_dialog_event(
                &CloseSessionConfirmationEvent::CloseSession {
                    dont_show_again: false,
                    open_confirmation_source: OpenDialogSource::ClosePane {
                        pane_group_id: shared_pane_group_id,
                        pane_id: shared_pane_id,
                    },
                },
                ctx,
            );
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            assert_eq!(number_of_shared_sessions_in_tab(workspace, 1, ctx), 0);
            let remaining_pane_id = workspace
                .get_pane_group_view_with_id(shared_pane_group_id)
                .unwrap()
                .as_ref(ctx)
                .pane_id_by_index(0)
                .unwrap();
            assert_ne!(remaining_pane_id, shared_pane_id);
            assert_eq!(workspace.tab_count(), 3);
        });
    });
}

#[test]
fn test_reopen_closed_shared_tab() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let shared_pane_group = workspace.get_pane_group_view(1).unwrap().clone();

            // Close the tab with the shared pane.
            workspace.close_tab(1, true, true, ctx);
            assert_eq!(workspace.tab_count(), 2);

            // Restore the shared tab.
            workspace.restore_closed_tab(1, TabData::new(shared_pane_group.to_owned()), ctx);
        });
        // Restored tab should no longer be shared.
        workspace.read(&app, |workspace, ctx| {
            let pane_group = workspace.get_pane_group_view(1).unwrap();
            assert!(!pane_group.as_ref(ctx).is_terminal_pane_being_shared(ctx));
            assert_eq!(workspace.tab_count(), 3);
        })
    });
}

#[test]
fn test_tab_pinning_keeps_a_stable_prefix_and_restores_it() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let snapshot = workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            assert_eq!(workspace.tab_count(), 3);

            let second_id = workspace.tabs[1].pane_group.id();
            let third_id = workspace.tabs[2].pane_group.id();
            assert_eq!(workspace.set_tab_pinned(1, true, ctx), Some(0));
            assert_eq!(workspace.set_tab_pinned(2, true, ctx), Some(1));
            assert_eq!(workspace.active_tab_index(), 1);
            assert_eq!(workspace.tabs[0].pane_group.id(), second_id);
            assert_eq!(workspace.tabs[1].pane_group.id(), third_id);
            assert_eq!(
                workspace
                    .tabs
                    .iter()
                    .map(|tab| tab.is_pinned)
                    .collect::<Vec<_>>(),
                vec![true, true, false]
            );

            workspace.move_tab(1, TabMovement::Right, ctx);
            assert_eq!(workspace.tabs[1].pane_group.id(), third_id);

            workspace.add_terminal_tab(false, ctx);
            assert_eq!(workspace.active_tab_index(), 2);
            assert_eq!(
                workspace
                    .tabs
                    .iter()
                    .map(|tab| tab.is_pinned)
                    .collect::<Vec<_>>(),
                vec![true, true, false, false]
            );

            assert_eq!(workspace.set_tab_pinned(0, false, ctx), Some(1));
            assert_eq!(workspace.tabs[1].pane_group.id(), second_id);
            assert_eq!(
                workspace
                    .tabs
                    .iter()
                    .map(|tab| tab.is_pinned)
                    .collect::<Vec<_>>(),
                vec![true, false, false, false]
            );

            workspace.snapshot(ctx.window_id(), false, ctx)
        });

        let restored = restored_workspace(&mut app, snapshot);
        restored.read(&app, |workspace, _| {
            assert_eq!(workspace.active_tab_index(), 2);
            assert_eq!(
                workspace
                    .tabs
                    .iter()
                    .map(|tab| tab.is_pinned)
                    .collect::<Vec<_>>(),
                vec![true, false, false, false]
            );
        });
    });
}

#[test]
fn restored_legacy_pin_order_keeps_the_original_active_tab() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let mut snapshot = workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            workspace.activate_tab(0, ctx);
            workspace.snapshot(ctx.window_id(), false, ctx)
        });
        snapshot.active_tab_index = 0;
        snapshot.tabs[0].selected_color = SelectedTabColor::Color(AnsiColorIdentifier::Red);
        snapshot.tabs[1].is_pinned = true;

        let restored = restored_workspace(&mut app, snapshot);
        restored.read(&app, |workspace, _| {
            assert_eq!(workspace.active_tab_index(), 1);
            assert_eq!(
                workspace.tabs[workspace.active_tab_index()].selected_color,
                SelectedTabColor::Color(AnsiColorIdentifier::Red)
            );
            assert_eq!(
                workspace
                    .tabs
                    .iter()
                    .map(|tab| tab.is_pinned)
                    .collect::<Vec<_>>(),
                vec![true, false, false]
            );
        });
    });
}

#[test]
fn test_pinned_tab_always_requires_close_confirmation() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(disable_quit_warning);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            assert_eq!(workspace.set_tab_pinned(1, true, ctx), Some(0));

            workspace.handle_action(&WorkspaceAction::CloseTab(0), ctx);

            assert_eq!(workspace.tab_count(), 2);
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
        });
    });
}

#[test]
fn test_close_other_tabs_confirmation_dialog() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let last_tab_id = workspace.get_pane_group_view(2).unwrap().id();

            // User tries to close other tabs choosing non-shared tab, dialog comes up.
            workspace.handle_action(&WorkspaceAction::CloseOtherTabs(2), ctx);
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // User confirms.
            workspace.handle_close_session_confirmation_dialog_event(
                &CloseSessionConfirmationEvent::CloseSession {
                    dont_show_again: false,
                    open_confirmation_source: OpenDialogSource::CloseOtherTabs { tab_index: 2 },
                },
                ctx,
            );
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            assert_eq!(workspace.tab_count(), 1);
            assert_eq!(workspace.get_pane_group_view(0).unwrap().id(), last_tab_id);
        });
    });
}

#[test]
fn test_close_tabs_right_confirmation_dialog() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let first_tab_id = workspace.get_pane_group_view(0).unwrap().id();

            // User tries to close all tabs right of the left-most tab, dialog comes up.
            workspace.handle_action(&WorkspaceAction::CloseTabsRight(0), ctx);
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // User confirms.
            workspace.handle_close_session_confirmation_dialog_event(
                &CloseSessionConfirmationEvent::CloseSession {
                    dont_show_again: false,
                    open_confirmation_source: OpenDialogSource::CloseTabsDirection {
                        tab_index: 0,
                        direction: TabMovement::Right,
                    },
                },
                ctx,
            );
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            assert_eq!(workspace.tab_count(), 1);
            assert_eq!(workspace.get_pane_group_view(0).unwrap().id(), first_tab_id);
        });
    });
}

#[test]
fn test_confirmation_dialog_dont_show_again() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(disable_quit_warning);

        let workspace = mock_workspace(&mut app);
        setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            // Close the tab with the shared pane, dialog comes up
            workspace.handle_action(&WorkspaceAction::CloseTab(1), ctx);
            assert!(
                workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );

            // User confirms, checking "Don't show again".
            workspace.handle_close_session_confirmation_dialog_event(
                &CloseSessionConfirmationEvent::CloseSession {
                    dont_show_again: true,
                    open_confirmation_source: OpenDialogSource::CloseTab { tab_index: 1 },
                },
                ctx,
            );
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            assert_eq!(workspace.tab_count(), 2);

            // Share the first tab
            let tab_view = workspace.get_pane_group_view(0).unwrap();
            tab_view.update(ctx, |view, ctx| {
                view.terminal_manager(0, ctx)
                    .unwrap()
                    .as_ref(ctx)
                    .model()
                    .lock()
                    .set_shared_session_status(SharedSessionStatus::ActiveSharer);
            });

            // Close the shared tab. No dialog should come up and action should go through.
            workspace.handle_action(&WorkspaceAction::CloseActiveTab, ctx);
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
            assert_eq!(workspace.tab_count(), 1);
        });
    });
}

#[test]
fn test_close_last_tab_skip_confirmation() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(disable_quit_warning);

        let workspace = mock_workspace(&mut app);
        setup_session_sharing_test(&workspace, &mut app);

        workspace.update(&mut app, |workspace, ctx| {
            // Close the non-shared tabs so there's just one shared tab left.
            workspace.handle_action(&WorkspaceAction::CloseTab(2), ctx);
            workspace.handle_action(&WorkspaceAction::CloseTab(0), ctx);
            assert_eq!(workspace.tab_count(), 1);
            // Close the last remaining tab with the shared pane, no dialog should come up because
            // we're going to close the window and there's already a confirmation on window close.
            workspace.handle_action(&WorkspaceAction::CloseActiveTab, ctx);
            assert!(
                !workspace
                    .current_workspace_state
                    .is_close_session_confirmation_dialog_open
            );
        });
    });
}

#[test]
fn test_notebook_pane_tracking() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            // Add a new notebook pane.
            workspace.open_notebook(
                &NotebookSource::New {
                    title: None,
                    owner: Owner::mock_current_user(),
                    initial_folder_id: None,
                },
                &ZaplexDriveObjectSettings::default(),
                ctx,
                true,
            );

            // Get the ID of the new notebook.
            let pane_group = workspace
                .get_pane_group_view(0)
                .expect("Pane group does not exist")
                .clone();
            let notebook_view = pane_group
                .as_ref(ctx)
                .notebook_view_at_pane_index(0, ctx)
                .expect("Notebook view was not created")
                .clone();
            let notebook_pane_id = pane_group
                .as_ref(ctx)
                .pane_id_from_index(0)
                .expect("Notebook view should have been created");
            let notebook_id = notebook_view
                .as_ref(ctx)
                .notebook_id(ctx)
                .expect("Notebook should have an ID");

            // The notebook should be registered with the NotebookManager.
            let (window, locator) = NotebookManager::as_ref(ctx)
                .find_pane(&NotebookSource::Existing(notebook_id))
                .expect("Notebook pane should be registered");
            assert_eq!(window, ctx.window_id());
            assert_eq!(
                locator,
                PaneViewLocator {
                    pane_group_id: pane_group.id(),
                    pane_id: notebook_pane_id,
                }
            );

            // Re-opening the notebook should not create a new view.
            workspace.open_notebook(
                &NotebookSource::Existing(notebook_id),
                &ZaplexDriveObjectSettings::default(),
                ctx,
                true,
            );
            assert_eq!(
                ctx.views_of_type::<NotebookView>(ctx.window_id()),
                Some(vec![notebook_view])
            );

            // Finally, closing the notebook pane should de-register it.
            pane_group.update(ctx, |pane_group, ctx| {
                pane_group.handle_action(&PaneGroupAction::RemoveActive, ctx)
            });
            assert_eq!(
                NotebookManager::handle(ctx)
                    .as_ref(ctx)
                    .find_pane(&NotebookSource::Existing(notebook_id)),
                None
            );
        });
    });
}

#[test]
fn test_set_active_terminal_input_contents_and_focus_app() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let initial_buffer_contents = workspace
                .get_active_input_view_handle(ctx)
                .map(|input_view_handle| input_view_handle.as_ref(ctx).buffer_text(ctx))
                .expect("There should be an active input view");
            assert_eq!(
                "", initial_buffer_contents,
                "initial active input should be empty"
            );

            workspace.set_active_terminal_input_contents_and_focus_app("foobar", ctx);

            assert_eq!(
                "foobar",
                workspace
                    .get_active_input_view_handle(ctx)
                    .map(|input_view_handle| input_view_handle.as_ref(ctx).buffer_text(ctx))
                    .expect("There should be an active input view")
            );
            assert!(ctx.windows().app_is_active());
        });
    });
}

/// Ensures that the terminal model is destroyed when it is no longer needed.
/// This is only a "workspace" test because we want to mimic what a normal
/// user would do and expect (e.g. close a tab and expect that its backing
/// data is correctly deallocated).
///
/// TODO(suraj): we may also want to investigate a more "real" integration test
/// that inspects the application process's overall memory consumption
/// instead of just the terminal model, but this is not easy because
/// 1. we want to measure non-shared memory (i.e. the "memory" value in Activity Monitor)
///    which is not easy; it's easier to measure "real memory" or RSS, but that includes
///    shared memory across processes.
/// 2. the test might be flaky depending on how much memory is actually allocated vs
///    freed up (not something easily controlled).
///
/// For now, this test is still useful because the terminal model is one of the largest data structures
/// maintained by our app, so we want to ensure we're not introducing regressions that cause it to not
/// be deallocated correctly.
#[test]
fn test_terminal_model_isnt_leaked() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Turn off undo-close so that we don't need to wait for deallocation.
        UndoCloseSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .enabled
                .set_value(false, ctx)
                .expect("Can turn off undo-close via settings.")
        });

        let workspace = mock_workspace(&mut app);

        let terminal_model = workspace.update(&mut app, |workspace, ctx| {
            // Add another tab so that the workspace isn't destroyed when we close the tab.
            workspace.add_terminal_tab(false, ctx);

            // Get a weak reference to the model.
            let model = workspace.get_active_session_terminal_model(ctx).unwrap();
            Arc::downgrade(&model)
        });

        workspace.update(&mut app, |workspace, ctx| {
            // Remove the tab. This should destroy the corresponding terminal view.
            workspace.remove_tab(workspace.active_tab_index(), true, true, ctx);
        });
        // For some reason, the update call above results in more pending effects, one of which
        // contains the actual logic that drops the `TerminalModel`.
        app.update(|_| ());

        // If we can't upgrade the weak reference, that means it was in fact destructed.
        assert!(
            terminal_model.upgrade().is_none(),
            "The terminal model should not exist once the tab is closed."
        )
    });
}

#[test]
fn test_open_or_toggle_warp_drive() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // First, unconditionally open Zaplex Drive as a system action. WD should be open and welcome tips should not have opening zap drive.
            workspace.open_or_toggle_warp_drive(
                false, /* toggle */
                false, /* explicit_user_action */
                ctx,
            );
            assert!(
                workspace.current_workspace_state.is_warp_drive_open,
                "Zaplex Drive should be open"
            );
            assert!(
                !workspace
                    .tips_completed
                    .as_ref(ctx)
                    .features_used
                    .contains(&Tip::Action(TipAction::ZaplexDrive)),
                "Zaplex drive welcome tip should not be completed"
            );

            // Next, toggle zap drive as a user action. WD should be closed and tip should not be filled out.
            workspace.open_or_toggle_warp_drive(
                true, /* toggle */
                true, /* explicit_user_action */
                ctx,
            );
            assert!(
                !workspace.current_workspace_state.is_warp_drive_open,
                "Zaplex Drive should be closed"
            );
            assert!(
                !workspace
                    .tips_completed
                    .as_ref(ctx)
                    .features_used
                    .contains(&Tip::Action(TipAction::ZaplexDrive)),
                "Zaplex drive welcome tip should not be completed"
            );

            // Finally, toggle zap drive again as a user action. WD should be open and tip filled out.
            workspace.open_or_toggle_warp_drive(
                true, /* toggle */
                true, /* explicit_user_action */
                ctx,
            );
            assert!(
                workspace.current_workspace_state.is_warp_drive_open,
                "Zaplex Drive should be open"
            );
            assert!(
                workspace
                    .tips_completed
                    .as_ref(ctx)
                    .features_used
                    .contains(&Tip::Action(TipAction::ZaplexDrive)),
                "Zaplex drive welcome tip should not be completed"
            );
        });
    });
}

#[test]
fn test_view_only_session() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Trying to open command search
        let workspace = mock_workspace_viewing_shared_session(&mut app);
        workspace.update(&mut app, |workspace: &mut Workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::ShowCommandSearch(Default::default()), ctx);
        });

        // Ensure command search doesn't work for read-only shared sessions
        workspace.read(&app, |workspace, _ctx| {
            assert!(!workspace.current_workspace_state.is_command_search_open);
        });
    });
}

#[test]
fn test_server_token_compatibility_finds_restored_local_conversation() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let token = ServerConversationToken::new("restored-token".to_string());
        let conversation_id = history_model.update(&mut app, |model, ctx| {
            let mut conversation = AIConversation::new(false);
            conversation.set_server_conversation_token(token.as_str().to_string());
            let conversation_id = conversation.id();
            model.restore_conversations(EntityId::new(), vec![conversation], ctx);
            conversation_id
        });

        app.read(|ctx| {
            assert_eq!(
                Workspace::find_local_conversation_id_by_server_token(&token, ctx),
                Some(conversation_id),
            );
        });
    });
}

#[test]
fn test_server_token_compatibility_ignores_unknown_token() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let token = ServerConversationToken::new("missing-token".to_string());

        app.read(|ctx| {
            assert_eq!(
                Workspace::find_local_conversation_id_by_server_token(&token, ctx),
                None,
            );
        });
    });
}

#[test]
// This tests the end-to-end behavior to correctly switch focus among panels.
// (The only panels that can be focused currently are WD, workspace, & AI assistant.)
fn test_switch_focus_panels() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |view, ctx| {
            view.focus_active_tab(ctx);
        });
        workspace.update(&mut app, |view, ctx| {
            assert!(
                view.active_tab_pane_group().is_self_or_child_focused(ctx),
                "Expected terminal to be focused"
            );
        });

        // Shift focus from terminal to left panel when WD is open
        workspace.update(&mut app, |view, ctx| {
            view.current_workspace_state.is_warp_drive_open = true;
            view.focus_left_panel(ctx);
        });
        workspace.update(&mut app, |view, ctx| {
            assert!(
                view.left_panel_view.is_self_or_child_focused(ctx),
                "Expected Zaplex Drive panel to be focused"
            );
        });

        // Shift focus from WD to left panel when AI panel is open
        workspace.update(&mut app, |view, ctx| {
            view.current_workspace_state.is_ai_assistant_panel_open = true;
            view.focus_left_panel(ctx);
        });
        workspace.update(&mut app, |view, ctx| {
            assert!(
                view.ai_assistant_panel.is_self_or_child_focused(ctx),
                "Expected AI panel to be focused"
            );
        });

        // Shift focus from AI panel to left panel (terminal)
        workspace.update(&mut app, |view, ctx| {
            view.focus_left_panel(ctx);
        });
        workspace.update(&mut app, |_view, ctx| {
            assert!(
                workspace.is_self_or_child_focused(ctx),
                "Expected terminal to be focused"
            );
        });

        // Shift focus from workspace to right panel when AI assistant is open
        workspace.update(&mut app, |view, ctx| {
            view.current_workspace_state.is_ai_assistant_panel_open = true;
            view.focus_right_panel(ctx);
        });
        workspace.update(&mut app, |view, ctx| {
            assert!(
                view.ai_assistant_panel.is_self_or_child_focused(ctx),
                "Expected AI panel to be focused"
            );
        });

        // Shift focus from WD to right panel (terminal)
        workspace.update(&mut app, |view, ctx| {
            view.focus_right_panel(ctx);
        });
        workspace.update(&mut app, |_view, ctx| {
            assert!(
                workspace.is_self_or_child_focused(ctx),
                "Expected terminal to be focused"
            );
        });
    });
}

#[test]
fn test_focus_notebook() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let pane_group = workspace.read(&app, |workspace, _ctx| {
            workspace
                .get_pane_group_view(0)
                .expect("should have pane group for tab 0")
                .clone()
        });

        let first_terminal_id = pane_group.read(&app, |panes, _ctx| {
            get_newly_created_pane_id(panes, &[])
                .as_terminal_pane_id()
                .expect("should be a terminal pane")
        });

        let notebook_id = pane_group.update(&mut app, |panes, ctx| {
            // Add a notebook to the left.
            let notebook_view = ctx.add_typed_action_view(NotebookView::new);
            panes.add_pane_with_direction(
                Direction::Left,
                NotebookPane::new(notebook_view, ctx),
                true, /* focus_new_pane */
                ctx,
            );
            get_newly_created_pane_id(panes, &[first_terminal_id.into()])
        });

        // The new pane should be focused, but the terminal is still the active session.
        pane_group.read(&app, |panes, ctx| {
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(panes.active_session_id(ctx), Some(first_terminal_id));
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert_eq!(
                active_session_state(panes, first_terminal_id, ctx),
                ActiveSessionState::Active
            );
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Focused)
            );
        });

        // Add a terminal below.
        let second_terminal_id = pane_group.update(&mut app, |panes, ctx| {
            panes.add_terminal_pane(Direction::Down, None, ctx);
            get_newly_created_pane_id(panes, &[first_terminal_id.into(), notebook_id])
                .as_terminal_pane_id()
                .expect("should be a terminal pane")
        });

        // The new terminal should be both focused and the active session.
        pane_group.read(&app, |panes, ctx| {
            assert_eq!(panes.focused_pane_id(ctx), second_terminal_id.into());
            assert_eq!(panes.active_session_id(ctx), Some(second_terminal_id));
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert_eq!(
                active_session_state(panes, first_terminal_id, ctx),
                ActiveSessionState::Inactive
            );
            assert_eq!(
                split_pane_state(panes, second_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Focused)
            );
            assert_eq!(
                active_session_state(panes, second_terminal_id, ctx),
                ActiveSessionState::Active
            );
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
        });

        // Close the new terminal.
        pane_group.update(&mut app, |panes, ctx| {
            panes.close_pane(second_terminal_id.into(), ctx);
        });

        // Focus should switch to the notebook, and the first terminal session
        // will activate.
        pane_group.read(&app, |panes, ctx| {
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(panes.active_session_id(ctx), Some(first_terminal_id));
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Focused)
            );
            assert_eq!(
                active_session_state(panes, first_terminal_id, ctx),
                ActiveSessionState::Active
            );
        });
    })
}

#[test]
fn test_close_active_session() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let pane_group = workspace.read(&app, |workspace, _ctx| {
            workspace
                .get_pane_group_view(0)
                .expect("should have pane group for tab 0")
                .clone()
        });

        let first_terminal_id = pane_group.read(&app, |panes, _ctx| {
            get_newly_created_pane_id(panes, &[])
                .as_terminal_pane_id()
                .expect("should be a terminal pane")
        });

        // Add a terminal above.
        let second_terminal_id = pane_group.update(&mut app, |panes, ctx| {
            panes.add_terminal_pane(Direction::Up, None, ctx);
            get_newly_created_pane_id(panes, &[first_terminal_id.into()])
                .as_terminal_pane_id()
                .expect("should be a terminal pane")
        });

        let notebook_id = pane_group.update(&mut app, |panes, ctx| {
            // Add a notebook to the left.
            let notebook_view = ctx.add_typed_action_view(NotebookView::new);
            panes.add_pane_with_direction(
                Direction::Left,
                NotebookPane::new(notebook_view, ctx),
                true, /* focus_new_pane */
                ctx,
            );
            get_newly_created_pane_id(
                panes,
                &[first_terminal_id.into(), second_terminal_id.into()],
            )
        });

        pane_group.read(&app, |panes, ctx| {
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(panes.active_session_id(ctx), Some(second_terminal_id));
        });

        pane_group.update(&mut app, |panes, ctx| {
            // Close the active session, which should leave the notebook focused and activate the
            // remaining session.
            panes.close_pane(second_terminal_id.into(), ctx);
        });

        pane_group.read(&app, |panes, ctx| {
            assert_eq!(panes.focused_pane_id(ctx), notebook_id);
            assert_eq!(panes.active_session_id(ctx), Some(first_terminal_id));
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert_eq!(
                active_session_state(panes, first_terminal_id, ctx),
                ActiveSessionState::Active
            );
        });

        pane_group.update(&mut app, |panes, ctx| {
            // Now, focus the remaining session, which should keep it activated.
            panes.focus_pane_by_id(first_terminal_id.into(), ctx);
        });

        pane_group.read(&app, |panes, ctx| {
            assert_eq!(panes.focused_pane_id(ctx), first_terminal_id.into());
            assert_eq!(panes.active_session_id(ctx), Some(first_terminal_id));
            assert_eq!(
                split_pane_state(panes, first_terminal_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Focused)
            );
            assert_eq!(
                split_pane_state(panes, notebook_id, ctx),
                SplitPaneState::InSplitPane(PaneState::Unfocused)
            );
            assert_eq!(
                active_session_state(panes, first_terminal_id, ctx),
                ActiveSessionState::Active
            );
        });
    });
}

fn set_left_panel_visibility_across_tabs(is_enabled: bool, ctx: &mut ViewContext<Workspace>) {
    WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
        window_settings
            .left_panel_visibility_across_tabs
            .set_value(is_enabled, ctx)
            .expect("Failed to update left_panel_visibility_across_tabs setting");
    });
}

#[test]
fn markdown_viewer_window_starts_without_vertical_tabs_sidebar() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });

            assert!(!Workspace::initial_vertical_tabs_panel_open(
                &NewWorkspaceSource::NotebookFromFilePath {
                    file_path: Some(PathBuf::from("README.md")),
                },
                ctx,
            ));
        });
    });
}

fn add_get_started_tab(workspace: &mut Workspace, ctx: &mut ViewContext<Workspace>) {
    workspace.add_tab_with_pane_layout(
        PanesLayout::Snapshot(Box::new(PaneNodeSnapshot::Leaf(LeafSnapshot {
            is_focused: true,
            custom_vertical_tabs_title: None,
            contents: LeafContents::GetStarted,
        }))),
        Arc::new(HashMap::<PaneUuid, Vec<SerializedBlockListItem>>::new()),
        None,
        ctx,
    );
}

fn find_terminal_tab_index(workspace: &Workspace, ctx: &AppContext) -> usize {
    workspace
        .tabs
        .iter()
        .position(|tab| tab.pane_group.as_ref(ctx).has_terminal_panes())
        .expect("Expected a terminal tab")
}

fn find_non_following_tab_index(workspace: &Workspace, ctx: &AppContext) -> usize {
    workspace
        .tabs
        .iter()
        .position(|tab| {
            !Workspace::should_enable_file_tree_and_global_search_for_pane_group(
                tab.pane_group.as_ref(ctx),
            )
        })
        .expect("Expected a non-following tab")
}

#[test]
fn test_left_panel_window_scoped_reconciles_between_terminal_tabs_when_enabled() {
    let _conversation_list_guard =
        FeatureFlag::AgentViewConversationListView.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            set_left_panel_visibility_across_tabs(true, ctx);

            workspace.add_terminal_tab(false, ctx);

            workspace.activate_tab(0, ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(!workspace.left_panel_open);

            workspace.open_left_panel(ctx);
            assert!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(workspace.left_panel_open);

            workspace.activate_tab(1, ctx);
            assert!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );

            workspace.close_left_panel(ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(!workspace.left_panel_open);

            workspace.activate_tab(0, ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
        });
    });
}

#[test]
fn test_left_panel_window_scoped_non_following_tab_does_not_reconcile_but_updates_window_state() {
    let _conversation_list_guard =
        FeatureFlag::AgentViewConversationListView.override_enabled(false);
    let _get_started_guard = FeatureFlag::GetStartedTab.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            set_left_panel_visibility_across_tabs(true, ctx);

            // Establish window-scoped desired state = open on a terminal tab.
            workspace.open_left_panel(ctx);
            assert!(workspace.left_panel_open);

            // Create a non-following tab (e.g. Get Started), which should not auto-open even though
            // the window state is open.
            add_get_started_tab(workspace, ctx);
            let non_following_tab_index = find_non_following_tab_index(workspace, ctx);
            workspace.activate_tab(non_following_tab_index, ctx);

            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(workspace.left_panel_open);

            // User actions in the non-following tab still update window state.
            workspace.open_left_panel(ctx);
            assert!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(workspace.left_panel_open);

            workspace.close_left_panel(ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(!workspace.left_panel_open);

            // The window state should reconcile back onto following tabs.
            let terminal_tab_index = find_terminal_tab_index(workspace, ctx);
            workspace.activate_tab(terminal_tab_index, ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );

            // But toggling the window state from a following tab should not auto-open the
            // non-following tab.
            workspace.open_left_panel(ctx);
            assert!(workspace.left_panel_open);

            workspace.activate_tab(non_following_tab_index, ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
            assert!(workspace.left_panel_open);
        });
    });
}

#[test]
fn test_left_panel_window_scoped_disabled_keeps_per_tab_state() {
    let _conversation_list_guard =
        FeatureFlag::AgentViewConversationListView.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            set_left_panel_visibility_across_tabs(false, ctx);

            workspace.add_terminal_tab(false, ctx);

            // Open left panel on tab 0.
            workspace.activate_tab(0, ctx);
            workspace.open_left_panel(ctx);
            assert!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );

            // With window scoping disabled, switching tabs should not reconcile the open state.
            workspace.activate_tab(1, ctx);
            assert!(
                !workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );

            // Each tab can be toggled independently.
            workspace.open_left_panel(ctx);
            assert!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );

            workspace.activate_tab(0, ctx);
            assert!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .left_panel_open
            );
        });
    });
}

#[test]
fn test_vertical_tabs_panel_visibility_restores_from_window_snapshot() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
        });

        let workspace = mock_workspace(&mut app);

        let closed_snapshot = workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = false;
            workspace.snapshot(ctx.window_id(), false, ctx)
        });
        let open_snapshot = workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = true;
            workspace.snapshot(ctx.window_id(), false, ctx)
        });

        let restored_closed = restored_workspace(&mut app, closed_snapshot);
        let restored_open = restored_workspace(&mut app, open_snapshot);

        restored_closed.read(&app, |workspace, _| {
            assert!(!workspace.vertical_tabs_panel_open);
        });
        restored_open.read(&app, |workspace, _| {
            assert!(workspace.vertical_tabs_panel_open);
        });
    });
}

#[test]
fn test_vertical_tabs_panel_restored_open_when_show_in_restored_windows_enabled() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
                report_if_error!(settings
                    .show_vertical_tab_panel_in_restored_windows
                    .set_value(true, ctx));
            });
        });

        let workspace = mock_workspace(&mut app);

        let closed_snapshot = workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = false;
            workspace.snapshot(ctx.window_id(), false, ctx)
        });

        let restored = restored_workspace(&mut app, closed_snapshot);
        restored.read(&app, |workspace, _| {
            assert!(workspace.vertical_tabs_panel_open);
        });
    });
}

#[test]
fn test_vertical_tabs_panel_defaults_open_for_new_window_when_vertical_tabs_enabled() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
        });

        let workspace = mock_workspace(&mut app);

        workspace.read(&app, |workspace, _| {
            assert!(workspace.vertical_tabs_panel_open);
        });
    });
}

#[test]
fn test_vertical_tabs_panel_inherits_transferred_tab_source_window_state() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
        });

        let transferred_closed = transferred_tab_workspace(&mut app, false);
        let transferred_open = transferred_tab_workspace(&mut app, true);

        transferred_closed.read(&app, |workspace, _| {
            assert!(!workspace.vertical_tabs_panel_open);
        });
        transferred_open.read(&app, |workspace, _| {
            assert!(workspace.vertical_tabs_panel_open);
        });
    });
}

#[test]
fn test_vertical_tabs_panel_auto_shows_when_setting_enabled() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.read(&app, |workspace, _| {
            assert!(!workspace.vertical_tabs_panel_open);
        });

        // Enabling vertical tabs should auto-open the panel.
        workspace.update(&mut app, |_, ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
        });
        workspace.read(&app, |workspace, _| {
            assert!(workspace.vertical_tabs_panel_open);
        });

        // Disabling vertical tabs should auto-close the panel.
        workspace.update(&mut app, |_, ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(false, ctx));
            });
        });
        workspace.read(&app, |workspace, _| {
            assert!(!workspace.vertical_tabs_panel_open);
        });
    });
}

#[test]
fn test_toggle_tab_configs_menu_opens_vertical_tabs_panel_and_menu() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
            workspace.vertical_tabs_panel_open = true;
        });
        workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = false;
            workspace.show_new_session_dropdown_menu = None;

            workspace.handle_action(&WorkspaceAction::ToggleTabConfigsMenu, ctx);

            assert!(workspace.vertical_tabs_panel_open);
            assert!(workspace.show_new_session_dropdown_menu.is_some());
        });
    });
}

#[test]
fn test_toggle_tab_configs_menu_keyboard_shortcut_selects_top_item() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.show_new_session_dropdown_menu = None;

            workspace.handle_action(&WorkspaceAction::ToggleTabConfigsMenu, ctx);

            assert!(workspace.show_new_session_dropdown_menu.is_some());
            assert_eq!(
                workspace
                    .new_session_dropdown_menu
                    .read(ctx, |menu, _| menu.selected_index()),
                Some(0)
            );
        });
    });
}

#[test]
fn test_pointer_opened_tab_configs_menu_does_not_select_top_item() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.toggle_new_session_dropdown_menu(Vector2F::zero(), false, ctx);

            assert!(workspace.show_new_session_dropdown_menu.is_some());
            assert_eq!(
                workspace
                    .new_session_dropdown_menu
                    .read(ctx, |menu, _| menu.selected_index()),
                None
            );
        });
    });
}

#[test]
fn test_open_tab_config_with_params_does_not_use_worktree_branch_as_implicit_title() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let tab_config = crate::tab_configs::TabConfig {
            name: "Untitled worktree".to_string(),
            title: None,
            color: None,
            panes: vec![TabConfigPaneNode {
                id: "main".to_string(),
                pane_type: Some(TabConfigPaneType::Terminal),
                split: None,
                children: None,
                is_focused: Some(true),
                directory: None,
                commands: Some(vec!["echo {{autogenerated_branch_name}}".to_string()]),
                shell: None,
            }],
            params: HashMap::new(),
            source_path: None,
        };

        workspace.update(&mut app, |workspace, ctx| {
            workspace.open_tab_config_with_params(
                tab_config.clone(),
                HashMap::new(),
                Some("mesa-coyote"),
                ctx,
            );
        });

        workspace.read(&app, |workspace, ctx| {
            assert_eq!(workspace.tab_count(), 2);
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .custom_title(ctx),
                None
            );
        });
    });
}

#[test]
fn test_open_tab_config_with_params_uses_explicit_title_template() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let tab_config = crate::tab_configs::TabConfig {
            name: "Titled worktree".to_string(),
            title: Some("{{autogenerated_branch_name}}".to_string()),
            color: None,
            panes: vec![TabConfigPaneNode {
                id: "main".to_string(),
                pane_type: Some(TabConfigPaneType::Terminal),
                split: None,
                children: None,
                is_focused: Some(true),
                directory: None,
                commands: Some(vec!["echo {{autogenerated_branch_name}}".to_string()]),
                shell: None,
            }],
            params: HashMap::new(),
            source_path: None,
        };

        workspace.update(&mut app, |workspace, ctx| {
            workspace.open_tab_config_with_params(
                tab_config.clone(),
                HashMap::new(),
                Some("mesa-coyote"),
                ctx,
            );
        });

        workspace.read(&app, |workspace, ctx| {
            assert_eq!(workspace.tab_count(), 2);
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .custom_title(ctx),
                Some("mesa-coyote".to_string())
            );
        });
    });
}
#[test]
fn test_toggle_tab_configs_menu_does_not_change_vertical_tabs_panel_in_horizontal_mode() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.use_vertical_tabs.set_value(false, ctx));
            });
            workspace.vertical_tabs_panel_open = true;
            workspace.show_new_session_dropdown_menu = None;

            workspace.handle_action(&WorkspaceAction::ToggleTabConfigsMenu, ctx);

            assert!(workspace.vertical_tabs_panel_open);
            assert!(workspace.show_new_session_dropdown_menu.is_some());
        });
    });
}

#[test]
fn test_unified_new_session_menu_uses_new_worktree_config_label_and_order() {
    let _tab_configs_guard = FeatureFlag::TabConfigs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let labels = workspace
                .unified_new_session_menu_items(ctx)
                .iter()
                .map(new_session_menu_label)
                .collect::<Vec<_>>();

            assert!(!labels.iter().any(|label| label == "Worktree in"));

            // Anchor on the worktree-config entry itself rather than "the first
            // separator": the menu now has earlier separators (the agent section
            // and the Favorites section), so a first-separator heuristic is wrong.
            // The contract under test is that "New worktree config" opens its own
            // section (preceded by a separator) and is immediately followed by
            // "New tab config".
            let worktree_index = labels
                .iter()
                .position(|label| label == "New worktree config")
                .expect("expected 'New worktree config' in the new-session menu");
            assert_eq!(
                labels.get(worktree_index - 1),
                Some(&"---".to_string()),
                "worktree config should open its own separated section"
            );
            assert_eq!(
                labels.get(worktree_index + 1),
                Some(&"New tab config".to_string())
            );
        });
    });
}

#[test]
fn test_unified_new_session_menu_includes_reopen_closed_session() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            let menu_items = workspace.unified_new_session_menu_items(ctx);
            assert!(matches!(
                menu_items.get(menu_items.len() - 2),
                Some(MenuItem::Separator)
            ));

            let reopen_item = reopen_closed_session_menu_item(&menu_items);
            assert!(reopen_item.is_disabled());
            assert!(matches!(
                reopen_item.on_select_action(),
                Some(action) if matches!(action, WorkspaceAction::ReopenClosedSession)
            ));

            workspace.add_terminal_tab(false, ctx);
            workspace.remove_tab(workspace.active_tab_index(), true, true, ctx);

            let menu_items = workspace.unified_new_session_menu_items(ctx);
            let reopen_item = reopen_closed_session_menu_item(&menu_items);
            assert!(!reopen_item.is_disabled());
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
#[ignore = "depends on the decommissioned PersistedWorkspace"]
fn test_worktree_sidecar_search_editor_proxies_navigation_and_escape() {
    unimplemented!(
        "PersistedWorkspace has been decommissioned, worktree sidecar repository list testing is paused"
    );
}

#[cfg(feature = "local_fs")]
#[test]
#[ignore = "depends on the decommissioned PersistedWorkspace"]
fn test_worktree_sidecar_hides_linked_worktrees_from_repo_list() {
    unimplemented!(
        "PersistedWorkspace has been decommissioned, worktree sidecar repository list testing is paused"
    );
}

#[test]
fn test_vertical_tabs_context_menu_does_not_show_hover_only_tab_bar() {
    let _full_screen_zen_mode_guard = FeatureFlag::FullScreenZenMode.override_enabled(true);
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings
                    .workspace_decoration_visibility
                    .set_value(WorkspaceDecorationVisibility::OnHover, ctx));
                report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
            });
            workspace.should_show_ai_assistant_warm_welcome = false;
            workspace.vertical_tabs_panel_open = true;

            workspace.show_tab_right_click_menu =
                Some((0, TabContextMenuAnchor::Pointer(Vector2F::zero())));

            assert_eq!(workspace.tab_bar_mode(ctx), ShowTabBar::Hidden);
        });
    });
}

#[test]
fn test_standard_tab_context_menu_shows_hover_only_tab_bar() {
    let _full_screen_zen_mode_guard = FeatureFlag::FullScreenZenMode.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);

        workspace.update(&mut app, |workspace, ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings
                    .workspace_decoration_visibility
                    .set_value(WorkspaceDecorationVisibility::OnHover, ctx));
            });
            workspace.should_show_ai_assistant_warm_welcome = false;

            workspace.show_tab_right_click_menu =
                Some((0, TabContextMenuAnchor::Pointer(Vector2F::zero())));

            assert_eq!(workspace.tab_bar_mode(ctx), ShowTabBar::Stacked);
        });
    });
}

// Deleted: test_open_ambient_agent_setup_guide_action_opens_management_view_and_is_idempotent
// agent_management_view field along with entire agent setup guide functionality deleted in Phase 2c.

/// Spawn-card host id-space reconciliation (Codex review, wrong-host-launch
/// regression): a Conductor `+` scopes by the *daemon* `HostId`, but the spawn
/// card resolves against SSH `node.id`s. `node_for_daemon_host_in` must invert
/// the live daemon↔node association so the scoped launch preselects the right
/// SSH node — even when two hosts share a display label — and must yield `None`
/// for a daemon that no longer maps to a live node.
#[cfg(all(unix, feature = "local_tty"))]
#[test]
fn node_for_daemon_host_in_translates_daemon_id_to_ssh_node() {
    // Two SSH nodes share the display label "devbox" but host distinct daemons.
    // Translating by the *daemon* id must land on the matching node, not the
    // first same-labelled one.
    let assocs = || {
        vec![
            ("node-1", "daemon-a".to_string()),
            ("node-2", "daemon-b".to_string()),
        ]
    };
    let registered = vec!["node-1".to_string(), "node-2".to_string()];

    assert_eq!(
        super::node_for_daemon_host_in("daemon-b", assocs(), &registered),
        Some("node-2".to_string()),
        "daemon-b must translate to its own SSH node, not the first same-named one",
    );
    assert_eq!(
        super::node_for_daemon_host_in("daemon-a", assocs(), &registered),
        Some("node-1".to_string()),
    );

    // An untranslatable daemon id (session since dropped) yields None.
    assert_eq!(
        super::node_for_daemon_host_in("daemon-gone", assocs(), &registered),
        None
    );

    // A stale manager association is no longer routable once its registered
    // host has been removed.
    assert_eq!(
        super::node_for_daemon_host_in("daemon-b", assocs(), &["node-1".to_string()],),
        None
    );
}

#[test]
fn unresolved_daemon_scope_never_falls_back_to_display_name() {
    let (node_id, host_name) = super::reconcile_spawn_host_scope(
        None,
        Some("daemon-gone"),
        None,
        Some("devbox".to_string()),
    );

    assert_eq!(node_id, None);
    assert_eq!(
        host_name, None,
        "a stale daemon association must not preselect a different same-named host"
    );
}

#[cfg(unix)]
#[test]
fn adopted_pty_deduplication_is_host_scoped() {
    let first = super::daemon_adoption_key("node-a", "pty-1", 7);
    let second = super::daemon_adoption_key("node-b", "pty-1", 7);

    assert_ne!(
        first, second,
        "matching daemon-local PTY ids and generations on different hosts must open different tabs"
    );
}

#[cfg(unix)]
#[test]
fn pty_dedupe_uses_current_authoritative_agent_after_handoff() {
    let identity = |session_id: &str| remote_server::proto::AgentSessionIdentity {
        provider: "codex".to_string(),
        account_email: "agent@example.com".to_string(),
        config_dir: "/home/agent/.codex".to_string(),
        account_id: String::new(),
        session_id: session_id.to_string(),
    };
    let agent_a = identity("agent-a");
    let agent_b = identity("agent-b");
    let inventory = remote_server::proto::AgentSessionList {
        sessions: vec![remote_server::proto::AgentSessionInfo {
            session_id: agent_b.session_id.clone(),
            provider: agent_b.provider.clone(),
            config_dir: agent_b.config_dir.clone(),
            account_email: agent_b.account_email.clone(),
            pty_session_id: "pty-1".to_string(),
            pty_session_generation: 7,
            pty_foreground: true,
            ..Default::default()
        }],
    };

    assert!(
        !super::agent_inventory_confirms_binding(&inventory, "pty-1", 7, &agent_a),
        "a stale inventory row must not focus a PTY after its foreground handoff"
    );
    assert!(
        super::agent_inventory_confirms_binding(&inventory, "pty-1", 7, &agent_b),
        "the current foreground agent may reuse the one PTY tab"
    );
}

#[test]
fn managed_lifecycle_success_requires_the_exact_response_envelope() {
    use remote_server::proto::{
        ManagedSessionLifecycleAction, ManagedSessionLifecycleRequest,
        ManagedSessionLifecycleResponse, ManagedSessionLifecycleStatus,
    };

    let request = ManagedSessionLifecycleRequest {
        schema_version: 1,
        action: ManagedSessionLifecycleAction::Restart.into(),
        session_id: "managed-pty".to_string(),
        expected_generation: 7,
        launch_id: "launch-7".to_string(),
        provider: "claude".to_string(),
        account_id: "account-7".to_string(),
        project_root: "/srv/project".to_string(),
    };
    let response = ManagedSessionLifecycleResponse {
        schema_version: 1,
        action: ManagedSessionLifecycleAction::Restart.into(),
        status: ManagedSessionLifecycleStatus::Restarted.into(),
        session_id: "managed-pty".to_string(),
        generation: 7,
        replacement_session_id: "managed-pty-next".to_string(),
        replacement_generation: 8,
        diagnostic_code: String::new(),
    };
    assert_eq!(
        super::managed_lifecycle_result(&request, &response),
        Ok("Managed agent restarted.")
    );

    let mut mismatched = response;
    mismatched.generation = 6;
    assert!(super::managed_lifecycle_result(&request, &mismatched).is_err());
}

#[test]
fn managed_lifecycle_rejects_malformed_success_and_accepts_typed_failure() {
    use remote_server::proto::{
        ManagedSessionLifecycleAction, ManagedSessionLifecycleRequest,
        ManagedSessionLifecycleResponse, ManagedSessionLifecycleStatus,
    };

    let request = ManagedSessionLifecycleRequest {
        schema_version: 1,
        action: ManagedSessionLifecycleAction::Stop.into(),
        session_id: "managed-stop".to_string(),
        expected_generation: 4,
        launch_id: "launch-stop".to_string(),
        provider: "codex".to_string(),
        account_id: "account-stop".to_string(),
        project_root: "/srv/stop".to_string(),
    };
    let malformed = ManagedSessionLifecycleResponse {
        schema_version: 1,
        action: ManagedSessionLifecycleAction::Stop.into(),
        status: ManagedSessionLifecycleStatus::Stopped.into(),
        session_id: "managed-stop".to_string(),
        generation: 4,
        replacement_session_id: "unexpected".to_string(),
        replacement_generation: 9,
        diagnostic_code: String::new(),
    };
    assert!(super::managed_lifecycle_result(&request, &malformed).is_err());

    let rejected = ManagedSessionLifecycleResponse {
        status: ManagedSessionLifecycleStatus::Blocked.into(),
        replacement_session_id: String::new(),
        replacement_generation: 0,
        diagnostic_code: "headroom-below-floor".to_string(),
        ..malformed
    };
    let error = super::managed_lifecycle_result(&request, &rejected).unwrap_err();
    assert!(error.contains("headroom-below-floor"));
}

#[cfg(unix)]
#[test]
fn rejected_adopt_cleanup_does_not_poison_retry_key() {
    let rejected_connection = warp_core::SessionId::from(41u64);
    let live_connection = warp_core::SessionId::from(42u64);
    let rejected_key = super::daemon_adoption_key("node-a", "pty-1", 7);
    let live_key = super::daemon_adoption_key("node-a", "pty-2", 8);
    let mut adopted = std::collections::HashMap::from([
        (
            rejected_key.clone(),
            super::AdoptedDaemonSession {
                pane_group_id: warpui::EntityId::new(),
                connection_session_id: rejected_connection,
            },
        ),
        (
            live_key.clone(),
            super::AdoptedDaemonSession {
                pane_group_id: warpui::EntityId::new(),
                connection_session_id: live_connection,
            },
        ),
    ]);

    super::remove_adopted_daemon_session(&mut adopted, rejected_connection);

    assert!(
        !adopted.contains_key(&rejected_key),
        "a rejected provisional tab must not intercept the next atomic attach attempt"
    );
    assert!(
        adopted.contains_key(&live_key),
        "cleaning one rejected connection must preserve unrelated live adopts"
    );
}

#[cfg(unix)]
#[test]
fn daemon_node_route_falls_back_to_surviving_connection() {
    let first = warp_core::SessionId::from(51u64);
    let rejected = warp_core::SessionId::from(52u64);
    let mut routes = std::collections::HashMap::new();

    super::remember_daemon_node_session(&mut routes, "node-a".to_string(), first);
    super::remember_daemon_node_session(&mut routes, "node-a".to_string(), rejected);
    super::forget_daemon_node_session(&mut routes, rejected);

    assert_eq!(
        routes.get("node-a"),
        Some(&vec![first]),
        "rejecting a provisional adopt must preserve an older live route for the node"
    );
}

#[cfg(unix)]
#[test]
fn cockpit_daemon_adopt_uses_resolved_onekey_auth() {
    let mut server = warp_ssh_manager::SshServerInfo::new_default("node-a".to_string());
    server.username = "placeholder".to_string();
    server.auth_type = warp_ssh_manager::AuthType::OneKey;
    let resolved = warp_ssh_manager::ResolvedSshAuth {
        username: "resolved-user".to_string(),
        auth_type: warp_ssh_manager::AuthType::Key,
        key_path: Some("/keys/resolved".to_string()),
        secret_lookup_id: "credential-a".to_string(),
        secret_kind: warp_ssh_manager::SecretKind::Passphrase,
    };

    let server = super::apply_daemon_server_auth(server, resolved);

    assert_eq!(server.username, "resolved-user");
    assert_eq!(server.auth_type, warp_ssh_manager::AuthType::Key);
    assert_eq!(server.key_path.as_deref(), Some("/keys/resolved"));
}

#[test]
fn invalid_pid_never_reaches_local_signal_backend() {
    let mut signal_calls = 0;
    let _ = send_verified_guardrail_signal_with(
        u32::MAX,
        Some("birth-1"),
        Some("birth-1"),
        zaplex_cockpit::GuardrailSignal::Interrupt,
        |_, _| {
            signal_calls += 1;
            GuardrailSendOutcome::Sent
        },
    );

    assert_eq!(signal_calls, 0);
}

#[test]
fn recycled_pid_never_reaches_local_signal_backend() {
    let mut signal_calls = 0;
    let _ = send_verified_guardrail_signal_with(
        4242,
        Some("original-process"),
        Some("recycled-process"),
        zaplex_cockpit::GuardrailSignal::Kill,
        |_, _| {
            signal_calls += 1;
            GuardrailSendOutcome::Sent
        },
    );

    assert_eq!(signal_calls, 0);
}

#[test]
fn unprovable_process_identity_never_reaches_local_signal_backend() {
    for (expected, observed) in [(None, Some("live")), (Some("expected"), None), (None, None)] {
        let mut signal_calls = 0;
        let _ = send_verified_guardrail_signal_with(
            4242,
            expected,
            observed,
            zaplex_cockpit::GuardrailSignal::Interrupt,
            |_, _| {
                signal_calls += 1;
                GuardrailSendOutcome::Sent
            },
        );

        assert_eq!(
            signal_calls, 0,
            "expected={expected:?}, observed={observed:?} must fail closed"
        );
    }
}

#[test]
fn recycled_remote_pid_is_rejected_before_remote_request() {
    let mut remote_request_calls = 0;
    let _ = send_verified_guardrail_signal_with(
        4242,
        Some("inventory-process"),
        Some("current-process"),
        zaplex_cockpit::GuardrailSignal::Interrupt,
        |_, _| {
            remote_request_calls += 1;
            GuardrailSendOutcome::Sent
        },
    );

    assert_eq!(
        remote_request_calls, 0,
        "a stale remote row must be rejected before sending a remote signal request"
    );
}

#[test]
fn matching_process_identity_reaches_signal_backend_once() {
    let mut signal_calls = 0;
    let _ = send_verified_guardrail_signal_with(
        4242,
        Some("same-process"),
        Some("same-process"),
        zaplex_cockpit::GuardrailSignal::Interrupt,
        |pid, signal| {
            signal_calls += 1;
            assert_eq!(pid, 4242);
            assert_eq!(signal, zaplex_cockpit::GuardrailSignal::Interrupt);
            GuardrailSendOutcome::Sent
        },
    );

    assert_eq!(signal_calls, 1);
}
