use super::*;

fn sample_server(name: &str) -> SshServerInfo {
    SshServerInfo {
        node_id: String::new(), // assigned by create_server
        host: format!("{name}.example.com"),
        port: 22,
        username: "root".into(),
        auth_type: AuthType::Password,
        key_path: None,
        credential_id: None,
        startup_command: None,
        notes: None,
        last_connected_at: None,
        session_resilience: SessionResilience::default(),
        ring_ceiling_mb: 0,
    }
}

#[test]
fn create_and_list_root_folder() {
    let mut conn = setup_in_memory();
    let f = SshRepository::create_folder(&mut conn, None, "Prod").unwrap();
    assert_eq!(f.kind, NodeKind::Folder);
    assert_eq!(f.name, "Prod");
    assert!(f.parent_id.is_none());

    let all = SshRepository::list_nodes(&mut conn).unwrap();
    assert_eq!(all.len(), 1);
}

#[test]
fn nested_folders_and_server() {
    let mut conn = setup_in_memory();
    let prod = SshRepository::create_folder(&mut conn, None, "Prod").unwrap();
    let web = SshRepository::create_folder(&mut conn, Some(&prod.id), "Web").unwrap();
    let srv =
        SshRepository::create_server(&mut conn, Some(&web.id), "edge1", &sample_server("edge1"))
            .unwrap();

    let all = SshRepository::list_nodes(&mut conn).unwrap();
    assert_eq!(all.len(), 3);
    let by_id: std::collections::HashMap<_, _> =
        all.into_iter().map(|n| (n.id.clone(), n)).collect();
    assert_eq!(by_id[&web.id].parent_id.as_deref(), Some(prod.id.as_str()));
    assert_eq!(by_id[&srv.id].parent_id.as_deref(), Some(web.id.as_str()));

    let server = SshRepository::get_server(&mut conn, &srv.id)
        .unwrap()
        .unwrap();
    assert_eq!(server.host, "edge1.example.com");
    assert_eq!(server.port, 22);
}

#[test]
fn sort_order_appends_within_parent() {
    let mut conn = setup_in_memory();
    let a = SshRepository::create_folder(&mut conn, None, "A").unwrap();
    let b = SshRepository::create_folder(&mut conn, None, "B").unwrap();
    let c = SshRepository::create_folder(&mut conn, None, "C").unwrap();
    assert_eq!(a.sort_order, 0);
    assert_eq!(b.sort_order, 1);
    assert_eq!(c.sort_order, 2);

    // Different parents each start from 0
    let child = SshRepository::create_folder(&mut conn, Some(&a.id), "child").unwrap();
    assert_eq!(child.sort_order, 0);
}

#[test]
fn rename_and_update_server() {
    let mut conn = setup_in_memory();
    let s = SshRepository::create_server(&mut conn, None, "old", &sample_server("foo")).unwrap();
    SshRepository::rename_node(&mut conn, &s.id, "new").unwrap();
    let mut info = SshRepository::get_server(&mut conn, &s.id)
        .unwrap()
        .unwrap();
    info.host = "bar.example.com".into();
    info.port = 2222;
    info.auth_type = AuthType::Key;
    info.key_path = Some("/k".into());
    SshRepository::update_server(&mut conn, &info).unwrap();

    let got = SshRepository::get_server(&mut conn, &s.id)
        .unwrap()
        .unwrap();
    assert_eq!(got.host, "bar.example.com");
    assert_eq!(got.port, 2222);
    assert_eq!(got.auth_type, AuthType::Key);
    assert_eq!(got.key_path.as_deref(), Some("/k"));
}

