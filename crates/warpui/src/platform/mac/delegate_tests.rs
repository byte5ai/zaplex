use super::{
    complete_notification_send, complete_request_notification_permissions,
    NotificationSendErrorCallback, RequestNotificationPermissionsCallback,
};
use anyhow::anyhow;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use warpui_core::notification::{NotificationSendError, RequestPermissionsOutcome};

struct DropCounter(Arc<AtomicUsize>);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn successful_notification_send_frees_callback_without_reporting_error() {
    let calls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let callback: NotificationSendErrorCallback = {
        let calls = calls.clone();
        let drop_counter = DropCounter(drops.clone());
        Box::new(move |_| {
            assert_eq!(drop_counter.0.load(Ordering::SeqCst), 0);
            calls.fetch_add(1, Ordering::SeqCst);
        })
    };

    complete_notification_send(callback, None);

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn native_notification_error_invokes_callback_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let callback: NotificationSendErrorCallback = {
        let calls = calls.clone();
        Box::new(move |error| {
            assert!(matches!(error, NotificationSendError::PermissionsDenied));
            calls.fetch_add(1, Ordering::SeqCst);
        })
    };

    complete_notification_send(callback, Some(Ok(NotificationSendError::PermissionsDenied)));

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn notification_error_decode_failure_is_reported_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let callback: NotificationSendErrorCallback = {
        let calls = calls.clone();
        Box::new(move |error| {
            let NotificationSendError::Other { error_message } = error else {
                panic!("expected a decoded native error");
            };
            assert!(error_message.contains("invalid native string"));
            calls.fetch_add(1, Ordering::SeqCst);
        })
    };

    complete_notification_send(callback, Some(Err(anyhow!("invalid native string"))));

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn permission_outcome_decode_failure_is_reported_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let callback: RequestNotificationPermissionsCallback = {
        let calls = calls.clone();
        Box::new(move |outcome| {
            let RequestPermissionsOutcome::OtherError { error_message } = outcome else {
                panic!("expected a decoded native outcome error");
            };
            assert!(error_message.contains("invalid native string"));
            calls.fetch_add(1, Ordering::SeqCst);
        })
    };

    complete_request_notification_permissions(callback, Err(anyhow!("invalid native string")));

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
