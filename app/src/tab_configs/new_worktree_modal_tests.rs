use std::{cell::RefCell, rc::Rc};

use warpui::{platform::WindowStyle, App, TypedActionView};

use super::{NewWorktreeModal, NewWorktreeModalEvent};
use crate::{
    tab_configs::repo_picker::RepoPickerAction,
    test_util::terminal::initialize_app_for_terminal_view,
};

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