#[test]
fn create_list_and_update_onekey_credential() {
    let mut conn = setup_in_memory();
    let credential = SshRepository::create_onekey_credential(
        &mut conn,
        "prod-root",
        "root",
        OneKeyCredentialKind::Password,
        None,
    )
    .unwrap();
    assert_eq!(credential.label, "prod-root");
    assert_eq!(credential.username, "root");
    assert_eq!(credential.kind, OneKeyCredentialKind::Password);
    assert_eq!(credential.key_path, None);

    let listed = SshRepository::list_onekey_credentials(&mut conn).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, credential.id);

    let mut updated = credential.clone();
    updated.label = "prod-admin".into();
    updated.username = "admin".into();
    updated.kind = OneKeyCredentialKind::Key;
    updated.key_path = Some("/home/admin/.ssh/id_ed25519".into());
    SshRepository::update_onekey_credential(&mut conn, &updated).unwrap();

    let got = SshRepository::get_onekey_credential(&mut conn, &credential.id)
        .unwrap()
        .unwrap();
    assert_eq!(got.label, "prod-admin");
    assert_eq!(got.username, "admin");
    assert_eq!(got.kind, OneKeyCredentialKind::Key);
    assert_eq!(got.key_path.as_deref(), Some("/home/admin/.ssh/id_ed25519"));
}

#[test]
fn server_can_reference_onekey_credential() {
    let mut conn = setup_in_memory();
    let credential = SshRepository::create_onekey_credential(
        &mut conn,
        "shared",
        "deploy",
        OneKeyCredentialKind::Password,
        None,
    )
    .unwrap();
    let mut info = sample_server("edge");
    info.auth_type = AuthType::OneKey;
    info.username = "ignored-local-user".into();
    info.credential_id = Some(credential.id.clone());
    let node = SshRepository::create_server(&mut conn, None, "edge", &info).unwrap();

    let got = SshRepository::get_server(&mut conn, &node.id)
        .unwrap()
        .unwrap();
    assert_eq!(got.auth_type, AuthType::OneKey);
    assert_eq!(got.credential_id.as_deref(), Some(credential.id.as_str()));

    let resolved = SshRepository::resolve_server_auth(&mut conn, &got).unwrap();
    assert_eq!(resolved.username, "deploy");
    assert_eq!(resolved.auth_type, AuthType::Password);
    assert_eq!(resolved.key_path, None);
    assert_eq!(resolved.secret_lookup_id, credential.id);
    assert_eq!(resolved.secret_kind, SecretKind::OneKeyPassword);
}

#[test]
fn onekey_key_credential_resolves_to_key_auth() {
    let mut conn = setup_in_memory();
    let credential = SshRepository::create_onekey_credential(
        &mut conn,
        "shared-key",
        "deploy",
        OneKeyCredentialKind::Key,
        Some("/home/deploy/.ssh/id_ed25519"),
    )
    .unwrap();
    let mut info = sample_server("edge");
    info.auth_type = AuthType::OneKey;
    info.credential_id = Some(credential.id.clone());
    let node = SshRepository::create_server(&mut conn, None, "edge", &info).unwrap();
    let got = SshRepository::get_server(&mut conn, &node.id)
        .unwrap()
        .unwrap();

    let resolved = SshRepository::resolve_server_auth(&mut conn, &got).unwrap();

    assert_eq!(resolved.username, "deploy");
    assert_eq!(resolved.auth_type, AuthType::Key);
    assert_eq!(
        resolved.key_path.as_deref(),
        Some("/home/deploy/.ssh/id_ed25519")
    );
    assert_eq!(resolved.secret_lookup_id, credential.id);
    assert_eq!(resolved.secret_kind, SecretKind::Passphrase);
}

#[test]
fn delete_onekey_credential_is_blocked_while_hosts_reference_it() {
    let mut conn = setup_in_memory();
    let credential = SshRepository::create_onekey_credential(
        &mut conn,
        "shared",
        "deploy",
        OneKeyCredentialKind::Password,
        None,
    )
    .unwrap();
    let mut info = sample_server("edge");
    info.auth_type = AuthType::OneKey;
    info.credential_id = Some(credential.id.clone());
    SshRepository::create_server(&mut conn, None, "edge", &info).unwrap();

    assert!(
        SshRepository::delete_onekey_credential(&mut conn, &credential.id).is_err(),
        "referenced OneKey credential was deleted"
    );
}

#[test]
fn delete_cascades_to_children_and_server_row() {
    let mut conn = setup_in_memory();
    let parent = SshRepository::create_folder(&mut conn, None, "P").unwrap();
    let _child =
        SshRepository::create_server(&mut conn, Some(&parent.id), "c", &sample_server("c"))
            .unwrap();
    SshRepository::delete_node(&mut conn, &parent.id).unwrap();

    assert!(SshRepository::list_nodes(&mut conn).unwrap().is_empty());
}

