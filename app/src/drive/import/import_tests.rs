use std::env::current_dir;

use warp_core::ui::appearance::Appearance;
use warpui::{platform::WindowStyle, App, TypedActionView, ViewHandle};

use crate::{
    cloud_object::model::persistence::ObjectStoreModel,
    cloud_object::update_manager::UpdateManager, network::NetworkStatus,
    workspaces::user_workspaces::UserWorkspaces, GlobalResourceHandles,
    GlobalResourceHandlesProvider,
};

use super::{expand_dirs, FilePickerError, ImportModalBody, ImportModalBodyAction, ImportState};

#[test]
fn test_expand_directories() {
    App::test((), |mut app| async move {
        app.update(crate::settings::init_and_register_user_preferences);

        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
        app.add_singleton_model(ObjectStoreModel::mock);
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(UpdateManager::mock);

        let directory = current_dir()
            .expect("current directory should exist")
            .parent()
            .expect("parent directory should exist")
            .to_path_buf()
            .join("crates")
            .join("integration");

        // Open a folder and verify we could expand it into the correct folder tree structure.
        assert_eq!(warpui::r#async::block_on(expand_dirs([directory].into_iter().collect())).debug_print(), "(integration(tests(INTEGRATION_TESTING, data(test, test_launch_config, test_theme, test_theme_with_name, test_workflow))))");
    });
}

fn picker_body(app: &mut App) -> ViewHandle<ImportModalBody> {
    app.update(crate::settings::init_and_register_user_preferences);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(UpdateManager::mock);
    let (_, body) = app.add_window(WindowStyle::NotStealFocus, ImportModalBody::new);
    body
}

#[test]
fn closed_import_rejects_late_picker_selection() {
    App::test((), |mut app| async move {
        let body = picker_body(&mut app);
        body.update(&mut app, |body, ctx| {
            body.handle_action(&ImportModalBodyAction::OpenFilePicker, ctx);
            let old_generation = body.current_generation;
            body.reset(ctx);
            body.handle_action(
                &ImportModalBodyAction::PathsSelected(
                    old_generation,
                    vec!["/must-not-be-imported.md".to_string()],
                ),
                ctx,
            );
            assert!(matches!(body.state, ImportState::Upload));
            assert!(body.in_progress_handle.is_none());
        });
    });
}

#[test]
fn reopened_import_rejects_all_old_picker_results_and_accepts_current_cancel() {
    App::test((), |mut app| async move {
        let body = picker_body(&mut app);
        body.update(&mut app, |body, ctx| {
            body.handle_action(&ImportModalBodyAction::OpenFilePicker, ctx);
            let old_generation = body.current_generation;
            body.reset(ctx);
            let target = Some(crate::server::ids::SyncId::ClientId(
                crate::server::ids::ClientId::new(),
            ));
            body.set_new_target(crate::cloud_object::Owner::mock_current_user(), target);
            body.handle_action(&ImportModalBodyAction::OpenFilePicker, ctx);
            let current_generation = body.current_generation;
            assert_ne!(old_generation, current_generation);
            for action in [
                ImportModalBodyAction::PathsSelected(
                    old_generation,
                    vec!["/must-not-be-imported.md".to_string()],
                ),
                ImportModalBodyAction::FilePickerCancelled(old_generation),
                ImportModalBodyAction::FilePickerError(
                    old_generation,
                    FilePickerError::DialogFailed("stale picker".to_string()),
                ),
            ] {
                body.handle_action(&action, ctx);
                assert!(matches!(body.state, ImportState::Loading));
                assert_eq!(body.initial_folder_id, target);
                assert_eq!(body.current_generation, current_generation);
                assert!(body.in_progress_handle.is_none());
            }
            body.handle_action(
                &ImportModalBodyAction::FilePickerCancelled(current_generation),
                ctx,
            );
            assert!(matches!(body.state, ImportState::Upload));
            // A callback can be consumed only while its picker is pending.
            body.handle_action(
                &ImportModalBodyAction::PathsSelected(current_generation, Vec::new()),
                ctx,
            );
            assert!(matches!(body.state, ImportState::Upload));
            assert!(body.in_progress_handle.is_none());
        });
    });
}
