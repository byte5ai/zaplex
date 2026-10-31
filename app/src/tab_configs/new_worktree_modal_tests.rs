use std::{cell::RefCell, rc::Rc};

use pathfinder_geometry::vector::vec2f;
use warp_util::user_input::UserInput;
use warpui::{
    event::{Event, KeyEventDetails},
    keymap::Keystroke,
    platform::WindowStyle,
    App, Presenter, TypedActionView, WindowId, WindowInvalidation,
};

use super::{NewWorktreeModal, NewWorktreeModalEvent};
use crate::{
    editor::{EditorAction, EditorView},
    tab_configs::repo_picker::RepoPickerAction,
    test_util::terminal::initialize_app_for_terminal_view,
};

fn initialize_keyboard(app: &mut App) {
    initialize_app_for_terminal_view(app);
    app.update(|ctx| {
        crate::editor::init(ctx);
        crate::menu::init(ctx);
        super::init(ctx);
    });
}

fn send_escape(app: &mut App, window: WindowId, presenter: &Rc<RefCell<Presenter>>) {
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
            .build_scene(vec2f(1024., 678.), 1., None, ctx);
        ctx.simulate_window_event(
            Event::KeyDown {
                keystroke: Keystroke::parse("escape").unwrap(),
                chars: String::new(),
                details: KeyEventDetails::default(),
                is_composing: false,
            },
            window,
            presenter.clone(),
        );
    });
}

#[test]
fn worktree_modal_repository_picker_escape_after_folder_result_closes_modal() {
    crate::i18n::init(Some("en"));
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("repository-a");
    let second = directory.path().join("repository-b");
    git2::Repository::init(&first).unwrap();
    git2::Repository::init(&second).unwrap();
    for selected_folder in [true, false] {
        let first = first.clone();
        let second = second.clone();
        App::test((), |mut app| async move {
            initialize_keyboard(&mut app);
            let (window, modal) = app.add_window(WindowStyle::NotStealFocus, NewWorktreeModal::new);
            app.read(|ctx| ctx.windows().show_window_and_focus_app(window));
            let closed = Rc::new(RefCell::new(0));
            let requested = Rc::new(RefCell::new(0));
            app.update(|ctx| {
                let closed = closed.clone();
                let requested = requested.clone();
                ctx.subscribe_to_view(&modal, move |_, event, _| match event {
                    NewWorktreeModalEvent::Close => *closed.borrow_mut() += 1,
                    NewWorktreeModalEvent::PickNewRepo => *requested.borrow_mut() += 1,
                    NewWorktreeModalEvent::Submit { .. } => panic!("Escape must never submit"),
                });
            });
            modal.update(&mut app, |modal, ctx| {
                modal.on_open(Some(first.clone()), ctx)
            });
            let picker = modal.read(&app, |modal, _| modal.repo_picker.clone());
            picker.update(&mut app, |picker, ctx| {
                assert!(picker.toggle_dropdown(ctx));
            });
            let focused = app.focused_view_id(window).unwrap();
            let search = app.read(|ctx| ctx.view_with_id::<EditorView>(window, focused).unwrap());
            search.update(&mut app, |editor, ctx| {
                editor.handle_action(
                    &EditorAction::UserInsert(UserInput::new("no-such-repository")),
                    ctx,
                );
            });
            search.read(&app, |editor, ctx| {
                assert_eq!(editor.buffer_text(ctx), "no-such-repository");
            });
            picker.update(&mut app, |picker, ctx| {
                picker.handle_action(&RepoPickerAction::AddNewRepo, ctx);
            });
            assert_eq!(*requested.borrow(), 1);
            if selected_folder {
                modal.update(&mut app, |modal, ctx| {
                    modal.on_new_repo_selected(second.clone(), ctx);
                });
            }
            picker.read(&app, |picker, ctx| {
                let expected = if selected_folder { &second } else { &first };
                assert_eq!(
                    picker.selected_value(ctx).as_deref(),
                    Some(expected.to_string_lossy().as_ref())
                );
            });
            assert_eq!(*closed.borrow(), 0);
            let presenter = Rc::new(RefCell::new(Presenter::new(window)));
            send_escape(&mut app, window, &presenter);
            assert_eq!(
                *closed.borrow(),
                1,
                "A real Escape must close the modal after folder result selected={selected_folder}"
            );
        });
    }
}

#[test]
fn worktree_modal_repository_picker_escape_closes_filter_before_modal() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        initialize_keyboard(&mut app);
        let (window, modal) = app.add_window(WindowStyle::NotStealFocus, NewWorktreeModal::new);
        app.read(|ctx| ctx.windows().show_window_and_focus_app(window));
        let closed = Rc::new(RefCell::new(0));
        app.update(|ctx| {
            let closed = closed.clone();
            ctx.subscribe_to_view(&modal, move |_, event, _| {
                if matches!(event, NewWorktreeModalEvent::Close) {
                    *closed.borrow_mut() += 1;
                }
            });
        });
        modal.update(&mut app, |modal, ctx| modal.on_open(None, ctx));
        let picker = modal.read(&app, |modal, _| modal.repo_picker.clone());
        picker.update(&mut app, |picker, ctx| {
            assert!(picker.toggle_dropdown(ctx));
        });
        let focused = app.focused_view_id(window).unwrap();
        assert!(app.read(|ctx| ctx.view_with_id::<EditorView>(window, focused).is_some()));
        let presenter = Rc::new(RefCell::new(Presenter::new(window)));
        send_escape(&mut app, window, &presenter);
        assert_eq!(
            *closed.borrow(),
            0,
            "First Escape only dismisses the filter"
        );
        assert_ne!(
            app.focused_view_id(window),
            Some(focused),
            "The hidden search editor must not retain keyboard focus"
        );
        send_escape(&mut app, window, &presenter);
        assert_eq!(*closed.borrow(), 1, "Second Escape dismisses the modal");
    });
}