#[test]
fn move_node_changes_parent_and_order() {
    let mut conn = setup_in_memory();
    let a = SshRepository::create_folder(&mut conn, None, "A").unwrap();
    let b = SshRepository::create_folder(&mut conn, None, "B").unwrap();
    let leaf =
        SshRepository::create_server(&mut conn, Some(&a.id), "x", &sample_server("x")).unwrap();

    SshRepository::move_node(&mut conn, &leaf.id, Some(&b.id), 5).unwrap();
    let nodes = SshRepository::list_nodes(&mut conn).unwrap();
    let leaf_now = nodes.iter().find(|n| n.id == leaf.id).unwrap();
    assert_eq!(leaf_now.parent_id.as_deref(), Some(b.id.as_str()));
    assert_eq!(leaf_now.sort_order, 5);
}

#[test]
fn delete_missing_returns_not_found() {
    let mut conn = setup_in_memory();
    let err = SshRepository::delete_node(&mut conn, "nope").unwrap_err();
    assert!(matches!(err, SshRepositoryError::NotFound(_)));
}

// ---- SyncMetaRepository tests ----

#[test]
fn sync_meta_get_version_default() {
    let mut conn = setup_in_memory();
    let version = SyncMetaRepository::get_sync_version(&mut conn).unwrap();
    assert_eq!(version, 0, "sync_version should be 0 when no data exists");
}

#[test]
fn sync_meta_set_and_get_version() {
    let mut conn = setup_in_memory();
    SyncMetaRepository::set_sync_version(&mut conn, 42).unwrap();
    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 42);
}

#[test]
fn sync_meta_increment_version() {
    let mut conn = setup_in_memory();
    let v1 = SyncMetaRepository::increment_sync_version(&mut conn).unwrap();
    assert_eq!(v1, 1);
    let v2 = SyncMetaRepository::increment_sync_version(&mut conn).unwrap();
    assert_eq!(v2, 2);
    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 2);
}

#[test]
fn sync_meta_increment_after_set() {
    let mut conn = setup_in_memory();
    SyncMetaRepository::set_sync_version(&mut conn, 99).unwrap();
    let v = SyncMetaRepository::increment_sync_version(&mut conn).unwrap();
    assert_eq!(v, 100);
}

#[test]
fn sync_meta_last_sync_time_default_empty() {
    let mut conn = setup_in_memory();
    let time = SyncMetaRepository::get_last_sync_time(&mut conn).unwrap();
    assert_eq!(time, "");
}

#[test]
fn sync_meta_last_sync_platform_default_empty() {
    let mut conn = setup_in_memory();
    let platform = SyncMetaRepository::get_last_sync_platform(&mut conn).unwrap();
    assert_eq!(platform, "");
}

#[test]
fn sync_meta_update_and_read() {
    let mut conn = setup_in_memory();
    SyncMetaRepository::update_sync_meta(&mut conn, "2026-05-26T10:00:00Z", "github").unwrap();
    assert_eq!(
        SyncMetaRepository::get_last_sync_time(&mut conn).unwrap(),
        "2026-05-26T10:00:00Z"
    );
    assert_eq!(
        SyncMetaRepository::get_last_sync_platform(&mut conn).unwrap(),
        "github"
    );
}

#[test]
fn sync_meta_update_overwrites_previous() {
    let mut conn = setup_in_memory();
    SyncMetaRepository::update_sync_meta(&mut conn, "t1", "gitee").unwrap();
    SyncMetaRepository::update_sync_meta(&mut conn, "t2", "github").unwrap();
    assert_eq!(
        SyncMetaRepository::get_last_sync_time(&mut conn).unwrap(),
        "t2"
    );
    assert_eq!(
        SyncMetaRepository::get_last_sync_platform(&mut conn).unwrap(),
        "github"
    );
}

