use std::{cell::Cell, rc::Rc};

use super::App;
use crate::modals::{AlertDialogWithCallbacks, ModalButton};
use crate::platform::app::{AppCallbackDispatcher, AppCallbacks, ApproveTerminateResult};

#[test]
fn startup_failure_disables_application_callbacks_but_preserves_modal_response() {
    App::test((), |mut app| async move {
        let modal_clicked = Rc::new(Cell::new(false));
        let modal_clicked_callback = modal_clicked.clone();
        let callbacks = AppCallbacks {
            on_resigned_active: Some(Box::new(|_| panic!("partially initialized callback"))),
            on_will_terminate: Some(Box::new(|_| panic!("partially initialized teardown"))),
            on_should_terminate_app: Some(Box::new(|_| panic!("partially initialized quit"))),
            on_os_appearance_changed: Some(Box::new(|_| panic!("partially initialized theme"))),
            ..Default::default()
        };
        let mut dispatcher = AppCallbackDispatcher::new(callbacks, app.clone());
        dispatcher.initialize_app(Box::new(move |ctx, _| {
            ctx.disable_application_callbacks_after_failed_startup();
            ctx.show_native_platform_modal(AlertDialogWithCallbacks::for_app(
                "Startup error",
                "State was preserved",
                vec![ModalButton::for_app("Close", move |_| {
                    modal_clicked_callback.set(true);
                })],
                |_| {},
            ));
        }));
        dispatcher.app_resigned_active();
        dispatcher.os_appearance_changed();
        assert!(matches!(
            dispatcher.should_terminate_app(),
            ApproveTerminateResult::Terminate
        ));
        dispatcher.app_will_terminate();
        let modal_id = app.update(|ctx| *ctx.platform_modal_data_map.keys().next().unwrap());
        dispatcher.process_platform_modal_response(modal_id, 0, false);
        assert!(modal_clicked.get());
    });
}

#[test]
fn successful_startup_keeps_application_callbacks() {
    App::test((), |app| async move {
        let callback_called = Rc::new(Cell::new(false));
        let called = callback_called.clone();
        let callbacks = AppCallbacks {
            on_os_appearance_changed: Some(Box::new(move |_| called.set(true))),
            on_should_terminate_app: Some(Box::new(|_| ApproveTerminateResult::Cancel)),
            ..Default::default()
        };
        let mut dispatcher = AppCallbackDispatcher::new(callbacks, app);
        dispatcher.initialize_app(Box::new(|_, _| {}));
        dispatcher.os_appearance_changed();
        assert!(callback_called.get());
        assert!(matches!(
            dispatcher.should_terminate_app(),
            ApproveTerminateResult::Cancel
        ));
    });
}
