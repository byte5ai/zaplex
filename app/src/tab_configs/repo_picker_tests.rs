use warpui::{platform::WindowStyle, App, AppContext, TypedActionView, View};

use super::{RepoPicker, RepoPickerAction};
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn assert_selection(picker: &RepoPicker, app: &AppContext, expected: Option<&str>) {
    let dropdown = picker.dropdown.as_ref(app);
    assert_eq!(
        dropdown
            .render(app)
            .debug_text_content()
            .unwrap_or_default(),
        expected.unwrap_or_default()
    );
    assert_eq!(dropdown.selected_item_label().as_deref(), expected);
    assert_eq!(picker.selected_value(app).as_deref(), expected);
}

#[test]
fn repository_picker_displays_current_repo_without_saved_projects() {
    crate::i18n::init(Some("en"));
    let directory = tempfile::tempdir().unwrap();
    git2::Repository::init(directory.path()).unwrap();
    let path = directory.path().to_string_lossy().into_owned();
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (_, picker) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RepoPicker::new(Some(path.clone()), ctx)
        });
        picker.read(&app, |picker, app| {
            assert_selection(picker, app, Some(&path))
        });
    });
}

#[test]
fn repository_picker_replaces_displayed_and_effective_selection() {
    crate::i18n::init(Some("en"));
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("repository-a");
    let second = directory
        .path()
        .join("repository-b-with-a-long-and-unambiguous-identity");
    git2::Repository::init(&first).unwrap();
    git2::Repository::init(&second).unwrap();
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (_, picker) =
            app.add_window(WindowStyle::NotStealFocus, |ctx| RepoPicker::new(None, ctx));
        for path in [first, second] {
            picker.update(&mut app, |picker, ctx| {
                picker.refresh_and_select(path.clone(), ctx)
            });
            picker.read(&app, |picker, app| {
                assert_selection(picker, app, Some(&path.to_string_lossy()));
            });
        }
    });
}

#[test]
fn repository_picker_requesting_another_repo_preserves_selection_until_chosen() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (_, picker) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RepoPicker::new(Some("/projects/repository-a".into()), ctx)
        });
        picker.update(&mut app, |picker, ctx| {
            picker.toggle_dropdown(ctx);
            picker.handle_action(&RepoPickerAction::AddNewRepo, ctx);
        });
        picker.read(&app, |picker, app| {
            assert_selection(picker, app, Some("/projects/repository-a"));
        });
    });
}

#[test]
fn repository_picker_without_a_path_has_no_selection_or_displayed_identity() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (_, picker) =
            app.add_window(WindowStyle::NotStealFocus, |ctx| RepoPicker::new(None, ctx));
        picker.read(&app, |picker, app| assert_selection(picker, app, None));
    });
}

#[test]
fn repository_picker_empty_default_does_not_select_an_empty_path() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (_, picker) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RepoPicker::new(Some(String::new()), ctx)
        });
        picker.read(&app, |picker, app| assert_selection(picker, app, None));
    });
}