#[test]
fn sync_meta_version_independent_of_meta() {
    let mut conn = setup_in_memory();
    SyncMetaRepository::set_sync_version(&mut conn, 10).unwrap();
    SyncMetaRepository::update_sync_meta(&mut conn, "t1", "gitee").unwrap();
    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 10);
}

// ---- Collapse operations should not increment sync_version ----

#[test]
fn set_collapsed_does_not_increment_sync_version() {
    let mut conn = setup_in_memory();
    let folder = SshRepository::create_folder(&mut conn, None, "F").unwrap();
    // create_folder increments once; reset to 0 for test
    SyncMetaRepository::set_sync_version(&mut conn, 0).unwrap();

    SshRepository::set_collapsed(&mut conn, &folder.id, true).unwrap();
    assert_eq!(
        SyncMetaRepository::get_sync_version(&mut conn).unwrap(),
        0,
        "set_collapsed should not increment sync_version"
    );

    let node = SshRepository::list_nodes(&mut conn)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert!(node.is_collapsed);
}

#[test]
fn set_collapsed_false_does_not_increment_sync_version() {
    let mut conn = setup_in_memory();
    let folder = SshRepository::create_folder(&mut conn, None, "F").unwrap();
    SshRepository::set_collapsed(&mut conn, &folder.id, true).unwrap();
    SyncMetaRepository::set_sync_version(&mut conn, 0).unwrap();

    SshRepository::set_collapsed(&mut conn, &folder.id, false).unwrap();
    assert_eq!(
        SyncMetaRepository::get_sync_version(&mut conn).unwrap(),
        0,
        "set_collapsed(false) should not increment sync_version"
    );
}

#[test]
fn set_all_folders_collapsed_does_not_increment_sync_version() {
    let mut conn = setup_in_memory();
    SshRepository::create_folder(&mut conn, None, "A").unwrap();
    SshRepository::create_folder(&mut conn, None, "B").unwrap();
    SyncMetaRepository::set_sync_version(&mut conn, 0).unwrap();

    SshRepository::set_all_folders_collapsed(&mut conn, true).unwrap();
    assert_eq!(
        SyncMetaRepository::get_sync_version(&mut conn).unwrap(),
        0,
        "set_all_folders_collapsed should not increment sync_version"
    );

    let nodes = SshRepository::list_nodes(&mut conn).unwrap();
    assert!(nodes.iter().all(|n| n.is_collapsed));
}

#[test]
fn set_collapsed_missing_node_returns_not_found() {
    let mut conn = setup_in_memory();
    let err = SshRepository::set_collapsed(&mut conn, "nonexistent", true).unwrap_err();
    assert!(matches!(err, SshRepositoryError::NotFound(_)));
}

#[test]
fn write_operations_do_increment_sync_version() {
    let mut conn = setup_in_memory();
    SyncMetaRepository::set_sync_version(&mut conn, 0).unwrap();

    let folder = SshRepository::create_folder(&mut conn, None, "F").unwrap();
    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 1);

    SshRepository::rename_node(&mut conn, &folder.id, "G").unwrap();
    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 2);

    SshRepository::delete_node(&mut conn, &folder.id).unwrap();
    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 3);
}

#[test]
fn repository_write_reports_sync_version_failure() {
    use diesel::connection::SimpleConnection as _;

    let mut conn = setup_in_memory();
    conn.batch_execute(
        "CREATE TRIGGER reject_sync_version \
         BEFORE INSERT ON sync_meta \
         WHEN NEW.key = 'sync_version' \
         BEGIN SELECT RAISE(FAIL, 'injected sync version failure'); END;",
    )
    .unwrap();

    assert!(SshRepository::create_folder(&mut conn, None, "must-roll-back").is_err());
    assert!(SshRepository::list_nodes(&mut conn).unwrap().is_empty());
}

// ---- move_node_to_end tests ----