#[test]
fn worktree_modal_repository_picker_close_preserves_other_editor_focus() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        initialize_keyboard(&mut app);
        let (window, modal) = app.add_window(WindowStyle::NotStealFocus, NewWorktreeModal::new);
        modal.update(&mut app, |modal, ctx| {
            modal.on_open(None, ctx);
            modal.handle_action(&super::NewWorktreeModalAction::ToggleAutogenerate, ctx);
        });
        let (picker, name) = modal.read(&app, |modal, _| {
            (
                modal.repo_picker.clone(),
                modal.worktree_name_editor.clone(),
            )
        });
        picker.update(&mut app, |picker, ctx| {
            assert!(picker.toggle_dropdown(ctx));
        });
        name.update(&mut app, |_, ctx| ctx.focus_self());
        assert_eq!(app.focused_view_id(window), Some(name.id()));
        // A late close notification must not reclaim a sibling editor's focus.
        picker.update(&mut app, |picker, ctx| {
            picker.handle_action(&RepoPickerAction::AddNewRepo, ctx);
        });
        assert_eq!(app.focused_view_id(window), Some(name.id()));
    });
}

#[test]
fn worktree_modal_submits_the_displayed_current_or_newly_selected_repository() {
    crate::i18n::init(Some("en"));
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("repository-a");
    let second = directory.path().join("repository-b");
    git2::Repository::init(&first).unwrap();
    git2::Repository::init(&second).unwrap();
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (_, modal) = app.add_window(WindowStyle::NotStealFocus, NewWorktreeModal::new);
        let submitted = Rc::new(RefCell::new(Vec::new()));
        let picked = Rc::new(RefCell::new(0));
        app.update(|app| {
            let submitted = submitted.clone();
            let picked = picked.clone();
            app.subscribe_to_view(&modal, move |_, event, _| match event {
                NewWorktreeModalEvent::Submit { repo, branch, .. } => {
                    submitted.borrow_mut().push((repo.clone(), branch.clone()));
                }
                NewWorktreeModalEvent::PickNewRepo => *picked.borrow_mut() += 1,
                NewWorktreeModalEvent::Close => {}
            });
        });

        modal.update(&mut app, |modal, ctx| {
            modal.on_open(Some(first.clone()), ctx)
        });
        for path in [&first, &second] {
            if path == &second {
                modal.update(&mut app, |modal, ctx| {
                    modal.on_new_repo_selected(path.clone(), ctx);
                });
            }
            // Deliver the branch picker's completion event independently of Git timing.
            modal.update(&mut app, |modal, ctx| {
                modal
                    .branch_picker
                    .update(ctx, |_, ctx| ctx.emit("main".into()));
            });
            modal.update(&mut app, |modal, ctx| {
                assert_eq!(
                    modal.repo_picker.as_ref(ctx).selected_value(ctx).as_deref(),
                    Some(path.to_string_lossy().as_ref())
                );
                modal.try_submit(ctx);
            });
        }

        // Requesting another folder does not replace B unless a selection is returned.
        modal.update(&mut app, |modal, ctx| {
            modal.repo_picker.update(ctx, |picker, ctx| {
                picker.handle_action(&RepoPickerAction::AddNewRepo, ctx);
            });
        });
        assert_eq!(*picked.borrow(), 1);
        modal.update(&mut app, |modal, ctx| modal.try_submit(ctx));
        assert_eq!(
            *submitted.borrow(),
            vec![
                (first.to_string_lossy().into_owned(), "main".into()),
                (second.to_string_lossy().into_owned(), "main".into()),
                (second.to_string_lossy().into_owned(), "main".into()),
            ]
        );
    });
}

#[test]
fn worktree_modal_without_repository_rejects_submission_even_with_a_branch() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (_, modal) = app.add_window(WindowStyle::NotStealFocus, NewWorktreeModal::new);
        let submitted = Rc::new(RefCell::new(0));
        app.update(|app| {
            let submitted = submitted.clone();
            app.subscribe_to_view(&modal, move |_, event, _| {
                if let NewWorktreeModalEvent::Submit { .. } = event {
                    *submitted.borrow_mut() += 1;
                }
            });
        });
        modal.update(&mut app, |modal, ctx| {
            modal
                .branch_picker
                .update(ctx, |_, ctx| ctx.emit("main".into()));
        });
        modal.update(&mut app, |modal, ctx| modal.try_submit(ctx));
        assert_eq!(*submitted.borrow(), 0);
    });
}
