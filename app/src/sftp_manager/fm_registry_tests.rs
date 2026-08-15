use super::*;

fn desc(id: u64, fs: FsNamespace, path: &str) -> FmPaneDescriptor {
    FmPaneDescriptor {
        id,
        label: format!("pane{id}"),
        fs,
        current_path: PathBuf::from(path),
        pane_group_id: None,
    }
}

fn desc_in_group(id: u64, fs: FsNamespace, path: &str, group: EntityId) -> FmPaneDescriptor {
    FmPaneDescriptor {
        pane_group_id: Some(group),
        ..desc(id, fs, path)
    }
}

#[test]
fn upsert_inserts_then_updates_in_place() {
    let mut reg = FileManagerRegistry::new();
    reg.upsert(desc(1, FsNamespace::Local, "/a"));
    reg.upsert(desc(2, FsNamespace::Local, "/b"));
    assert_eq!(reg.panes().len(), 2);

    // Same id → replace, not append.
    reg.upsert(desc(1, FsNamespace::Local, "/a2"));
    assert_eq!(reg.panes().len(), 2);
    assert_eq!(
        reg.panes().iter().find(|p| p.id == 1).unwrap().current_path,
        PathBuf::from("/a2")
    );
}

#[test]
fn remove_is_idempotent() {
    let mut reg = FileManagerRegistry::new();
    reg.upsert(desc(1, FsNamespace::Local, "/a"));
    reg.remove(1);
    reg.remove(1); // no panic, no-op
    assert!(reg.panes().is_empty());
}

#[test]
fn others_same_fs_excludes_self_and_other_namespaces() {
    let mut reg = FileManagerRegistry::new();
    reg.upsert(desc(1, FsNamespace::Local, "/a"));
    reg.upsert(desc(2, FsNamespace::Local, "/b"));
    reg.upsert(desc(3, FsNamespace::Remote("host1".into()), "/c"));
    reg.upsert(desc(4, FsNamespace::Remote("host2".into()), "/d"));

    // From pane 1 (local): only pane 2 is a valid local target.
    let targets = reg.others_same_fs(1, &FsNamespace::Local);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].id, 2);

    // From pane 3 (host1): no other host1 pane exists.
    assert!(reg
        .others_same_fs(3, &FsNamespace::Remote("host1".into()))
        .is_empty());

    // `others` ignores the namespace: pane 1 sees 2, 3 and 4.
    let all = reg.others(1);
    assert_eq!(all.len(), 3);
    assert!(all.iter().all(|p| p.id != 1));
}

#[test]
fn backend_handle_is_stored_and_dropped_with_the_pane() {
    use super::super::sftp_backend::InMemorySftpBackend;
    let mut reg = FileManagerRegistry::new();
    reg.upsert(desc(1, FsNamespace::Local, "/a"));
    assert!(reg.backend_for(1).is_none());

    let backend: Arc<dyn SftpBackend> = Arc::new(InMemorySftpBackend::new(PathBuf::from("/")));
    reg.set_backend(1, backend);
    assert!(reg.backend_for(1).is_some());

    // Removing the pane drops its backend handle.
    reg.remove(1);
    assert!(reg.backend_for(1).is_none());
}

#[test]
fn backend_for_namespace_finds_a_live_backend_for_the_host() {
    use super::super::sftp_backend::InMemorySftpBackend;
    let mut reg = FileManagerRegistry::new();
    let host = FsNamespace::Remote("h1".into());
    // Two panes on the same host; only the second has a backend registered.
    reg.upsert(desc(1, host.clone(), "/a"));
    reg.upsert(desc(2, host.clone(), "/b"));
    reg.upsert(desc(3, FsNamespace::Local, "/c"));
    assert!(reg.backend_for_namespace(&host).is_none());

    let backend: Arc<dyn SftpBackend> = Arc::new(InMemorySftpBackend::new(PathBuf::from("/")));
    reg.set_backend(2, backend);
    // A pane on the host now has a backend → resolves.
    assert!(reg.backend_for_namespace(&host).is_some());
    // A different host / the local namespace does not.
    assert!(reg
        .backend_for_namespace(&FsNamespace::Remote("other".into()))
        .is_none());
    assert!(reg.backend_for_namespace(&FsNamespace::Local).is_none());

    // Dropping the only backed pane makes it unresolvable again.
    reg.remove(2);
    assert!(reg.backend_for_namespace(&host).is_none());
}

