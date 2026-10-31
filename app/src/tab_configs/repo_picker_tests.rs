use std::{cell::RefCell, rc::Rc};

use pathfinder_geometry::vector::vec2f;
use warp_util::user_input::UserInput;
use warpui::{
    event::Event as WindowEvent, platform::WindowStyle, App, AppContext, Presenter,
    SingletonEntity, TypedActionView, View, WindowInvalidation,
};

use super::{RepoPicker, RepoPickerAction, RepoPickerEvent, DEFAULT_DROPDOWN_WIDTH};
use crate::{
    appearance::Appearance,
    editor::{EditorAction, EditorView},
    menu::Menu,
    test_util::terminal::initialize_app_for_terminal_view,
    view_components::dropdown::{DropdownAction, DROPDOWN_PADDING},
};

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

#[test]
fn repository_picker_footer_uses_the_shared_repository_action_label() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (window, picker) =
            app.add_window(WindowStyle::NotStealFocus, |ctx| RepoPicker::new(None, ctx));
        picker.update(&mut app, |picker, ctx| {
            assert!(picker.toggle_dropdown(ctx));
        });
        let menus = app.read(|ctx| {
            ctx.view_ids_for_window(window)
                .into_iter()
                .filter_map(|id| {
                    ctx.view_with_id::<Menu<DropdownAction<RepoPickerAction>>>(window, id)
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(menus.len(), 1);
        menus[0].read(&app, |menu, ctx| {
            assert_eq!(
                menu.render(ctx).debug_text_content().as_deref(),
                Some(" + Add new repo")
            );
        });
    });
}

#[test]
fn repository_picker_displays_new_folder_after_an_unmatched_search() {
    crate::i18n::init(Some("en"));
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("repository-a");
    let second = directory.path().join("repository-b");
    git2::Repository::init(&first).unwrap();
    git2::Repository::init(&second).unwrap();
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (window, picker) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RepoPicker::new(Some(first.to_string_lossy().into_owned()), ctx)
        });
        picker.update(&mut app, |picker, ctx| {
            picker.toggle_dropdown(ctx);
        });
        let focused = app.focused_view_id(window).unwrap();
        let search = app.read(|app| app.view_with_id::<EditorView>(window, focused).unwrap());
        search.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &EditorAction::UserInsert(UserInput::new("no-such-repository")),
                ctx,
            );
        });
        search.read(&app, |editor, app| {
            assert_eq!(editor.buffer_text(app), "no-such-repository");
        });
        picker.update(&mut app, |picker, ctx| {
            picker.handle_action(&RepoPickerAction::AddNewRepo, ctx);
        });
        picker.update(&mut app, |picker, ctx| {
            picker.refresh_and_select(second.clone(), ctx);
        });
        picker.read(&app, |picker, app| {
            assert_selection(picker, app, Some(&second.to_string_lossy()));
        });
    });
}

#[test]
fn repository_picker_add_repo_row_accepts_mouse_clicks_across_its_width() {
    crate::i18n::init(Some("en"));
    for x in [
        16.,
        DEFAULT_DROPDOWN_WIDTH / 2.,
        DEFAULT_DROPDOWN_WIDTH - 16.,
    ] {
        App::test((), |mut app| async move {
            initialize_app_for_terminal_view(&mut app);
            let (window, picker) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
                RepoPicker::new(Some("/projects/repository-a".into()), ctx)
            });
            let requests = Rc::new(RefCell::new(0));
            let observed = requests.clone();
            app.update(|ctx| {
                ctx.subscribe_to_view(&picker, move |_, event, _| {
                    if matches!(event, RepoPickerEvent::RequestAddRepo) {
                        *observed.borrow_mut() += 1;
                    }
                });
            });
            picker.update(&mut app, |picker, ctx| {
                picker.toggle_dropdown(ctx);
            });
            let focused = app.focused_view_id(window).unwrap();
            let search = app.read(|app| app.view_with_id::<EditorView>(window, focused).unwrap());
            search.update(&mut app, |editor, ctx| {
                editor.handle_action(
                    &EditorAction::UserInsert(UserInput::new("no-such-repository")),
                    ctx,
                );
            });
            search.read(&app, |editor, app| {
                assert_eq!(editor.buffer_text(app), "no-such-repository");
            });
            // The empty results put the pinned footer directly below the search bar
            // and the menu's top padding. All three points lie in that visible row.
            let y = app.read(|app| {
                Appearance::as_ref(app).dropdown_top_bar_height() + DROPDOWN_PADDING + 20.
            });
            let position = vec2f(x, y);
            let presenter = Rc::new(RefCell::new(Presenter::new(window)));
            for event in [
                WindowEvent::MouseMoved {
                    position,
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                },
                WindowEvent::LeftMouseDown {
                    position,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                WindowEvent::LeftMouseUp {
                    position,
                    modifiers: Default::default(),
                },
            ] {
                app.update(|ctx| {
                    presenter.borrow_mut().invalidate(
                        WindowInvalidation {
                            updated: ctx.view_ids_for_window(window).into_iter().collect(),
                            ..Default::default()
                        },
                        ctx,
                    );
                    presenter
                        .borrow_mut()
                        .build_scene(vec2f(800., 600.), 1., None, ctx);
                    ctx.simulate_window_event(event, window, presenter.clone());
                });
            }
            assert_eq!(*requests.borrow(), 1, "footer click at x={x}");
            picker.read(&app, |picker, app| {
                assert!(!picker.dropdown.as_ref(app).is_expanded());
                assert_selection(picker, app, Some("/projects/repository-a"));
            });
        });
    }
}