#[test]
fn move_node_to_end_from_folder_a_to_folder_b() {
    let mut conn = setup_in_memory();
    let a = SshRepository::create_folder(&mut conn, None, "A").unwrap();
    let b = SshRepository::create_folder(&mut conn, None, "B").unwrap();
    let srv = SshRepository::create_server(&mut conn, Some(&a.id), "srv1", &sample_server("srv1"))
        .unwrap();

    SshRepository::move_node_to_end(&mut conn, &srv.id, Some(&b.id)).unwrap();

    let nodes = SshRepository::list_nodes(&mut conn).unwrap();
    let moved = nodes.iter().find(|n| n.id == srv.id).unwrap();
    assert_eq!(moved.parent_id.as_deref(), Some(b.id.as_str()));
    assert_eq!(
        moved.sort_order, 0,
        "B has no other children, sort_order should be 0"
    );
}

#[test]
fn move_node_to_end_to_root() {
    let mut conn = setup_in_memory();
    let folder = SshRepository::create_folder(&mut conn, None, "F").unwrap();
    let srv =
        SshRepository::create_server(&mut conn, Some(&folder.id), "srv1", &sample_server("srv1"))
            .unwrap();

    SshRepository::move_node_to_end(&mut conn, &srv.id, None).unwrap();

    let nodes = SshRepository::list_nodes(&mut conn).unwrap();
    let moved = nodes.iter().find(|n| n.id == srv.id).unwrap();
    assert!(
        moved.parent_id.is_none(),
        "parent_id should be None after moving to root"
    );
}

#[test]
fn move_node_to_end_appends_after_existing_children() {
    let mut conn = setup_in_memory();
    let folder = SshRepository::create_folder(&mut conn, None, "F").unwrap();
    let _s1 = SshRepository::create_server(&mut conn, Some(&folder.id), "s1", &sample_server("s1"))
        .unwrap();
    let _s2 = SshRepository::create_server(&mut conn, Some(&folder.id), "s2", &sample_server("s2"))
        .unwrap();

    let other = SshRepository::create_folder(&mut conn, None, "Other").unwrap();
    let srv =
        SshRepository::create_server(&mut conn, Some(&other.id), "mover", &sample_server("mover"))
            .unwrap();

    SshRepository::move_node_to_end(&mut conn, &srv.id, Some(&folder.id)).unwrap();

    let nodes = SshRepository::list_nodes(&mut conn).unwrap();
    let moved = nodes.iter().find(|n| n.id == srv.id).unwrap();
    assert_eq!(
        moved.sort_order, 2,
        "F already has 2 children, new node sort_order should be 2"
    );
    assert_eq!(moved.parent_id.as_deref(), Some(folder.id.as_str()));
}

#[test]
fn move_node_to_end_empty_target_folder() {
    let mut conn = setup_in_memory();
    let folder = SshRepository::create_folder(&mut conn, None, "Empty").unwrap();
    let srv =
        SshRepository::create_server(&mut conn, None, "srv1", &sample_server("srv1")).unwrap();

    SshRepository::move_node_to_end(&mut conn, &srv.id, Some(&folder.id)).unwrap();

    let nodes = SshRepository::list_nodes(&mut conn).unwrap();
    let moved = nodes.iter().find(|n| n.id == srv.id).unwrap();
    assert_eq!(
        moved.sort_order, 0,
        "sort_order should be 0 under empty folder"
    );
    assert_eq!(moved.parent_id.as_deref(), Some(folder.id.as_str()));
}

#[test]
fn move_node_to_end_missing_node_returns_not_found() {
    let mut conn = setup_in_memory();
    let err = SshRepository::move_node_to_end(&mut conn, "nonexistent", None).unwrap_err();
    assert!(
        matches!(err, SshRepositoryError::NotFound(_)),
        "nonexistent node should return NotFound error"
    );
}

#[test]
fn move_node_to_end_increments_sync_version() {
    let mut conn = setup_in_memory();
    let folder = SshRepository::create_folder(&mut conn, None, "F").unwrap();
    let srv =
        SshRepository::create_server(&mut conn, Some(&folder.id), "srv1", &sample_server("srv1"))
            .unwrap();
    SyncMetaRepository::set_sync_version(&mut conn, 0).unwrap();

    SshRepository::move_node_to_end(&mut conn, &srv.id, None).unwrap();

    assert_eq!(
        SyncMetaRepository::get_sync_version(&mut conn).unwrap(),
        1,
        "move_node_to_end should increment sync_version"
    );
}