#[test]
fn plan_transfer_covers_every_direction() {
    let local = FsNamespace::Local;
    let host_a = FsNamespace::Remote("a".into());
    let host_a2 = FsNamespace::Remote("a".into());
    let host_b = FsNamespace::Remote("b".into());

    assert_eq!(plan_transfer(&local, &local), TransferKind::DirectSameFs);
    assert_eq!(plan_transfer(&host_a, &host_a2), TransferKind::DirectSameFs);
    assert_eq!(plan_transfer(&local, &host_a), TransferKind::Upload);
    assert_eq!(plan_transfer(&host_a, &local), TransferKind::Download);
    assert_eq!(
        plan_transfer(&host_a, &host_b),
        TransferKind::RemoteToRemote
    );
}

#[test]
fn one_other_visible_file_manager_is_default_target() {
    let mut reg = FileManagerRegistry::new();
    let group = EntityId::from_usize(1);
    reg.upsert(desc_in_group(10, FsNamespace::Local, "/source", group));
    reg.upsert(desc_in_group(
        20,
        FsNamespace::Remote("host".into()),
        "/target",
        group,
    ));

    let targets = reg.transfer_targets(10);
    assert_eq!(targets.selectable.len(), 1);
    assert_eq!(targets.default.map(|pane| pane.id), Some(20));
}

#[test]
fn hidden_tab_file_managers_are_selectable_targets() {
    let mut reg = FileManagerRegistry::new();
    reg.upsert(desc_in_group(
        10,
        FsNamespace::Local,
        "/source",
        EntityId::from_usize(1),
    ));
    // Simulate panes registered from inactive tabs in an arbitrary
    // activation order. Target discovery must not inherit that order.
    reg.upsert(desc_in_group(
        30,
        FsNamespace::Remote("hidden-b".into()),
        "/b",
        EntityId::from_usize(3),
    ));
    reg.upsert(desc_in_group(
        20,
        FsNamespace::Remote("hidden-a".into()),
        "/a",
        EntityId::from_usize(2),
    ));

    let targets = reg.transfer_targets(10);
    assert!(targets.default.is_none());
    let target_ids = targets
        .selectable
        .into_iter()
        .map(|pane| pane.id)
        .collect::<Vec<_>>();
    assert_eq!(target_ids, vec![20, 30]);
}

#[test]
fn one_visible_target_wins_over_hidden_tab_candidates() {
    let mut reg = FileManagerRegistry::new();
    let visible_group = EntityId::from_usize(1);
    reg.upsert(desc_in_group(
        10,
        FsNamespace::Local,
        "/source",
        visible_group,
    ));
    reg.upsert(desc_in_group(
        20,
        FsNamespace::Local,
        "/visible",
        visible_group,
    ));
    reg.upsert(desc_in_group(
        30,
        FsNamespace::Remote("hidden".into()),
        "/hidden",
        EntityId::from_usize(2),
    ));

    let targets = reg.transfer_targets(10);
    assert_eq!(targets.default.map(|pane| pane.id), Some(20));
    assert_eq!(targets.selectable.len(), 2);
}

#[test]
fn multiple_visible_targets_require_an_explicit_choice() {
    let mut reg = FileManagerRegistry::new();
    let group = EntityId::from_usize(1);
    for id in [10, 20, 30] {
        reg.upsert(desc_in_group(id, FsNamespace::Local, "/", group));
    }

    let targets = reg.transfer_targets(10);
    assert!(targets.default.is_none());
    assert_eq!(targets.selectable.len(), 2);
}

#[test]
fn unattached_source_never_implicitly_targets_another_pane() {
    let mut reg = FileManagerRegistry::new();
    reg.upsert(desc(10, FsNamespace::Local, "/source"));
    reg.upsert(desc(20, FsNamespace::Local, "/other"));

    let targets = reg.transfer_targets(10);
    assert!(targets.default.is_none());
    assert_eq!(targets.selectable.len(), 1);
}
