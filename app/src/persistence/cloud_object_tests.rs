use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use diesel::{Connection, SqliteConnection};

use crate::{
    auth::UserUid,
    cloud_object::{
        ObjectType, ServerObjectContainer, StoredObjectGuest, StoredObjectMetadata,
        StoredObjectPermissions,
    },
    drive::sharing::{LinkSharingSubjectType, SharingAccessLevel, Subject, TeamKind, UserKind},
    server::ids::{ClientId, ServerId, SyncId},
};

#[test]
fn metadata_probe_errors_do_not_fall_through_to_insert() {
    let mut conn = SqliteConnection::establish(":memory:").expect("connection should open");
    let create_called = Arc::new(AtomicBool::new(false));
    let create_called_by_callback = Arc::clone(&create_called);

    let result = super::upsert_stored_object(
        &mut conn,
        ObjectType::Workflow,
        SyncId::ClientId(ClientId::new()),
        StoredObjectMetadata::mock(),
        StoredObjectPermissions::mock_personal(),
        Box::new(move |_| {
            create_called_by_callback.store(true, Ordering::SeqCst);
            Ok(1)
        }),
        Box::new(|_, _| Ok(())),
    );

    assert!(result.is_err(), "a missing metadata table must be reported");
    assert!(!create_called.load(Ordering::SeqCst));
}

#[test]
fn test_roundtrip_guests() {
    let guests = vec![
        StoredObjectGuest {
            subject: Subject::User(UserKind::Account(UserUid::new("local_user_uid"))),
            access_level: SharingAccessLevel::Edit,
            source: None,
        },
        StoredObjectGuest {
            subject: Subject::PendingUser {
                email: Some("pending@warp.dev".to_string()),
            },
            access_level: SharingAccessLevel::View,
            source: Some(ServerObjectContainer::Folder {
                folder_uid: 123.into(),
            }),
        },
        StoredObjectGuest {
            subject: Subject::Team(TeamKind::Team {
                team_uid: ServerId::from(99),
            }),
            access_level: SharingAccessLevel::Edit,
            source: None,
        },
    ];

    let encoded = super::encode_guests(&guests).expect("encode should succeed");
    let decoded = super::decode_guests(&encoded).expect("decode should succeed");

    assert_eq!(guests, decoded);
}

#[test]
fn test_fail_unsupported_subjects() {
    let result = super::encode_guests(&[StoredObjectGuest {
        subject: Subject::AnyoneWithLink(LinkSharingSubjectType::Anyone),
        access_level: SharingAccessLevel::View,
        source: None,
    }]);
    assert!(result.is_err());
}
